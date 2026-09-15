//! A Java generic type parameter is not a type reference.
//!
//! `class Box<T> { T item; }` names `T` twice, and neither is a type this
//! repository could contain — `T` is a placeholder the caller fills in. The
//! extractor recorded it as an ordinary type reference all the same.
//!
//! # Why this is a correctness fix and not a tidy-up
//!
//! An unresolved `T` costs a spurious unbound reference, which is the visible
//! half. The invisible half is worse: when some *other* file happens to declare
//! a class called `T`, the resolver finds it and binds. The edge is then
//! `Resolution::Resolved` — gate-safe — and says a generic parameter depends on
//! an unrelated class in a different file.
//!
//! That is not hypothetical. On `google/gson`, `MultiParameters<A, B, C, D, E>`
//! has fields `A a; B b; C c;`, and a test file elsewhere declares classes
//! named `A`, `B` and `C`. Seven edges bound across that coincidence, and
//! `blast-radius` on those classes named `MultiParameters` as a dependent.
//! Guessing by name across a repository is the bug this project exists to
//! avoid, and the Tier 1 Java resolver was doing it.

#![cfg(feature = "java")]

use openinvar_core::ir::FileIR;

fn analyze(name: &str, files: &[(&str, &str)]) -> Vec<FileIR> {
    let root = std::env::temp_dir().join(format!(
        "openinvar-java-generics-{name}-{}-{}",
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

fn unbound(files: &[FileIR]) -> u32 {
    files.iter().map(|f| f.unbound_references).sum()
}

#[test]
fn a_type_parameter_never_binds_to_a_same_named_class_elsewhere() {
    // The gson bug, reduced. `Box<T>` and an unrelated `T` in another file:
    // before this, `Box.item` bound to that `T` and the edge was reported as
    // resolved, which is a gate-safe claim that one file depends on another for
    // no reason but a shared letter.
    let files = analyze(
        "cross-file",
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
                 \x20   private T item;\n\
                 \x20   public T get() { return item; }\n\
                 }\n",
            ),
        ],
    );

    let box_file = files
        .iter()
        .find(|f| f.file.ends_with("Box.java"))
        .expect("Box.java was analysed");

    let leaked: Vec<&str> = box_file
        .relationships
        .iter()
        .filter(|r| r.to.contains("T.java"))
        .map(|r| r.to.as_str())
        .collect();

    assert!(
        leaked.is_empty(),
        "a type parameter bound to an unrelated class of the same name, and the \
         edge is gate-safe: {leaked:?}"
    );
}

#[test]
fn a_type_parameter_is_not_counted_as_an_unbound_reference() {
    // The visible half. With no same-named class anywhere, `T` simply fails to
    // resolve and inflates the figure R′ is built on.
    let files = analyze(
        "unbound",
        &[(
            "src/com/example/Box.java",
            "package com.example;\n\
             public class Box<T> {\n\
             \x20   private T item;\n\
             \x20   public T get() { return item; }\n\
             \x20   public void put(T value) { this.item = value; }\n\
             }\n",
        )],
    );

    assert_eq!(
        unbound(&files),
        0,
        "a class whose only unresolved names are its own type parameters \
         reported {} unbound reference(s)",
        unbound(&files)
    );
}

#[test]
fn a_methods_own_type_parameter_is_in_scope_too() {
    // Java declares them in two places, and a method's are in scope for its
    // body as much as its class's are. Reading only the class declaration would
    // leave every `<T> T unwrap()` still reporting `T`.
    let files = analyze(
        "method-generic",
        &[(
            "src/com/example/Util.java",
            "package com.example;\n\
             public class Util {\n\
             \x20   public <T> T identity(T value) { return value; }\n\
             \x20   public <K, V> K firstKey(V ignored) { return null; }\n\
             }\n",
        )],
    );

    assert_eq!(
        unbound(&files),
        0,
        "a method's own type parameters were reported as unresolved types"
    );
}

