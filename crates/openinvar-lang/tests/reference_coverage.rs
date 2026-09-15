//! Counting the references the analysis could not place.
//!
//! Resolution coverage answers "can a gate act on what is in the graph". It
//! cannot answer "how much of the source is in the graph", and it reads as if
//! it can. An edge exists only once something bound it, so a reference that
//! failed to bind leaves nothing to count — it never enters the denominator,
//! and failing to bind *more* references raises the percentage.
//!
//! These pin the count that supplies the missing denominator. The property
//! that matters most is the last one: adding the measurement must not change
//! what the analysis records.

use std::path::PathBuf;

fn analyze(name: &str, files: &[(&str, &str)]) -> openinvar_core::ir::RepoIR {
    let root = std::env::temp_dir().join(format!(
        "openinvar-refcov-{name}-{}-{}",
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

fn unbound(ir: &openinvar_core::ir::RepoIR) -> u32 {
    ir.files.iter().map(|f| f.unbound_references).sum()
}

/// Only used by the all-bound case, which needs one language with a resolver.
/// A slim build without it compiles this file with the helper unreferenced.
#[allow(dead_code)]
fn edges(ir: &openinvar_core::ir::RepoIR) -> usize {
    ir.files.iter().map(|f| f.relationships.len()).sum()
}

/// One unbindable reference per language: a type from a package with no source
/// in the tree, and a type never declared anywhere.
fn unbindable() -> Vec<(&'static str, Vec<(&'static str, &'static str)>)> {
    vec![
        #[cfg(feature = "typescript")]
        (
            "typescript",
            vec![(
                "src/a.ts",
                "import { Unknowable } from 'some-untyped-pkg';\n\
                 export class Widget {\n\
                 \x20 render(x: Unknowable, y: MissingType): void { console.log(x.field, y.other); }\n\
                 }\n",
            )],
        ),
        #[cfg(feature = "python")]
        (
            "python",
            vec![(
                "a.py",
                "def render(y: \"MissingType\"):\n\x20   return y.other\n",
            )],
        ),
        #[cfg(feature = "rust")]
        (
            "rust",
            vec![("a.rs", "pub fn render(y: MissingType) -> u32 { y.other }\n")],
        ),
        #[cfg(feature = "go")]
        (
            "go",
            vec![(
                "a.go",
                "package main\nfunc Render(y MissingType) int { return y.Other }\n",
            )],
        ),
        #[cfg(feature = "java")]
        (
            "java",
            vec![(
                "A.java",
                "public class A { public int render(MissingType y) { return y.other; } }\n",
            )],
        ),
        #[cfg(feature = "csharp")]
        (
            "csharp",
            vec![(
                "A.cs",
                "namespace N { public class A { public int Render(MissingType y) { return y.Other; } } }\n",
            )],
        ),
    ]
}

#[test]
fn a_reference_that_bound_to_nothing_is_counted() {
    let cases = unbindable();
    assert!(!cases.is_empty(), "this build carries no Tier 1 language");

    for (language, files) in cases {
        let ir = analyze(language, &files);
        assert!(
            unbound(&ir) > 0,
            "{language} bound nothing for `MissingType` and reported 0 unbound \
             references — the failure is invisible, which is the whole defect"
        );
        assert!(
            ir.files.iter().all(|f| f.references_counted),
            "{language} produced a file that does not claim to count references"
        );
    }
}

#[test]
fn a_repository_where_everything_binds_reports_none() {
    // The other half. A count that only ever rises is not a measurement, and
    // this fails if `unbound_references` were wired to something like "every
    // reference" or "every dropped relationship".
    #[cfg(feature = "csharp")]
    {
        let ir = analyze(
            "all-bound",
            &[
                (
                    "a/Service.cs",
                    "namespace A { public class Service { public void Run() {} } }",
                ),
                (
                    "a/Caller.cs",
                    "namespace A { public class Caller { public void Go() { Service s = new Service(); s.Run(); } } }",
                ),
            ],
        );
        assert_eq!(
            unbound(&ir),
            0,
            "a repository whose references all resolve reported unbound ones: {:#?}",
            ir.files
                .iter()
                .map(|f| (&f.file, f.unbound_references))
                .collect::<Vec<_>>()
        );
        assert!(edges(&ir) > 0, "nothing was recorded at all");
    }
}

#[test]
fn counting_does_not_change_what_is_recorded() {
    // The measurement must observe the analysis, not alter it. An earlier
    // attempt at this had the adapters stop discarding unresolved references so
    // one central counter could see them — which changed seven adapters' output
    // and broke eleven tests asserting real properties, among them "a builtin
    // call records no edge". The count is now taken where each adapter already
    // makes its decision, so nothing about the graph moves.
    //
    // Asserted rather than trusted: every edge here has a real target, and the
    // unbound references alongside them added none.
    let cases = unbindable();
    for (language, files) in cases {
        let ir = analyze(language, &files);
        for file in &ir.files {
            for rel in &file.relationships {
                assert!(
                    !openinvar_core::symbol_id::is_placeholder(&rel.to),
                    "{language} recorded an edge to a placeholder while counting: {} -> {}",
                    rel.from,
                    rel.to
                );
            }
        }
    }
}

#[test]
fn the_count_is_the_same_on_every_run() {
    #[cfg(feature = "java")]
    {
        let files = vec![(
            "A.java",
            "import com.untyped.Unknowable;\n\
             public class A { public int render(Unknowable x, MissingType y) { return x.field + y.other; } }\n",
        )];
        let first = unbound(&analyze("determinism-0", &files));
        for run in 1..4 {
            assert_eq!(
                unbound(&analyze(&format!("determinism-{run}"), &files)),
                first,
                "run {run} counted a different number of unbound references"
            );
        }
    }
}
