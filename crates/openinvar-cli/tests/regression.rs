//! Cross-language regressions for the defects that shipped in the
//! multi-language adapter branch.
//!
//! Each of these failed on the original implementation. They live in one file,
//! run through the dispatch layer, so a fix in one adapter cannot quietly
//! regress another.

use std::path::{Path, PathBuf};

use openinvar_core::ir::{RelationshipKind, RepoIR, SymbolKind};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../fixtures/regression/{name}"))
}

fn all_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(root).into_iter().flatten() {
        if entry.path().is_file() {
            out.push(entry.path().to_path_buf());
        }
    }
    out.sort();
    out
}

fn analyze(name: &str) -> RepoIR {
    let root = fixture(name);
    openinvar_lang::analyze_files(&root, &all_files(&root))
        .unwrap_or_else(|e| panic!("analysis of '{name}' failed: {e}"))
}

/// Properties recorded against the symbol whose id ends with `target_suffix`.
fn properties_for(repo: &RepoIR, target_suffix: &str) -> Vec<String> {
    let mut out: Vec<String> = repo
        .files
        .iter()
        .flat_map(|f| f.relationships.iter())
        .filter(|r| r.kind == RelationshipKind::AccessesProperty && r.to.ends_with(target_suffix))
        .flat_map(|r| r.properties_accessed.iter().cloned())
        .collect();
    out.sort();
    out.dedup();
    assert!(
        !out.is_empty(),
        "no properties recorded for '{target_suffix}'; edges were: {:?}",
        repo.files
            .iter()
            .flat_map(|f| f.relationships.iter())
            .map(|r| format!("{:?} -> {}", r.kind, r.to))
            .collect::<Vec<_>>()
    );
    out
}

// ── B-01: property tracking must not depend on a variable's name ─

/// The original resolvers began their property-attribution check by comparing
/// the receiver against the literal string `"data"` — the variable name used in
/// every fixture. These fixtures name their variables `whatever` and
/// `anything`, so they fail against that implementation in all four languages.
#[test]
fn rust_property_tracking_works_for_any_variable_name() {
    let repo = analyze("rust");
    assert_eq!(
        properties_for(&repo, "src/alpha.rs::Alpha::class"),
        vec!["a_field".to_string()]
    );
}

#[test]
fn go_property_tracking_works_for_any_variable_name() {
    let repo = analyze("go");
    assert_eq!(
        properties_for(&repo, "models/alpha.go::Alpha::class"),
        vec!["AField".to_string()]
    );
}

#[test]
fn python_property_tracking_works_for_any_variable_name() {
    let repo = analyze("py");
    assert_eq!(
        properties_for(&repo, "models/types.py::Alpha::class"),
        vec!["a_field".to_string()]
    );
}

#[test]
fn c_property_tracking_works_for_any_variable_name() {
    let repo = analyze("c");
    assert_eq!(
        properties_for(&repo, "include/types.h::Alpha::class"),
        vec!["a_field".to_string()]
    );
}

// ── H-07: properties belong to one type, not to the whole file ───

/// Properties used to be collected file-wide and unioned onto every edge, so a
/// struct was reported as having another struct's fields. Each fixture touches
/// two types in one file with disjoint field names.
#[test]
fn properties_are_not_shared_between_types_in_any_language() {
    for (language, alpha, beta, a_field, b_field) in [
        (
            "rust",
            "src/alpha.rs::Alpha::class",
            "src/beta.rs::Beta::class",
            "a_field",
            "b_field",
        ),
        (
            "go",
            "models/alpha.go::Alpha::class",
            "models/alpha.go::Beta::class",
            "AField",
            "BField",
        ),
        (
            "py",
            "models/types.py::Alpha::class",
            "models/types.py::Beta::class",
            "a_field",
            "b_field",
        ),
        (
            "c",
            "include/types.h::Alpha::class",
            "include/types.h::Beta::class",
            "a_field",
            "b_field",
        ),
    ] {
        let repo = analyze(language);

        let alpha_props = properties_for(&repo, alpha);
        let beta_props = properties_for(&repo, beta);

        assert_eq!(
            alpha_props,
            vec![a_field.to_string()],
            "{language}: Alpha should have only its own field"
        );
        assert_eq!(
            beta_props,
            vec![b_field.to_string()],
            "{language}: Beta should have only its own field"
        );
    }
}

