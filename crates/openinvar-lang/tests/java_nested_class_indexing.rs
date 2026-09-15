//! A Java nested class is indexed under the name an import can spell.
//!
//! Java's canonical name for a nested class is `Outer.Inner`, and that is what
//! an import statement names. The index was built from each type's *simple*
//! name, so `TestTypes.Bag` went in as `com.example.common.Bag` — a name no
//! import can write and none of its references could match.
//!
//! Two things followed. Every reference through such an import missed: 235 of
//! them on `google/gson`, and 279 edges once resolution is counted. And the
//! entry that was written is one no valid Java import could ever reach, so it
//! sat in the index doing nothing while shadowing nothing.
//!
//! The nesting is recorded nowhere in the IR — a nested class's symbol id
//! carries its simple name alone — so it is recovered from the line ranges the
//! extractor already emits. A type declared inside another is spanned by it,
//! and two siblings never span each other.

#![cfg(feature = "java")]

use openinvar_core::ir::FileIR;

fn analyze(name: &str, files: &[(&str, &str)]) -> Vec<FileIR> {
    let root = std::env::temp_dir().join(format!(
        "openinvar-java-nested-{name}-{}-{}",
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

const TEST_TYPES: &str = "package com.example.common;\n\
                          public class TestTypes {\n\
                          \x20   public static class Bag {\n\
                          \x20       public int value;\n\
                          \x20   }\n\
                          }\n";

#[test]
fn a_nested_class_resolves_through_its_canonical_import() {
    // The gson case. `import com.example.common.TestTypes.Bag` is how Java
    // names a nested class, and until the index carried that name the import
    // matched nothing.
    let files = analyze(
        "canonical",
        &[
            ("src/com/example/common/TestTypes.java", TEST_TYPES),
            (
                "src/com/example/Use.java",
                "package com.example;\n\
                 import com.example.common.TestTypes.Bag;\n\
                 public class Use {\n\
                 \x20   private Bag held;\n\
                 }\n",
            ),
        ],
    );

    assert!(
        targets(&files, "Use.java")
            .iter()
            .any(|t| t.contains("TestTypes.java") && t.contains("Bag")),
        "the nested class was not reached through its own import: {:?}",
        targets(&files, "Use.java")
    );
    assert_eq!(
        files.iter().map(|f| f.unbound_references).sum::<u32>(),
        0,
        "the reference through the nested import is still unbound"
    );
}

#[test]
fn a_nested_class_is_not_reachable_by_a_name_java_cannot_write() {
    // The other half, and the reason this is a fix rather than an addition.
    // `com.example.common.Bag` was the name the index used to hold, and it is
    // not a name any Java import can name — `Bag` is not a top-level type of
    // that package. An index that answers to it is answering a question no
    // compiler would ask, and would resolve an import that does not compile.
    let files = analyze(
        "wrong-name",
        &[
            ("src/com/example/common/TestTypes.java", TEST_TYPES),
            (
                "src/com/example/Use.java",
                "package com.example;\n\
                 import com.example.common.Bag;\n\
                 public class Use {\n\
                 \x20   private Bag held;\n\
                 }\n",
            ),
        ],
    );

    assert!(
        !targets(&files, "Use.java")
            .iter()
            .any(|t| t.contains("TestTypes.java")),
        "an import Java would reject resolved anyway: {:?}",
        targets(&files, "Use.java")
    );
}

#[test]
fn two_classes_of_one_name_stay_apart() {
    // The risk in indexing more names is binding the wrong one. A nested
    // `common.TestTypes.Bag` and a top-level `other.Bag` share a simple name
    // and nothing else; each import must reach its own.
    //
    // On gson this is `BagOfPrimitives`, which exists both nested in
    // `TestTypes` and as a top-level class under `metrics`. 110 references go
    // to the first and 10 to the second, and they must not be pooled.
    let files = analyze(
        "same-simple-name",
        &[
            ("src/com/example/common/TestTypes.java", TEST_TYPES),
            (
                "src/com/example/other/Bag.java",
                "package com.example.other;\n\
                 public class Bag {\n\
                 \x20   public int different;\n\
                 }\n",
            ),
            (
                "src/com/example/UsesNested.java",
                "package com.example;\n\
                 import com.example.common.TestTypes.Bag;\n\
                 public class UsesNested {\n\
                 \x20   private Bag held;\n\
                 }\n",
            ),
            (
                "src/com/example/UsesTopLevel.java",
                "package com.example;\n\
                 import com.example.other.Bag;\n\
                 public class UsesTopLevel {\n\
                 \x20   private Bag held;\n\
                 }\n",
            ),
        ],
    );

    let nested = targets(&files, "UsesNested.java");
    let top = targets(&files, "UsesTopLevel.java");

    assert!(
        nested.iter().any(|t| t.contains("TestTypes.java")),
        "the nested import did not reach the nested class: {nested:?}"
    );
    assert!(
        !nested.iter().any(|t| t.contains("other/Bag.java")),
        "the nested import reached the unrelated top-level class: {nested:?}"
    );
    assert!(
        top.iter().any(|t| t.contains("other/Bag.java")),
        "the top-level import stopped working: {top:?}"
    );
    assert!(
        !top.iter().any(|t| t.contains("TestTypes.java")),
        "the top-level import reached the nested class: {top:?}"
    );
}

#[test]
fn nesting_more_than_one_deep_is_carried_all_the_way() {
    // `Outer.Middle.Inner` is a name Java writes and therefore a name the index
    // has to hold. Recovering only the immediate parent would stop at
    // `Middle.Inner` and miss again, one level further in.
    let files = analyze(
        "deep",
        &[
            (
                "src/com/example/common/Outer.java",
                "package com.example.common;\n\
                 public class Outer {\n\
                 \x20   public static class Middle {\n\
                 \x20       public static class Inner {\n\
                 \x20           public int value;\n\
                 \x20       }\n\
                 \x20   }\n\
                 }\n",
            ),
            (
                "src/com/example/Use.java",
                "package com.example;\n\
                 import com.example.common.Outer.Middle.Inner;\n\
                 public class Use {\n\
                 \x20   private Inner held;\n\
                 }\n",
            ),
        ],
    );

    assert!(
        targets(&files, "Use.java")
            .iter()
            .any(|t| t.contains("Outer.java") && t.contains("Inner")),
        "a class two levels deep was not reached: {:?}",
        targets(&files, "Use.java")
    );
}

#[test]
fn a_sibling_nested_class_is_not_treated_as_a_parent() {
    // Nesting is recovered from line ranges, so the rule has to be containment
    // and not merely "declared earlier". Two siblings sit one after the other
    // inside the same outer class, and neither spans the other — `First` must
    // not become a parent of `Second`.
    let files = analyze(
        "siblings",
        &[
            (
                "src/com/example/common/Pair.java",
                "package com.example.common;\n\
                 public class Pair {\n\
                 \x20   public static class First {\n\
                 \x20       public int a;\n\
                 \x20   }\n\
                 \x20   public static class Second {\n\
                 \x20       public int b;\n\
                 \x20   }\n\
                 }\n",
            ),
            (
                "src/com/example/Use.java",
                "package com.example;\n\
                 import com.example.common.Pair.Second;\n\
                 public class Use {\n\
                 \x20   private Second held;\n\
                 }\n",
            ),
        ],
    );

    assert!(
        targets(&files, "Use.java")
            .iter()
            .any(|t| t.contains("Pair.java") && t.contains("Second")),
        "a sibling was mistaken for an enclosing type, so `Pair.Second` was \
         indexed under some other name: {:?}",
        targets(&files, "Use.java")
    );
}

#[test]
fn a_top_level_class_is_unaffected() {
    // The common case, which must not move. A top-level type has no enclosing
    // type, so its canonical name is its simple name and the index entry is
    // exactly what it always was.
    let files = analyze(
        "top-level",
        &[
            (
                "src/com/example/common/Plain.java",
                "package com.example.common;\n\
                 public class Plain {\n\
                 \x20   public int value;\n\
                 }\n",
            ),
            (
                "src/com/example/Use.java",
                "package com.example;\n\
                 import com.example.common.Plain;\n\
                 public class Use {\n\
                 \x20   private Plain held;\n\
                 }\n",
            ),
        ],
    );

    assert!(
        targets(&files, "Use.java")
            .iter()
            .any(|t| t.contains("Plain.java")),
        "an ordinary top-level import stopped resolving: {:?}",
        targets(&files, "Use.java")
    );
}
