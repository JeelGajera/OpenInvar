//! Tier 2, end to end — PHP.
//!
//! Mirrors `ruby_structural.rs`. Half of what these assert is what Tier 2
//! *cannot* do: a structural language that quietly appeared to resolve across
//! files would be worse than one that openly does not, because every gate in
//! the later phases decides how much to trust a region by its tier.

#![cfg(feature = "php")]

use std::path::{Path, PathBuf};

use openinvar_core::ir::{Language, RepoIR, SymbolKind};
use openinvar_lang::spec::Tier;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/adapter-php/basic")
}

fn all_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in walkdir::WalkDir::new(root).into_iter().flatten() {
        if e.path().is_file()
            && matches!(e.path().extension().and_then(|x| x.to_str()), Some("php"))
        {
            out.push(e.path().to_path_buf());
        }
    }
    out.sort();
    out
}

fn analyzed() -> RepoIR {
    let root = fixture_root();
    openinvar_lang::dispatch::analyze_files(&root, &all_files(&root)).expect("analysis must succeed")
}

#[test]
fn the_grammars_own_tags_query_extracts_symbols() {
    // The claim the tier rests on: no query file was written for PHP, and
    // symbols still come out. A grammar whose `TAGS_QUERY` is absent or empty
    // would silently produce nothing, which is why this asserts a count rather
    // than that analysis merely succeeded.
    let ir = analyzed();
    let named: Vec<&str> = ir
        .files
        .iter()
        .flat_map(|f| f.symbols.iter())
        .filter(|s| s.kind != SymbolKind::Module)
        .map(|s| s.name.as_str())
        .collect();

    assert!(
        named.len() >= 4,
        "the tags query extracted almost nothing: {named:?}"
    );
    assert!(
        named.contains(&"UserService"),
        "expected UserService among the symbols, got {named:?}"
    );
}

#[test]
fn the_dialect_this_adapter_chose_parses_a_template_cleanly() {
    // The dialect choice, pinned at the grammar. `tree-sitter-php` ships two:
    // `LANGUAGE_PHP` parses PHP embedded in text, `LANGUAGE_PHP_ONLY` parses
    // the code alone. A `.php` file on disk is the former — `<?php` opens a
    // region and everything outside it is markup.
    //
    // This asserts on the parse tree rather than on extracted symbols, because
    // symbols do not tell the two apart. tree-sitter recovers from the error
    // and the tags query still finds `render_title`, so an adapter wired to
    // the wrong dialect produces the same IR on this fixture and every test
    // above it passes. The difference is only visible in the tree.
    let spec =
        openinvar_lang::spec::for_language(&Language::Php).expect("this build carries PHP");
    let source = std::fs::read_to_string(fixture_root().join("template.php"))
        .expect("the template fixture is readable");

    let parse_with = |language: tree_sitter::Language| {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&language).expect("grammar loads");
        parser.parse(&source, None).expect("a tree comes back")
    };

    let chosen = parse_with(spec.grammar().expect("PHP has a grammar"));
    assert!(
        !chosen.root_node().has_error(),
        "the dialect this adapter exposes cannot parse a file that mixes PHP \
         with markup, which is what a .php file is"
    );

    // And the other dialect cannot, so the fixture is still telling them apart.
    // Were upstream to make both handle markup, this would start passing for a
    // reason that has nothing to do with the choice, and the assertion above
    // would be pinning nothing.
    let code_only = parse_with(tree_sitter_php::LANGUAGE_PHP_ONLY.into());
    assert!(
        code_only.root_node().has_error(),
        "both dialects now parse the template cleanly, so this fixture no \
         longer distinguishes them and no longer pins the adapter's choice"
    );
}

#[test]
fn every_file_is_recorded_as_this_language() {
    let ir = analyzed();
    assert!(!ir.files.is_empty(), "no files were analysed at all");
    assert!(
        ir.files.iter().all(|f| f.language == Language::Php),
        "a file was attributed to the wrong language"
    );
}

#[test]
fn every_edge_is_structural() {
    // The property that makes broad coverage safe. A gate reads resolution per
    // edge, so a Tier 2 edge that claimed to be resolved would be acted on.
    let ir = analyzed();
    let resolutions: Vec<_> = ir
        .files
        .iter()
        .flat_map(|f| f.relationships.iter())
        .map(|r| (r.file.as_str(), r.line, r.resolution))
        .collect();

    assert!(!resolutions.is_empty(), "no edges at all to check");
    assert!(
        resolutions
            .iter()
            .all(|(_, _, res)| *res == openinvar_core::ir::Resolution::Structural),
        "a Tier 2 edge claimed to be resolved: {resolutions:?}"
    );
}

#[test]
fn nothing_resolves_across_files() {
    // `Elsewhere` references a class declared in the other file, and even has
    // a `use App\Service\UserService;` naming it. Tier 2 does not resolve
    // across files, so no edge may point out of the file it was written in —
    // silence here is the honest answer, and `status` reports the tier so it
    // is not mistaken for "nothing uses it".
    let ir = analyzed();

    for file in &ir.files {
        for rel in &file.relationships {
            assert!(
                rel.to.starts_with(&file.file) || !rel.to.contains("::"),
                "{} points at another file: {}",
                file.file,
                rel.to
            );
        }
    }
}

#[test]
fn the_spec_reports_tier_two() {
    let spec =
        openinvar_lang::spec::for_language(&Language::Php).expect("this build carries PHP");
    assert_eq!(spec.tier(), Tier::Structural);
    assert!(
        !spec.tier().is_gate_safe(),
        "PHP must never be gate-safe while it resolves nothing across files"
    );
}

#[test]
fn phpunit_test_files_are_recognised() {
    // Test detection is by convention, and PHPUnit's is a `Test` suffix rather
    // than a prefix — `UserServiceTest.php`, not `TestUserService.php`. Getting
    // it backwards would file every test as ordinary source and every source
    // file as a test in the ecosystems that use the other order.
    let spec =
        openinvar_lang::spec::for_language(&Language::Php).expect("this build carries PHP");

    for path in [
        "tests/UserServiceTest.php",
        "src/Service/UserServiceTest.php",
        "tests/Unit/AnyName.php",
    ] {
        assert!(spec.is_test_file(path), "{path} was not recognised as a test");
    }
    for path in ["src/UserService.php", "src/Testing.php", "src/Contest.php"] {
        assert!(
            !spec.is_test_file(path),
            "{path} was wrongly recognised as a test"
        );
    }
}
