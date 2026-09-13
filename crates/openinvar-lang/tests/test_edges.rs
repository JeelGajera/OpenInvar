//! Test detection and the `tests` edges derived from it.
//!
//! Two failure modes matter here and they are not symmetric. A test edge
//! OpenInvar misses is a test it will fail to suggest — the developer runs more
//! of the suite than they needed to. A test edge OpenInvar invents is a claim
//! that a change is covered when it is not, and someone merges on the strength
//! of it. So the cases asserted most heavily are the ones where an edge must
//! *not* appear.

use openinvar_lang::for_path;

fn is_test(path: &str) -> bool {
    for_path(path).is_some_and(|spec| spec.is_test_file(path))
}

#[test]
fn each_language_recognises_its_own_convention() {
    // These are the rules the language's own tooling uses to find tests, not
    // heuristics layered on top of them.
    let cases: &[(&str, bool)] = &[
        // Go: the compiler's own rule.
        ("api/handler_test.go", true),
        ("api/handler.go", false),
        // TypeScript and JavaScript.
        ("src/payload.test.ts", true),
        ("src/payload.spec.ts", true),
        ("src/__tests__/payload.ts", true),
        ("src/payload.ts", false),
        // Python: pytest's collection rules.
        ("test_service.py", true),
        ("service_test.py", true),
        ("conftest.py", true),
        ("service.py", false),
        // Rust: the tests directory, not `#[cfg(test)] mod tests`.
        ("tests/calc_test.rs", true),
        ("crates/thing/tests/golden.rs", true),
        ("src/lib.rs", false),
    ];

    for (path, expected) in cases {
        assert_eq!(
            is_test(path),
            *expected,
            "{path} should{} be a test file",
            if *expected { "" } else { " not" }
        );
    }
}

#[test]
fn a_source_file_whose_name_merely_contains_test_is_not_a_test_file() {
    // `latest.ts` ends in "test". Treating it as a test file would attribute
    // its references to coverage that does not exist.
    for path in [
        "src/latest.ts",
        "src/contest.py",
        "src/protest.go",
        "src/attestation.rs",
    ] {
        assert!(!is_test(path), "{path} was misread as a test file");
    }
}

#[test]
fn a_language_this_build_does_not_carry_claims_nothing() {
    // `for_path` returns None, and the caller must read that as "not a test"
    // rather than panicking or guessing.
    assert!(!is_test("notes.txt"));
    assert!(!is_test("Makefile"));
}

// ── the derived edges ────────────────────────────────────────

use std::path::{Path, PathBuf};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/test-edges")
}

fn analyze() -> openinvar_core::ir::RepoIR {
    let root = fixture();
    let mut files = Vec::new();
    collect(&root, &mut files);
    files.sort();
    openinvar_lang::analyze_files(&root, &files).expect("fixture analyses")
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read fixture dir") {
        let entry = entry.expect("entry");
        if entry.path().is_dir() {
            collect(&entry.path(), out);
        } else {
            out.push(entry.path());
        }
    }
}

/// Every `tests` edge, as `(test file, target symbol id)`.
fn test_edges() -> Vec<(String, String)> {
    use openinvar_core::ir::RelationshipKind;
    let repo = analyze();
    let mut out: Vec<(String, String)> = repo
        .files
        .iter()
        .flat_map(|f| {
            f.relationships
                .iter()
                .filter(|r| r.kind == RelationshipKind::Tests)
                .map(move |r| (f.file.clone(), r.to.clone()))
        })
        .collect();
    out.sort();
    out
}

#[test]
fn a_test_gets_an_edge_to_the_code_it_exercises() {
    let edges = test_edges();
    assert!(!edges.is_empty(), "the fixture produced no test edges at all");

    for (test_file, target) in [
        ("src/payload.test.ts", "src/payload.ts::encode::function"),
        ("test_service.py", "service.py::build::function"),
        ("api/handler_test.go", "api/handler.go::Serve::function"),
    ] {
        assert!(
            edges
                .iter()
                .any(|(f, t)| f == test_file && t == target),
            "{test_file} should test {target}; got {edges:#?}"
        );
    }
}

#[test]
fn a_test_referring_to_another_test_produces_no_edge() {
    // `helper.test.ts` calls `payload.test.ts`. A helper exercising a helper
    // is not coverage of the code under test, and counting it would make every
    // test appear to cover the whole suite.
    let edges = test_edges();
    assert!(
        !edges.iter().any(|(f, _)| f == "src/helper.test.ts"),
        "a test-to-test reference was counted as coverage: {edges:#?}"
    );
}

#[test]
fn an_external_package_is_never_reported_as_tested() {
    // A test importing `std` does not test `std`. These parse as symbol ids,
    // so they have to be refused explicitly.
    let edges = test_edges();
    for (_, target) in &edges {
        assert!(
            !target.starts_with("ext::"),
            "an external package was reported as tested: {target}"
        );
    }
}

#[test]
fn the_underlying_reference_survives_alongside_the_test_edge() {
    // The `tests` edge is added, not substituted. A query for callers that
    // silently dropped tests would under-report the blast radius.
    use openinvar_core::ir::RelationshipKind;
    let repo = analyze();
    let payload_test = repo
        .files
        .iter()
        .find(|f| f.file == "src/payload.test.ts")
        .expect("the fixture has this file");

    let target = "src/payload.ts::encode::function";
    assert!(
        payload_test
            .relationships
            .iter()
            .any(|r| r.to == target && r.kind == RelationshipKind::Calls),
        "the original call edge was replaced rather than supplemented"
    );
    assert!(payload_test
        .relationships
        .iter()
        .any(|r| r.to == target && r.kind == RelationshipKind::Tests));
}

#[test]
fn the_same_symbol_exercised_twice_yields_one_edge() {
    // `payload.test.ts` both imports and calls `encode`. It exercises it once.
    let edges = test_edges();
    let count = edges
        .iter()
        .filter(|(f, t)| f == "src/payload.test.ts" && t == "src/payload.ts::encode::function")
        .count();
    assert_eq!(count, 1, "duplicate test edges inflate every coverage count");
}

#[test]
fn analysis_is_reproducible() {
    let first = test_edges();
    for _ in 0..3 {
        assert_eq!(test_edges(), first, "test edges must not vary between runs");
    }
}
