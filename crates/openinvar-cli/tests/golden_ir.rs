//! Golden snapshots of the analysis over the whole fixture corpus.
//!
//! OpenInvar's per-adapter tests each assert one property of one language. That
//! catches a regression someone thought to look for, and misses the kind that
//! matters most here: a refactor that quietly changes the graph everywhere.
//! The 1.0.0 plan opens with a large behaviour-preserving move of every
//! adapter into one crate, and "behaviour-preserving" is a claim, not a fact,
//! until the whole corpus is compared before and after.
//!
//! So these tests record the complete analysis of every fixture project and
//! diff it. A change to the goldens is not a failure — it is the diff you were
//! asked to review. What it must never be is invisible.
//!
//! Regenerate after an intended change, then read the diff before committing:
//!
//! ```bash
//! UPDATE_GOLDEN=1 cargo test -p openinvar-cli --test golden_ir
//! git diff crates/openinvar-cli/tests/golden
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;

/// The fixture projects under `fixtures/`, each analyzed as its own repository.
///
/// Listed rather than discovered: a fixture that silently stops being covered
/// because a directory was renamed is exactly the gap this suite exists to
/// close, and an empty corpus would otherwise pass.
const CORPUS: &[&str] = &[
    "adapter-c",
    "adapter-go",
    "adapter-py",
    "adapter-rust",
    "adapter-ts",
    "alias-import-bug",
    "polyglot",
    "regression",
    "scan",
    "test-edges",
];

/// Fixtures for Tier 2 languages, which are deliberately not in the corpus.
///
/// Their analysis depends on an optional feature, so the same fixture produces
/// a populated report in one build and an empty one in another. A golden that
/// depends on how the binary was configured is not a golden. Tier 2 behaviour
/// is covered directly in `openinvar-lang/tests/<language>_structural.rs`, where
/// the feature is a precondition rather than a variable.
const STRUCTURAL_FIXTURES: &[&str] = &["adapter-csharp", "adapter-java", "adapter-ruby"];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.json"))
}

/// Copy a fixture into a scratch directory.
///
/// `analyze` persists a graph beside the sources it reads, so running it over
/// `fixtures/` directly would write into the source tree and let two
/// concurrent tests collide over one database.
fn scratch_copy(name: &str) -> PathBuf {
    let dest = std::env::temp_dir().join(format!("openinvar-golden-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dest);
    copy_tree(&repo_root().join("fixtures").join(name), &dest);
    dest
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create scratch dir");
    for entry in std::fs::read_dir(from).expect("read fixture dir") {
        let entry = entry.expect("read fixture entry");
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy fixture file");
        }
    }
}

/// Analyze a fixture and return its report with the one machine-specific
/// value replaced.
///
/// `root` is the absolute path of whatever directory the fixture was copied
/// into, which differs per machine and per run. Every other path in the
/// document is relative to it, so blanking it is all that stands between this
/// report and a portable one.
fn analyze_normalized(name: &str) -> String {
    let root = scratch_copy(name);
    let output = Command::new(env!("CARGO_BIN_EXE_openinvar"))
        .arg("analyze")
        .arg(&root)
        .arg("--json")
        .output()
        .expect("run openinvar analyze --json");

    assert!(
        output.status.success(),
        "analyze failed on fixture '{name}': {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let raw = String::from_utf8(output.stdout).expect("stdout is valid UTF-8");
    let mut doc: serde_json::Value = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("fixture '{name}' did not emit valid JSON: {e}"));
    doc["root"] = serde_json::Value::String("<root>".to_string());

    let _ = std::fs::remove_dir_all(&root);
    serde_json::to_string_pretty(&doc).expect("re-serialize report") + "\n"
}

fn updating() -> bool {
    std::env::var("UPDATE_GOLDEN").is_ok_and(|v| !v.is_empty() && v != "0")
}

fn check_fixture(name: &str) {
    let actual = analyze_normalized(name);
    let path = golden_path(name);

    if updating() {
        std::fs::create_dir_all(path.parent().unwrap()).expect("create golden dir");
        std::fs::write(&path, &actual).expect("write golden");
        return;
    }

    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "no golden recorded for fixture '{name}'.\n\
             Record it with: UPDATE_GOLDEN=1 cargo test -p openinvar-cli --test golden_ir"
        )
    });

    if actual != expected {
        // Lead with what moved in the graph, not with the first line that
        // differs textually. Adding one symbol shifts a trailing comma several
        // lines before anything meaningful, so a purely positional report
        // names a punctuation change when the finding is "three more symbols".
        let summary = stats_delta(&expected, &actual);
        let (line, exp, act) = first_difference(&expected, &actual);
        panic!(
            "analysis of fixture '{name}' changed\n{summary}\n\
             first textual difference at line {line}\n  \
             golden: {exp}\n  \
             actual: {act}\n\n\
             If this change is intended, regenerate and review the diff:\n  \
             UPDATE_GOLDEN=1 cargo test -p openinvar-cli --test golden_ir\n  \
             git diff crates/openinvar-cli/tests/golden"
        );
    }
}

