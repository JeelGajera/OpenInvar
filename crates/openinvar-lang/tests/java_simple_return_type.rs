//! A method's return type is a reference whatever shape it is written in.
//!
//! `List<Thing> get()` recorded a reference to `Thing`. `Thing get()` recorded
//! nothing at all. The two went through the same code and the difference was
//! entirely in how deep the name sat: the walk that collected type names
//! skipped the node it started from, and for a plain return type that node
//! *is* the name.
//!
//! # Why a missing edge is the worse direction
//!
//! The two Java fixes before this one both removed edges — a type parameter
//! that had bound to an unrelated class, an index entry no import could reach.
//! This one adds them, and the case for it is what the graph is asked for.
//! `blast-radius Thing` answers "what breaks if I change this", and a factory
//! method returning `Thing` is the most ordinary way for a caller to depend on
//! it. Reported as safe to modify, that is a wrong answer in the direction a
//! gate acts on. An over-reported dependency wastes a reviewer's time; an
//! under-reported one is the reason the reviewer was not called.
//!
//! On `google/gson`, roughly 605 methods return a plain type.

#![cfg(feature = "java")]

use openinvar_core::ir::FileIR;

fn analyze(name: &str, files: &[(&str, &str)]) -> Vec<FileIR> {
    let root = std::env::temp_dir().join(format!(
        "openinvar-java-return-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let mut paths = Vec::new();
    for (path, source) in files {
        let full = root.join(path);
        std::fs::create_dir_all(full.parent().expect("parent")).expect("mkdir");
        std::fs::write(&full, source).expect("write");
        paths.push(full);
    }
    paths.sort();
    let ir = openinvar_lang::dispatch::analyze_files(&root, &paths).expect("analysis");
    let _ = std::fs::remove_dir_all(&root);
    ir.files
}

/// Where the references in `file` point, by target symbol id.
fn targets<'a>(files: &'a [FileIR], file: &str) -> Vec<&'a str> {
    files
        .iter()
        .find(|f| f.file.ends_with(file))
        .unwrap_or_else(|| panic!("{file} was analysed"))
        .relationships
        .iter()
        .map(|r| r.to.as_str())
        .collect()
}

fn unbound(files: &[FileIR]) -> u32 {
    files.iter().map(|f| f.unbound_references).sum()
}

const THING: (&str, &str) = (
    "src/com/example/Thing.java",
    "package com.example;\n\
     public class Thing {\n\
     \x20   public int value() { return 1; }\n\
     }\n",
);

#[test]
fn a_plain_return_type_is_recorded() {
    // The fix. Nothing about this method is unusual — it is the shape most
    // factory and accessor methods have.
    let files = analyze(
        "plain",
        &[
            THING,
            (
                "src/com/example/Factory.java",
                "package com.example;\n\
                 public class Factory {\n\
                 \x20   public Thing make() { return null; }\n\
                 }\n",
            ),
        ],
    );

    assert!(
        targets(&files, "Factory.java")
            .iter()
            .any(|t| t.contains("Thing.java")),
        "a method returning `Thing` recorded no reference to it: {:?}",
        targets(&files, "Factory.java")
    );
}

#[test]
fn a_plain_return_type_is_recorded_once() {
    // The shape of the fix is "the starting node counts too", and the way to
    // get that wrong is to count it twice — once as itself and once as its own
    // descendant. A duplicated edge would not fail the test above, and on a
    // real repository it inflates every count the graph reports.
    let files = analyze(
        "once",
        &[
            THING,
            (
                "src/com/example/Factory.java",
                "package com.example;\n\
                 public class Factory {\n\
                 \x20   public Thing make() { return null; }\n\
                 }\n",
            ),
        ],
    );

    let to_thing = targets(&files, "Factory.java")
        .iter()
        .filter(|t| t.contains("Thing.java"))
        .count();
    assert_eq!(to_thing, 1, "the return type was recorded {to_thing} times");
}

#[test]
fn a_wrapped_return_type_still_records_everything_it_did() {
    // The half that already worked, and the half a careless change breaks.
    // `List<Thing>` names two types and both are references; an array names
    // one. All three roots are wrapper nodes rather than the name itself, so
    // this asserts that making the root count did not disturb them.
    let files = analyze(
        "wrapped",
        &[
            THING,
            (
                "src/com/example/Holder.java",
                "package com.example;\n\
                 public class Holder {\n\
                 \x20   public java.util.List<Thing> all() { return null; }\n\
                 \x20   public Thing[] array() { return null; }\n\
                 }\n",
            ),
        ],
    );

    let to_thing = targets(&files, "Holder.java")
        .iter()
        .filter(|t| t.contains("Thing.java"))
        .count();
    assert_eq!(
        to_thing, 2,
        "a generic and an array return type between them should reference \
         `Thing` twice, got {to_thing}: {:?}",
        targets(&files, "Holder.java")
    );
}

