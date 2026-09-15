//! Why a Java reference did not bind.
//!
//! Java attempted more references than any other adapter in the evaluation
//! corpus and classified none of them, so R′ read as a pure ceiling for the
//! repository contributing 83% of its unbound total.
//!
//! The classification is evidence-only, and the evidence is the file's own
//! import statements rather than a list of names. That choice is what these
//! pin, because the tempting shortcut — "the index has no entry for this
//! fully-qualified name, so it is somebody else's code" — is wrong in a way
//! that flatters the number. A nested class is imported by an FQN the index
//! does not key, and on gson that shortcut would have reported 235 of the
//! project's own types as external.

#![cfg(feature = "java")]

use openinvar_core::ir::FileIR;

fn analyze(name: &str, files: &[(&str, &str)]) -> Vec<FileIR> {
    let root = std::env::temp_dir().join(format!(
        "openinvar-java-unbound-{name}-{}-{}",
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

/// Total unbound, and the share the adapter placed outside the repository.
fn split(files: &[FileIR]) -> (u32, u32) {
    (
        files.iter().map(|f| f.unbound_references).sum(),
        files.iter().map(|f| f.unbound_outside_repository).sum(),
    )
}

#[test]
fn a_type_imported_from_a_package_this_tree_does_not_declare_is_outside() {
    // The ordinary case, and the bulk of it: `java.io` is not a package any
    // file here declares, so the import statement settles the question.
    let files = analyze(
        "imported-external",
        &[(
            "src/com/example/Reader.java",
            "package com.example;\n\
             import java.io.IOException;\n\
             public class Reader {\n\
             \x20   private IOException last;\n\
             \x20   public IOException read() { return new IOException(); }\n\
             }\n",
        )],
    );

    let (total, outside) = split(&files);
    assert!(total > 0, "nothing was counted as unbound at all");
    assert_eq!(
        outside, total,
        "a type imported from java.io was left unexplained"
    );
}

#[test]
fn a_nested_class_this_tree_declares_is_not_claimed_as_outside() {
    // The case that rules out classifying on a missing index entry, and the
    // reason this file exists.
    //
    // `TestTypes.Bag` is imported by its nested FQN. Nothing is keyed under
    // that exact string, so an implementation asking "is this name in the
    // index?" would answer no and call it external — while the class is right
    // there in the next file. Asking instead whether any prefix of the name is
    // a package this tree declares gets it right: `com.example.common` is
    // declared here, so this is a reference the resolver missed, and it belongs
    // in the unexplained half.
    let files = analyze(
        "nested-class",
        &[
            (
                "src/com/example/common/TestTypes.java",
                "package com.example.common;\n\
                 public class TestTypes {\n\
                 \x20   public static class Bag {\n\
                 \x20       public int value;\n\
                 \x20   }\n\
                 }\n",
            ),
            (
                "src/com/example/Use.java",
                "package com.example;\n\
                 import com.example.common.TestTypes.Bag;\n\
                 public class Use {\n\
                 \x20   public Bag make() { return new Bag(); }\n\
                 }\n",
            ),
        ],
    );

    let (total, outside) = split(&files);
    assert!(
        total > 0,
        "the nested class bound after all, so this no longer tests anything"
    );
    assert_eq!(
        outside, 0,
        "a class declared in this very tree was attributed outside it, which is \
         the one error that makes the figure look better than the work"
    );
}

#[test]
fn a_java_lang_type_needs_no_import_to_be_outside() {
    // `java.lang` is in scope without an import, so there is no statement to
    // read and a list is the only evidence available. Confined to `java.lang`
    // precisely because its contents are fixed by the language rather than by
    // anyone's dependencies.
    let files = analyze(
        "java-lang",
        &[(
            "src/com/example/Greet.java",
            "package com.example;\n\
             public class Greet {\n\
             \x20   public String name() { return new StringBuilder().toString(); }\n\
             \x20   public void fail() { throw new IllegalStateException(); }\n\
             }\n",
        )],
    );

    let (total, outside) = split(&files);
    assert!(total > 0, "nothing was counted as unbound at all");
    assert_eq!(
        outside, total,
        "a java.lang type was left unexplained: {total} unbound, {outside} placed"
    );
}

#[test]
fn a_statically_imported_member_is_outside_when_its_declarer_is() {
    // What places gson's 1,100-odd `assertThat` calls. The receiver is the test
    // class itself, which is very much in the repository, so classifying by the
    // receiver would say nothing. The static import names the declaring type,
    // and that is what settles it.
    let files = analyze(
        "static-import",
        &[(
            "src/com/example/ThingTest.java",
            "package com.example;\n\
             import static com.google.common.truth.Truth.assertThat;\n\
             public class ThingTest {\n\
             \x20   public void check() { assertThat(1); }\n\
             }\n",
        )],
    );

    let (total, outside) = split(&files);
    assert!(total > 0, "nothing was counted as unbound at all");
    assert_eq!(
        outside, total,
        "a member statically imported from outside the tree was left unexplained"
    );
}

#[test]
fn a_member_of_a_type_this_tree_declares_stays_unexplained() {
    // The counterweight to the test above. `Helper.missing` names a method
    // that does not exist on a class that does, so nothing here is evidence of
    // anything external — and a classifier reaching for the receiver's package
    // would wrongly place it, because the package is this one.
    let files = analyze(
        "internal-member",
        &[
            (
                "src/com/example/Helper.java",
                "package com.example;\n\
                 public class Helper {\n\
                 \x20   public int present() { return 1; }\n\
                 }\n",
            ),
            (
                "src/com/example/Caller.java",
                "package com.example;\n\
                 public class Caller {\n\
                 \x20   public void go() { Helper h = new Helper(); h.missing(); }\n\
                 }\n",
            ),
        ],
    );

    let (total, outside) = split(&files);
    assert!(total > 0, "the missing method bound after all");
    assert_eq!(
        outside, 0,
        "a member of a class declared in this tree was attributed outside it"
    );
}

#[test]
fn a_name_reachable_only_through_a_wildcard_is_not_classified() {
    // Deliberately unexplained. With two on-demand imports in scope there is no
    // way to say which one a bare name came from, and the whole point of the
    // split is that one side of it is evidence rather than a guess. `Thing`
    // could be from either package; neither is checked, so it stays where it
    // cannot mislead.
    let files = analyze(
        "wildcard",
        &[(
            "src/com/example/Use.java",
            "package com.example;\n\
             import java.util.*;\n\
             import com.vendor.widgets.*;\n\
             public class Use {\n\
             \x20   public Thing make() { return new Thing(); }\n\
             }\n",
        )],
    );

    let (total, outside) = split(&files);
    assert!(total > 0, "nothing was counted as unbound at all");
    assert_eq!(
        outside, 0,
        "a name reachable only through a wildcard import was classified on a guess"
    );
}

#[test]
fn the_classified_share_never_exceeds_the_total() {
    // The two counts are separate fields incremented at separate sites, so
    // nothing in the type system stops them disagreeing. A file mixing every
    // case above is where drift would show.
    let files = analyze(
        "mixed",
        &[(
            "src/com/example/Mixed.java",
            "package com.example;\n\
             import java.io.IOException;\n\
             import java.util.*;\n\
             public class Mixed {\n\
             \x20   public String a() { return new StringBuilder().toString(); }\n\
             \x20   public void b() throws IOException {}\n\
             \x20   public Thing c() { return new Thing(); }\n\
             \x20   public void d() { nowhere(); }\n\
             }\n",
        )],
    );

    for file in &files {
        assert!(
            file.unbound_outside_repository <= file.unbound_references,
            "{}: classified {} of {} unbound references",
            file.file,
            file.unbound_outside_repository,
            file.unbound_references
        );
    }
    let (total, outside) = split(&files);
    assert!(
        outside > 0 && outside < total,
        "expected a mix of placed and unexplained, got {outside} of {total}"
    );
}
