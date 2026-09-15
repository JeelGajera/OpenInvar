//! A generic type parameter is not a missing type.
//!
//! `fn label<T: Identify>(&self, subject: &T)` declares `T` in its own
//! signature. Binding `subject` to `T` would leave `T` permanently
//! unresolvable, so the scope analyser collects the parameter names and filters
//! them out.
//!
//! # Why this file exists
//!
//! That filter reads the parse tree for node kinds, and tree-sitter-rust 0.24
//! replaced three of them — a bare `type_identifier`, `constrained_type_parameter`
//! and `optional_type_parameter` — with one `type_parameter`. The filter went on
//! matching names the grammar no longer produced.
//!
//! It failed **silently**, which is the point. An empty set filters nothing, so
//! nothing errored and nothing crashed: every generic function simply began
//! reporting its own type parameter as a type it could not find. The only
//! visible trace was one extra warning in one golden file.
//!
//! So these assert the *filter*, not just that analysis succeeded. A test that
//! only checked "no panic" would have passed throughout.

#![cfg(feature = "rust")]

use std::path::PathBuf;

use openinvar_core::ir::RepoIR;

fn analyze(name: &str, files: &[(&str, &str)]) -> RepoIR {
    let root = std::env::temp_dir().join(format!(
        "openinvar-generics-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let mut paths: Vec<PathBuf> = Vec::new();
    for (path, source) in files {
        let full = root.join(path);
        std::fs::create_dir_all(full.parent().expect("parent")).expect("mkdir");
        std::fs::write(&full, source).expect("write");
        paths.push(full);
    }
    paths.sort();
    let ir = openinvar_lang::dispatch::analyze_files(&root, &paths).expect("analysis");
    let _ = std::fs::remove_dir_all(&root);
    ir
}

/// Names the analysis said it could not resolve.
fn unresolved_types(ir: &RepoIR) -> Vec<String> {
    let mut out: Vec<String> = ir
        .files
        .iter()
        .flat_map(|f| f.diagnostics.iter())
        .filter_map(|d| {
            d.message
                .split_once("unable to resolve type '")
                .and_then(|(_, rest)| rest.split_once('\''))
                .map(|(name, _)| name.to_string())
        })
        .collect();
    out.sort();
    out
}

fn unbound(ir: &RepoIR) -> u32 {
    ir.files.iter().map(|f| f.unbound_references).sum()
}

#[test]
fn a_functions_own_type_parameter_is_not_a_missing_type() {
    // The exact shape that regressed.
    let ir = analyze(
        "fn-generic",
        &[(
            "src/lib.rs",
            "pub trait Identify { fn identity(&self) -> String; }\n\
             pub struct Reporter;\n\
             impl Reporter {\n\
             \x20   pub fn label<T: Identify>(&self, subject: &T) -> String { subject.identity() }\n\
             }\n",
        )],
    );

    assert!(
        !unresolved_types(&ir).contains(&"T".to_string()),
        "a type parameter declared in the same signature was reported missing: {:?}",
        unresolved_types(&ir)
    );
    assert_eq!(unbound(&ir), 0, "it was also counted against coverage");
}

#[test]
fn a_type_parameter_on_the_impl_is_in_scope_for_its_methods() {
    // `impl<T> Store<T>` declares `T` once for every method inside it, so the
    // filter has to walk outward from the function, not just read its own
    // signature.
    let ir = analyze(
        "impl-generic",
        &[(
            "src/lib.rs",
            "pub trait Identify { fn identity(&self) -> String; }\n\
             pub struct Store<T> { item: T }\n\
             impl<T: Identify> Store<T> {\n\
             \x20   pub fn name(&self) -> String { self.item.identity() }\n\
             }\n",
        )],
    );

    assert!(
        !unresolved_types(&ir).contains(&"T".to_string()),
        "a type parameter from the enclosing impl was reported missing: {:?}",
        unresolved_types(&ir)
    );
}

#[test]
fn every_parameter_shape_the_grammar_produces_is_collected() {
    // The regression was one node kind going unmatched, so one shape is not
    // enough to pin this. `T` is bare, `U` is bounded, `V` has a default, and
    // `N` and `'a` are a const and a lifetime parameter sitting in the same
    // list — neither names a type, and neither may break the walk over the
    // ones that do.
    let ir = analyze(
        "all-shapes",
        &[(
            "src/lib.rs",
            "pub trait Identify { fn identity(&self) -> String; }\n\
             pub struct Wrap;\n\
             impl Wrap {\n\
             \x20   pub fn go<'a, T, U: Identify, V: Identify = Wrap, const N: usize>(\n\
             \x20       &self, t: &'a T, u: &U, v: &V,\n\
             \x20   ) -> String { let _ = t; let _ = v; u.identity() }\n\
             }\n\
             impl Identify for Wrap { fn identity(&self) -> String { String::new() } }\n",
        )],
    );

    let missing = unresolved_types(&ir);
    for name in ["T", "U", "V"] {
        assert!(
            !missing.contains(&name.to_string()),
            "type parameter `{name}` was reported missing: {missing:?}"
        );
    }
}

#[test]
fn a_genuinely_missing_type_is_still_reported() {
    // The other half, and the one that stops the fix being "suppress
    // everything". `Absent` is declared nowhere and is not a type parameter, so
    // the analysis must still say it could not place it — otherwise the filter
    // would be hiding real gaps rather than phantom ones.
    let ir = analyze(
        "real-miss",
        &[(
            "src/lib.rs",
            "pub struct Holder;\n\
             impl Holder {\n\
             \x20   pub fn read(&self, thing: &Absent) -> String { thing.field.clone() }\n\
             }\n",
        )],
    );

    assert!(
        unresolved_types(&ir).contains(&"Absent".to_string()),
        "a type that really is missing was filtered away too: {:?}",
        unresolved_types(&ir)
    );
}