#[test]
fn a_return_type_that_is_a_type_parameter_is_still_filtered() {
    // The interaction that matters most, because it runs the other way. The
    // generic-parameter filter sits in the same function, and a root node that
    // now counts is a root node that now has to be filtered. `T get()` inside
    // `Box<T>` must stay silent — otherwise this change reintroduces exactly
    // the spurious cross-file binding the previous one removed.
    let files = analyze(
        "type-parameter",
        &[
            (
                "src/com/example/T.java",
                "package com.example;\n\
                 public class T {\n\
                 \x20   public int unrelated() { return 1; }\n\
                 }\n",
            ),
            (
                "src/com/example/Box.java",
                "package com.example;\n\
                 public class Box<T> {\n\
                 \x20   public T get() { return null; }\n\
                 \x20   public <U> U convert() { return null; }\n\
                 }\n",
            ),
        ],
    );

    let leaked: Vec<&str> = targets(&files, "Box.java")
        .into_iter()
        .filter(|t| t.contains("T.java"))
        .collect();
    assert!(
        leaked.is_empty(),
        "a return type that is a type parameter bound to an unrelated class of \
         the same name: {leaked:?}"
    );
    assert_eq!(
        unbound(&files),
        0,
        "a return type that is a type parameter was counted as unbound"
    );
}

#[test]
fn void_and_primitive_return_types_record_nothing() {
    // `void` and `int` are `void_type` and `integral_type`, not
    // `type_identifier`, so the root-counting change must not reach them. A
    // repository cannot declare `int`, and an edge to one would be unresolvable
    // by construction — a permanent unbound reference on almost every method.
    let files = analyze(
        "primitive",
        &[(
            "src/com/example/Counter.java",
            "package com.example;\n\
             public class Counter {\n\
             \x20   public void reset() {}\n\
             \x20   public int count() { return 0; }\n\
             \x20   public boolean empty() { return true; }\n\
             }\n",
        )],
    );

    assert_eq!(
        unbound(&files),
        0,
        "a primitive or `void` return type was recorded as a type reference"
    );
    assert!(
        targets(&files, "Counter.java").is_empty(),
        "a class of primitive-returning methods recorded references: {:?}",
        targets(&files, "Counter.java")
    );
}

#[test]
fn a_plain_supertype_was_never_affected_and_still_is_not() {
    // Worth pinning because the asymmetry is surprising: `extends Base` names a
    // plain type too, and it always resolved, because the grammar wraps it in a
    // `superclass` node while a return type sits bare under the method. The
    // change is at the root of the expression, so if it had shifted the
    // wrapper cases this is where a duplicate would appear.
    let files = analyze(
        "supertype",
        &[
            (
                "src/com/example/Base.java",
                "package com.example;\n\
                 public class Base {\n\
                 \x20   public int base() { return 1; }\n\
                 }\n",
            ),
            (
                "src/com/example/Derived.java",
                "package com.example;\n\
                 public class Derived extends Base {\n\
                 \x20   public int derived() { return 2; }\n\
                 }\n",
            ),
        ],
    );

    let to_base = targets(&files, "Derived.java")
        .iter()
        .filter(|t| t.contains("Base.java"))
        .count();
    assert_eq!(
        to_base, 1,
        "`extends Base` should record exactly one reference, got {to_base}: {:?}",
        targets(&files, "Derived.java")
    );
}

#[test]
fn a_qualifier_in_a_return_type_is_still_read_as_a_type() {
    // Not fixed here, and asserted so that it is written down somewhere rather
    // than merely true. `com.example.Thing get()` parses as nested
    // `scoped_type_identifier`s whose every segment is a `type_identifier`, so
    // the walk collects `com` and `example` alongside `Thing` and offers both
    // to the resolver as though a repository might contain a type called `com`.
    //
    // It costs unbound references rather than wrong edges, since a package
    // segment is lowercase by convention and will not match a type — but the
    // convention is not a rule, and this is the same class of name-shaped
    // guessing the generic-parameter fix removed. Left alone because narrowing
    // it is a change to how qualified names are read, which is not what this
    // commit is about; a reference to the fully-qualified type is recorded
    // either way, so the useful edge is present.
    //
    // If this starts failing, qualified type names have been narrowed to their
    // last segment. That is the improvement; delete this test.
    let files = analyze(
        "qualifier",
        &[
            THING,
            (
                "src/com/example/Factory.java",
                "package com.example;\n\
                 public class Factory {\n\
                 \x20   public com.example.Thing make() { return null; }\n\
                 }\n",
            ),
        ],
    );

    assert!(
        targets(&files, "Factory.java")
            .iter()
            .any(|t| t.contains("Thing.java")),
        "the fully-qualified return type recorded no reference to the type \
         itself: {:?}",
        targets(&files, "Factory.java")
    );
    assert_eq!(
        unbound(&files),
        2,
        "expected the two package segments of `com.example.Thing` to be the \
         only unbound references, got {}: {:?}",
        unbound(&files),
        targets(&files, "Factory.java")
    );
}