#[test]
fn a_bound_names_a_real_type_that_nothing_records_yet() {
    // Not a consequence of this change, and checked against the released 0.3.0
    // binary before being written down: `class Holder<T extends Shape>` records
    // no edge to `Shape`, and never did. The extractor reads `superclass` and
    // the interface lists, and a `type_parameters` field is neither.
    //
    // Asserted rather than left unsaid because it is a real gap — a class that
    // constrains a parameter to a repository type does depend on it — and
    // because the filter added here deliberately keeps its hands off the bound.
    // If bounds ever start being recorded, this fails and sends the reader to
    // check that the filter did not swallow them on the way in.
    let files = analyze(
        "bound",
        &[
            (
                "src/com/example/Shape.java",
                "package com.example;\n\
                 public class Shape {\n\
                 \x20   public int sides() { return 0; }\n\
                 }\n",
            ),
            (
                "src/com/example/Holder.java",
                "package com.example;\n\
                 public class Holder<T extends Shape> {\n\
                 \x20   private T held;\n\
                 }\n",
            ),
        ],
    );

    let holder = files
        .iter()
        .find(|f| f.file.ends_with("Holder.java"))
        .expect("Holder.java was analysed");

    assert!(
        !holder
            .relationships
            .iter()
            .any(|r| r.to.contains("Shape.java")),
        "a type-parameter bound now records a reference, which is an \
         improvement — check that `generic_parameter_names` still takes only \
         the direct `type_identifier` child and is not eating the bound, then \
         invert this assertion"
    );
    assert_eq!(
        unbound(&files),
        0,
        "the parameter `T` itself is still counted"
    );
}

#[test]
fn an_ordinary_type_of_the_same_shape_still_resolves() {
    // A single uppercase letter is a convention, not a rule, and a class may be
    // named anything. `Box` here has no type parameters at all, so nothing is
    // in scope to filter and a field typed `T` is a genuine reference to the
    // class `T` next door.
    let files = analyze(
        "no-generics",
        &[
            (
                "src/com/example/T.java",
                "package com.example;\n\
                 public class T {\n\
                 \x20   public int value() { return 1; }\n\
                 }\n",
            ),
            (
                "src/com/example/Plain.java",
                "package com.example;\n\
                 public class Plain {\n\
                 \x20   private T item;\n\
                 }\n",
            ),
        ],
    );

    let plain = files
        .iter()
        .find(|f| f.file.ends_with("Plain.java"))
        .expect("Plain.java was analysed");

    assert!(
        plain.relationships.iter().any(|r| r.to.contains("T.java")),
        "a real class named `T` stopped resolving because the filter is keyed \
         on the name rather than on what is declared: {:?}",
        plain.relationships.iter().map(|r| &r.to).collect::<Vec<_>>()
    );
}

#[test]
fn a_generic_supertype_records_the_supertype_and_not_the_parameter() {
    // `class Box<T> extends Holder<T>` names both, and exactly one of them is a
    // type this repository contains.
    let files = analyze(
        "generic-extends",
        &[
            (
                "src/com/example/Holder.java",
                "package com.example;\n\
                 public class Holder<T> {\n\
                 \x20   protected T held;\n\
                 }\n",
            ),
            (
                "src/com/example/Box.java",
                "package com.example;\n\
                 public class Box<T> extends Holder<T> {\n\
                 \x20   public T peek() { return held; }\n\
                 }\n",
            ),
        ],
    );

    let box_file = files
        .iter()
        .find(|f| f.file.ends_with("Box.java"))
        .expect("Box.java was analysed");

    assert!(
        box_file
            .relationships
            .iter()
            .any(|r| r.to.contains("Holder.java")),
        "the supertype was dropped along with the parameter"
    );
    assert_eq!(
        unbound(&files),
        0,
        "the parameter in `extends Holder<T>` was still counted"
    );
}
