//! Importing a name a module does not declare.
//!
//! A framework single-file component *is* its file. `UserCard.vue` declares no
//! symbol called `UserCard` — its `<script setup>` block makes the component
//! the module itself — so `import { UserCard } from './UserCard.vue'` can never
//! find a matching declaration, and the reference was dropped. The component
//! then had no dependents at all, which is the one answer `blast-radius` must
//! never give wrongly.
//!
//! An import creates a dependency on the file whether or not the name inside it
//! resolves, and that much is true, so it is recorded. This is the convention
//! the Rust adapter already follows for its `UnknownMember` case.
//!
//! The line is drawn at re-exports: `export { foo } from './b'` claims to
//! forward one specific name, so when that name does not exist, forwarding
//! nothing is the honest answer and a file-level edge would overstate it.

#![cfg(feature = "typescript")]

use std::path::PathBuf;

use openinvar_core::ir::{RelationshipKind, RepoIR};

fn analyze(name: &str, files: &[(&str, &str)]) -> RepoIR {
    let root = std::env::temp_dir().join(format!(
        "openinvar-tscomp-{name}-{}-{}",
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

fn targets(ir: &RepoIR, file: &str, kind: RelationshipKind) -> Vec<String> {
    let mut out: Vec<String> = ir
        .files
        .iter()
        .filter(|f| f.file.ends_with(file))
        .flat_map(|f| f.relationships.iter())
        .filter(|r| r.kind == kind)
        .map(|r| r.to.clone())
        .collect();
    out.sort();
    out
}

fn unbound(ir: &RepoIR) -> u32 {
    ir.files.iter().map(|f| f.unbound_references).sum()
}

#[test]
fn a_vue_component_import_reaches_the_component_file() {
    let ir = analyze(
        "vue",
        &[
            (
                "src/components/UserCard.vue",
                "<template><div>{{ user.name }}</div></template>\n\
                 <script setup lang=\"ts\">\n\
                 defineProps<{ user: string }>();\n\
                 </script>\n",
            ),
            (
                "src/App.vue",
                "<template><UserCard :user=\"u\" /></template>\n\
                 <script setup lang=\"ts\">\n\
                 import { UserCard } from \"./components/UserCard.vue\";\n\
                 </script>\n",
            ),
        ],
    );

    assert!(
        targets(&ir, "App.vue", RelationshipKind::Imports)
            .iter()
            .any(|t| t.contains("UserCard.vue")),
        "the component import reached nothing: {:?}",
        targets(&ir, "App.vue", RelationshipKind::Imports)
    );
    assert_eq!(unbound(&ir), 0, "a reference was left unbound");
}

#[test]
fn an_astro_component_import_reaches_the_component_file() {
    let ir = analyze(
        "astro",
        &[
            (
                "src/layouts/Layout.astro",
                "---\nconst { title } = Astro.props;\n---\n<html><body><slot /></body></html>\n",
            ),
            (
                "src/pages/index.astro",
                "---\nimport { Layout } from \"../layouts/Layout.astro\";\n---\n<Layout />\n",
            ),
        ],
    );

    assert!(
        targets(&ir, "index.astro", RelationshipKind::Imports)
            .iter()
            .any(|t| t.contains("Layout.astro")),
        "the component import reached nothing: {:?}",
        targets(&ir, "index.astro", RelationshipKind::Imports)
    );
}

#[test]
fn an_import_of_a_name_that_exists_still_binds_to_the_symbol() {
    // The fallback must not shadow real resolution. If it did, every import
    // would collapse to a file-level edge and every `usages` answer would name
    // files rather than symbols.
    let ir = analyze(
        "named",
        &[
            ("src/user.ts", "export interface User { id: string }\n"),
            (
                "src/app.ts",
                "import { User } from './user';\nexport function go(u: User) { return u.id; }\n",
            ),
        ],
    );

    let imports = targets(&ir, "app.ts", RelationshipKind::Imports);
    assert!(
        imports.iter().any(|t| t.ends_with("User::interface")),
        "an import of a declared name fell back to the module: {imports:?}"
    );
    assert!(
        !imports.iter().any(|t| t.ends_with("module::module")),
        "the fallback fired for a name that resolves: {imports:?}"
    );
}

#[test]
fn a_re_export_of_a_name_that_does_not_exist_stays_unresolved() {
    // The deliberate limit. `foo` is declared nowhere — the two files forward it
    // to each other — so nothing may claim to have found it. A file-level edge
    // here would say "a re-exports something from b", which is not true of a
    // name that does not exist.
    let ir = analyze(
        "reexport-cycle",
        &[
            ("src/a.ts", "export { foo } from './b';\n"),
            ("src/b.ts", "export { foo } from './a';\n"),
        ],
    );

    assert!(
        targets(&ir, "a.ts", RelationshipKind::ReExports).is_empty(),
        "a re-export of a non-existent name was given a target: {:?}",
        targets(&ir, "a.ts", RelationshipKind::ReExports)
    );
    assert!(
        unbound(&ir) > 0,
        "the unresolvable re-exports were not counted as unbound"
    );
}

#[test]
fn the_component_edge_makes_the_component_reachable() {
    // The point of the fix, rather than its mechanism. Before it, a Vue
    // component had no dependents recorded anywhere in the graph, so anything
    // reasoning about what breaks if it changes saw nothing.
    let ir = analyze(
        "reachable",
        &[
            (
                "src/components/UserCard.vue",
                "<script setup lang=\"ts\">\ndefineProps<{ user: string }>();\n</script>\n",
            ),
            (
                "src/App.vue",
                "<script setup lang=\"ts\">\nimport { UserCard } from \"./components/UserCard.vue\";\n</script>\n",
            ),
        ],
    );

    let reaching_component: Vec<_> = ir
        .files
        .iter()
        .flat_map(|f| f.relationships.iter())
        .filter(|r| r.to.contains("UserCard.vue"))
        .collect();

    assert!(
        !reaching_component.is_empty(),
        "nothing in the graph points at the component, so it has no dependents"
    );
}