/// Describe how the headline counts moved between two reports.
///
/// This is what a reviewer of a behaviour-preserving refactor actually needs:
/// whether the graph changed shape, and in which direction.
fn stats_delta(expected: &str, actual: &str) -> String {
    let (Ok(before), Ok(actual)) = (
        serde_json::from_str::<serde_json::Value>(expected),
        serde_json::from_str::<serde_json::Value>(actual),
    ) else {
        return "  (could not parse one of the reports to compare counts)".to_string();
    };

    let mut lines = Vec::new();
    for key in [
        "symbols",
        "relationships",
        "files_indexed",
        "alias_chains",
        "aliases",
        "diagnostics",
    ] {
        let b = before["stats"][key].as_i64().unwrap_or(0);
        let a = actual["stats"][key].as_i64().unwrap_or(0);
        if b != a {
            let delta = a - b;
            lines.push(format!("  {key}: {b} -> {a} ({delta:+})"));
        }
    }

    if lines.is_empty() {
        "  headline counts unchanged; the difference is in edge or symbol detail".to_string()
    } else {
        lines.join("\n")
    }
}

fn first_difference(expected: &str, actual: &str) -> (usize, String, String) {
    for (i, (e, a)) in expected.lines().zip(actual.lines()).enumerate() {
        if e != a {
            return (i + 1, e.trim().to_string(), a.trim().to_string());
        }
    }
    let common = expected.lines().count().min(actual.lines().count());
    (
        common + 1,
        format!("<{} lines total>", expected.lines().count()),
        format!("<{} lines total>", actual.lines().count()),
    )
}

macro_rules! golden_test {
    ($test_name:ident, $fixture:expr) => {
        #[test]
        fn $test_name() {
            check_fixture($fixture);
        }
    };
}

golden_test!(golden_adapter_c, "adapter-c");
golden_test!(golden_adapter_go, "adapter-go");
golden_test!(golden_adapter_py, "adapter-py");
golden_test!(golden_adapter_rust, "adapter-rust");
golden_test!(golden_adapter_ts, "adapter-ts");
golden_test!(golden_alias_import_bug, "alias-import-bug");
golden_test!(golden_polyglot, "polyglot");
golden_test!(golden_regression, "regression");
golden_test!(golden_scan, "scan");
golden_test!(golden_test_edges, "test-edges");

#[test]
fn every_fixture_project_is_covered() {
    // A new fixture with no golden is a hole in the corpus that no other test
    // reports, because a test that does not exist cannot fail.
    let fixtures = repo_root().join("fixtures");
    let mut on_disk: Vec<String> = std::fs::read_dir(&fixtures)
        .expect("read fixtures dir")
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with('.'))
        .filter(|n| !STRUCTURAL_FIXTURES.contains(&n.as_str()))
        .collect();
    on_disk.sort();

    let mut covered: Vec<String> = CORPUS.iter().map(|s| s.to_string()).collect();
    covered.sort();

    assert_eq!(
        on_disk, covered,
        "fixtures/ and the golden corpus have diverged — \
         add the new fixture to CORPUS and record its golden"
    );
}

#[test]
fn the_corpus_is_reproducible() {
    // The property every golden above silently depends on. If analysis were
    // not reproducible, a golden would record one run's output and fail on the
    // next, and the whole suite would read as flaky rather than as the
    // determinism regression it would actually be.
    for name in CORPUS {
        assert_eq!(
            analyze_normalized(name),
            analyze_normalized(name),
            "fixture '{name}' analyzed differently on two consecutive runs"
        );
    }
}