// ── B-02 / B-03: C produces a connected, unambiguous graph ───────

#[test]
fn c_includes_produce_edges_the_graph_can_address() {
    // Includes previously resolved to `local_header::<name>`, which
    // `add_relationship` drops, so a C repository built a graph with no edges.
    let repo = analyze("c");
    let svc = repo
        .files
        .iter()
        .find(|f| f.file.ends_with("src/svc.c"))
        .expect("svc.c is analysed");

    assert!(
        svc.relationships.iter().any(|r| {
            r.kind == RelationshipKind::Imports && r.to.ends_with("include/types.h::module::module")
        }),
        "have: {:?}",
        svc.relationships
            .iter()
            .map(|r| r.to.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_c_type_is_defined_exactly_once_however_many_files_mention_it() {
    // `Alpha *whatever` in a signature is a `struct_specifier` too; treating it
    // as a definition split one type across every file that used it and made
    // `blast-radius` report the name as ambiguous.
    let repo = analyze("c");
    let definitions: Vec<&str> = repo
        .files
        .iter()
        .flat_map(|f| f.symbols.iter())
        .filter(|s| s.name == "Alpha" && s.kind != SymbolKind::Module)
        .map(|s| s.id.as_str())
        .collect();

    assert_eq!(definitions.len(), 1, "got {definitions:?}");
}

// ── H-05: a Go package import is stable ──────────────────────────

#[test]
fn a_go_package_import_does_not_resolve_to_a_member() {
    // The import edge used to point at `symbol_ids.first()`, so adding a file
    // to the package silently moved every edge into it.
    let repo = analyze("go");
    let svc = repo
        .files
        .iter()
        .find(|f| f.file.ends_with("svc/svc.go"))
        .expect("svc.go is analysed");

    let import = svc
        .relationships
        .iter()
        .find(|r| r.kind == RelationshipKind::Imports)
        .expect("the models import is recorded");

    assert!(
        import.to.ends_with("models::models::module"),
        "an import names a package, not one of its types; got {}",
        import.to
    );
}

// ── H-06: local Python imports are never invented dependencies ───

#[test]
fn a_local_python_import_is_not_reported_as_a_third_party_package() {
    let repo = analyze("py");
    let external: Vec<&str> = repo
        .files
        .iter()
        .flat_map(|f| f.relationships.iter())
        .filter(|r| r.to.starts_with("ext::"))
        .map(|r| r.to.as_str())
        .collect();

    assert!(
        external.is_empty(),
        "everything in this fixture is local; got {external:?}"
    );
}

// ── invariants that hold for every language ──────────────────────

#[test]
fn no_adapter_leaves_an_unresolved_placeholder_in_its_output() {
    // A placeholder that reaches the graph is an edge `add_relationship` drops
    // without telling anyone. Resolvers must either resolve it, rewrite it to
    // an external package, or remove it with a diagnostic.
    for language in ["rust", "go", "py", "c"] {
        let repo = analyze(language);
        for f in &repo.files {
            for rel in &f.relationships {
                assert!(
                    !openinvar_core::symbol_id::is_placeholder(&rel.to),
                    "{language}/{} leaked a placeholder: {}",
                    f.file,
                    rel.to
                );
            }
        }
    }
}

#[test]
fn analysis_is_reproducible_across_runs() {
    // `RepoIR.files` ordering determines graph construction order, and
    // determinism is the project's first documented guarantee. Grouping used a
    // `HashMap`, whose iteration order varies per process.
    for language in ["rust", "go", "py", "c"] {
        let first = analyze(language);
        let second = analyze(language);

        let order =
            |repo: &RepoIR| -> Vec<String> { repo.files.iter().map(|f| f.file.clone()).collect() };
        assert_eq!(
            order(&first),
            order(&second),
            "{language}: file order must be stable"
        );

        let edges = |repo: &RepoIR| -> Vec<String> {
            repo.files
                .iter()
                .flat_map(|f| f.relationships.iter())
                .map(|r| format!("{}|{:?}|{}", r.from, r.kind, r.to))
                .collect()
        };
        assert_eq!(
            edges(&first),
            edges(&second),
            "{language}: edge order must be stable"
        );
    }
}
