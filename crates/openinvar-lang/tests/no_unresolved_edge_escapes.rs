//! Nothing an adapter failed to bind may reach the graph.
//!
//! Every Tier 1 adapter extracts a reference with a placeholder target and
//! replaces it once it knows the answer, so a placeholder that survives means
//! the adapter could not bind that reference. `dispatch` then stamps every
//! Tier 1 edge `Resolution::Resolved`, which is what a gate reads — so a
//! placeholder reaching the graph is not a cosmetic leak. It is a name sitting
//! where a symbol belongs, marked as a fact a gate may act on, and counted by
//! `blast-radius` as a dependent.
//!
//! TypeScript did exactly that, and the reason is the reason this test is at
//! the dispatch level rather than inside one adapter: TypeScript minted its
//! own placeholder ids with a private prefix, so `is_placeholder` — the shared
//! check every layer above uses — did not recognise them, and every guard in
//! the codebase passed them through. A per-adapter test would not have caught
//! it, because each adapter was doing what it believed was right.

use std::path::PathBuf;

fn analyze(name: &str, files: &[(&str, &str)]) -> openinvar_core::ir::RepoIR {
    let root = std::env::temp_dir().join(format!(
        "openinvar-escape-{name}-{}-{}",
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

/// Every reference in these is deliberately unbindable: the type comes from a
/// package with no source in the tree, or is never declared at all. An adapter
/// that guesses would produce an edge; one that binds honestly produces none.
fn unbindable_sources() -> Vec<(&'static str, Vec<(&'static str, &'static str)>)> {
    vec![
        #[cfg(feature = "typescript")]
        (
            "typescript",
            vec![(
                "src/a.ts",
                "import { Unknowable } from 'some-untyped-pkg';\n\
                 export class Widget {\n\
                 \x20 render(x: Unknowable, y: MissingType): void {\n\
                 \x20   console.log(x.field, y.other);\n\
                 \x20 }\n\
                 }\n",
            )],
        ),
        #[cfg(feature = "python")]
        (
            "python",
            vec![(
                "a.py",
                "from untyped_pkg import Unknowable\n\
                 def render(x: Unknowable, y: \"MissingType\"):\n\
                 \x20   return x.field + y.other\n",
            )],
        ),
        #[cfg(feature = "rust")]
        (
            "rust",
            vec![(
                "a.rs",
                "use some_crate::Unknowable;\n\
                 pub fn render(x: Unknowable, y: MissingType) -> u32 { x.field + y.other }\n",
            )],
        ),
        #[cfg(feature = "go")]
        (
            "go",
            vec![(
                "a.go",
                "package main\n\
                 import \"github.com/x/untyped\"\n\
                 func Render(x untyped.Unknowable, y MissingType) int { return x.Field + y.Other }\n",
            )],
        ),
        #[cfg(feature = "java")]
        (
            "java",
            vec![(
                "A.java",
                "import com.untyped.Unknowable;\n\
                 public class A { public int render(Unknowable x, MissingType y) { return x.field + y.other; } }\n",
            )],
        ),
        #[cfg(feature = "csharp")]
        (
            "csharp",
            vec![(
                "A.cs",
                "using Untyped.Pkg;\n\
                 namespace N { public class A { public int Render(Unknowable x, MissingType y) { return x.Field + y.Other; } } }\n",
            )],
        ),
    ]
}

#[test]
fn no_unresolved_reference_reaches_the_graph() {
    let cases = unbindable_sources();
    assert!(
        !cases.is_empty(),
        "this build carries no Tier 1 language, so the test asserted nothing"
    );

    for (language, files) in cases {
        let ir = analyze(language, &files);
        let escaped: Vec<String> = ir
            .files
            .iter()
            .flat_map(|f| f.relationships.iter())
            .filter(|rel| openinvar_core::symbol_id::is_placeholder(&rel.to))
            .map(|rel| format!("{:?} {} -> {}", rel.kind, rel.from, rel.to))
            .collect();

        assert!(
            escaped.is_empty(),
            "{language} let {} unresolved reference(s) into the graph: {escaped:#?}",
            escaped.len()
        );
    }
}

#[test]
fn an_edge_that_did_resolve_is_still_recorded() {
    // The other half, so the guard above cannot be satisfied by dropping
    // everything. An import that leaves the repository is a fact about it, and
    // an external package node is not a placeholder.
    #[cfg(feature = "typescript")]
    {
        let ir = analyze(
            "kept",
            &[(
                "src/a.ts",
                "import { Thing } from 'real-pkg';\nexport const t: Thing = null as any;\n",
            )],
        );
        let externals: Vec<&str> = ir
            .files
            .iter()
            .flat_map(|f| f.relationships.iter())
            .map(|rel| rel.to.as_str())
            .filter(|to| openinvar_core::symbol_id::is_external_package(to))
            .collect();

        assert!(
            !externals.is_empty(),
            "the guard dropped a resolved external-package edge as well"
        );
    }
}

#[test]
fn every_adapter_agrees_on_what_a_placeholder_looks_like() {
    // The actual defect was a private placeholder format, not a missing drop.
    // An adapter that mints its own ids passes every shared guard silently, so
    // this asserts the vocabulary rather than one adapter's behaviour: any
    // prefix `is_placeholder` does not recognise is invisible to the codebase.
    let ids = [
        openinvar_core::symbol_id::unresolved_local_type_id("Missing"),
        openinvar_core::symbol_id::unresolved_import_id("some-pkg", "Thing"),
    ];
    for id in &ids {
        assert!(
            openinvar_core::symbol_id::is_placeholder(id),
            "{id} is minted as a placeholder but not recognised as one"
        );
    }

    #[cfg(feature = "typescript")]
    {
        use openinvar_lang::lang::typescript::extractor::{
            unresolved_import_symbol_id, unresolved_local_type_symbol_id,
        };
        for id in [
            unresolved_local_type_symbol_id("Missing"),
            unresolved_import_symbol_id("some-pkg", "Thing"),
        ] {
            assert!(
                openinvar_core::symbol_id::is_placeholder(&id),
                "the TypeScript adapter minted {id}, which no shared guard recognises"
            );
        }
    }
}
