// Exercises the rust module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "rust")]

//! Resolution across a Cargo workspace.
//!
//! Resolution assumed one crate rooted at `<repo>/src/`, so in a workspace
//! nothing resolved at all: OpenInvar could not analyze itself, and
//! `blast-radius RepoIR` reported a type used in dozens of files as safe to
//! modify. A tool that answers "safe" wrongly is worse than one that declines
//! to answer, so these cover each way a crate can be named rather than only
//! the common one.

use std::path::{Path, PathBuf};

use openinvar_core::ir::{FileIR, RelationshipKind, RepoIR};
use openinvar_lang::lang::rust::analyze_files;

fn fixture_root(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../fixtures/adapter-rust/{name}"))
}

fn all_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in walkdir::WalkDir::new(root).into_iter().flatten() {
        if e.path().is_file() && matches!(e.path().extension().and_then(|x| x.to_str()), Some("rs"))
        {
            out.push(e.path().to_path_buf());
        }
    }
    out.sort();
    out
}

fn workspace() -> RepoIR {
    let root = fixture_root("workspace");
    analyze_files(&root, &all_files(&root)).expect("workspace analysis must succeed")
}

fn file<'a>(ir: &'a RepoIR, suffix: &str) -> &'a FileIR {
    ir.files
        .iter()
        .find(|f| f.file.ends_with(suffix))
        .unwrap_or_else(|| panic!("{suffix} is present in the fixture"))
}

/// The resolved target of the import on `line`, if it resolved at all.
fn import_target(ir: &RepoIR, suffix: &str, line: u32) -> Option<String> {
    file(ir, suffix)
        .relationships
        .iter()
        .find(|r| r.kind == RelationshipKind::Imports && r.line == line)
        .map(|r| r.to.clone())
}

#[test]
fn an_inter_crate_import_resolves_through_a_re_export() {
    // `use core_lib::models::UserPayload` crosses two boundaries at once: the
    // package is `core-lib` and the path segment is `core_lib`, and
    // `UserPayload` is declared in `models::payload` and only re-exported by
    // `models`. Both have to work for the edge to land on the declaration.
    let ir = workspace();
    let target = import_target(&ir, "app/src/service.rs", 4).expect("the import resolved");
    assert!(
        target.ends_with("core-lib/src/models/payload.rs::UserPayload::class"),
        "expected the declaring module, got {target}"
    );
}

#[test]
fn a_dependency_rename_resolves_to_the_real_package() {
    // `aliased_core = { package = "core-lib" }` means `use aliased_core::…`
    // is core-lib's. Nothing in the path spells the package's own name.
    let ir = workspace();
    let target = import_target(&ir, "app/src/service.rs", 7).expect("the renamed import resolved");
    assert!(
        target.ends_with("core-lib/src/models/payload.rs::UserPayload::class"),
        "expected core-lib through its rename, got {target}"
    );
}

#[test]
fn a_lib_name_override_is_what_source_names() {
    // The package is `renamed-lib`; its library is `kernel`. Source says
    // `use kernel::…`, and the package name never appears.
    let ir = workspace();
    let target = import_target(&ir, "app/src/service.rs", 10).expect("the kernel import resolved");
    assert!(
        target.ends_with("renamed-lib/src/lib.rs::Kernel::class"),
        "expected the crate named by [lib] name, got {target}"
    );
}

#[test]
fn a_binary_can_name_its_own_packages_library() {
    // `src/main.rs` and `src/lib.rs` sit in one package and share a directory.
    // The binary reaches the library by the package's extern name, not by
    // `crate::`, and both roots have to exist for that to resolve.
    let ir = workspace();
    let target = import_target(&ir, "app/src/main.rs", 2).expect("the bin -> lib import resolved");
    assert!(
        target.ends_with("app/src/service.rs::Service::class"),
        "expected the library's Service, got {target}"
    );
}

#[test]
fn a_package_excluded_from_the_workspace_still_resolves_its_own_paths() {
    // `exclude` removes a package from the workspace, not from the world. Its
    // `crate::` still means itself.
    let ir = workspace();
    let target =
        import_target(&ir, "legacy/src/api.rs", 3).expect("the intra-crate import resolved");
    assert!(
        target.ends_with("legacy/src/store.rs::LegacyStore::class"),
        "expected the excluded package's own module, got {target}"
    );
}

#[test]
fn property_access_is_attributed_across_crates() {
    // The point of resolving the import is what it enables: a field read on a
    // value declared in another crate is recorded against the declaring type.
    let ir = workspace();
    let service = file(&ir, "app/src/service.rs");

    let mut attributed: Vec<(&str, &str)> = service
        .relationships
        .iter()
        .filter(|r| r.kind == RelationshipKind::AccessesProperty)
        .flat_map(|r| {
            r.properties_accessed
                .iter()
                .map(move |p| (r.to.as_str(), p.as_str()))
        })
        .collect();
    attributed.sort_unstable();

    assert!(
        attributed
            .iter()
            .any(|(to, prop)| to.contains("payload.rs::UserPayload") && *prop == "user_id"),
        "user_id must be attributed to UserPayload in another crate: {attributed:?}"
    );
    assert!(
        attributed
            .iter()
            .any(|(to, prop)| to.contains("renamed-lib/src/lib.rs::Kernel") && *prop == "id"),
        "id must be attributed to Kernel: {attributed:?}"
    );
}

#[test]
fn the_workspace_resolves_without_diagnostics() {
    // Every path in the fixture is resolvable, so any diagnostic is a real
    // failure rather than an honest report of a genuine limit.
    let ir = workspace();
    let complaints: Vec<String> = ir
        .files
        .iter()
        .flat_map(|f| {
            f.diagnostics
                .iter()
                .map(|d| format!("{}: {}", f.file, d.message))
        })
        .collect();
    assert!(
        complaints.is_empty(),
        "unexpected diagnostics: {complaints:#?}"
    );
}

#[test]
fn a_single_crate_tree_without_a_manifest_still_resolves() {
    // Not every tree OpenInvar is pointed at ships a Cargo.toml — a fixture, a
    // subdirectory, a vendored source drop. `src/lib.rs` is a crate root
    // regardless, and requiring the manifest would have silently reclassified
    // these paths as third-party rather than resolving them.
    let root = fixture_root("alias_import_bug");
    let ir = analyze_files(&root, &all_files(&root)).expect("analysis succeeds");
    let mapper = file(&ir, "view_model_mapper.rs");

    let aliased = mapper
        .relationships
        .iter()
        .find(|r| {
            r.kind == RelationshipKind::Imports && r.alias.as_deref() == Some("ResponseModel")
        })
        .expect("the aliased import is present");
    assert!(
        aliased.to.contains("UserPayload"),
        "expected UserPayload, got {}",
        aliased.to
    );
}
