//! The `analyze --json` contract.
//!
//! These drive the real binary rather than the library, because two of the
//! three properties under test are properties of the process: that stdout
//! carries a parseable document and nothing else, and that two runs of that
//! process agree byte for byte. Neither is observable from inside a library
//! call.

use std::path::{Path, PathBuf};
use std::process::Command;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_openinvar")
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name)
}

/// Copy a fixture into a scratch directory.
///
/// `analyze` persists a graph next to the sources it reads, so running it
/// against `fixtures/` directly would write into the source tree and let two
/// concurrently-running tests collide over the same database.
fn scratch_copy(fixture_name: &str, test_name: &str) -> PathBuf {
    let dest = std::env::temp_dir().join(format!(
        "openinvar-{}-{}-{}",
        fixture_name.replace('/', "-"),
        test_name,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dest);
    copy_tree(&fixture(fixture_name), &dest);
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

fn analyze_json(root: &Path) -> String {
    let output = Command::new(binary())
        .arg("analyze")
        .arg(root)
        .arg("--json")
        .output()
        .expect("run openinvar analyze --json");

    assert!(
        output.status.success(),
        "analyze --json failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("stdout is valid UTF-8")
}

#[test]
fn json_output_carries_a_versioned_envelope() {
    let root = scratch_copy("polyglot", "envelope");
    let raw = analyze_json(&root);
    let doc: serde_json::Value = serde_json::from_str(&raw).expect("stdout parses as JSON");

    assert_eq!(doc["schema_version"], 1);
    assert!(doc["root"].is_string());
    assert!(doc["files"].is_array());

    // The headline counts a consumer reads before deciding whether to look
    // closer. Absent fields here would surface as a null at the point of use,
    // long after the analysis that should have reported them.
    for key in [
        "symbols",
        "relationships",
        "files_indexed",
        "alias_chains",
        "diagnostics",
    ] {
        assert!(
            doc["stats"][key].is_u64(),
            "stats.{key} missing from the report"
        );
    }

    let files = doc["files"].as_array().unwrap();
    assert_eq!(files.len(), 4, "polyglot fixture has four source files");
    for file in files {
        assert!(file["file"].is_string());
        assert!(file["symbols"].is_array());
        assert!(file["relationships"].is_array());
    }

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn json_output_is_the_only_thing_on_stdout() {
    // Progress lines and a JSON document cannot share a stream. A single
    // stray banner turns the report into something no parser accepts, and the
    // failure surfaces in the consumer rather than here.
    let root = scratch_copy("polyglot", "stdout");
    let raw = analyze_json(&root);

    assert!(
        raw.trim_start().starts_with('{'),
        "stdout does not begin with the document: {:?}",
        &raw[..raw.len().min(120)]
    );
    serde_json::from_str::<serde_json::Value>(&raw).expect("stdout parses as JSON");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn two_runs_over_identical_input_agree_byte_for_byte() {
    // Determinism is the property the whole product rests on, and it holds
    // only if it survives all the way out to the emitted bytes. It did not:
    // `RepoIR.language_stats` was a `HashMap`, whose iteration order is
    // seeded per process, so the same analysis serialized differently on
    // every run.
    let root = scratch_copy("polyglot", "determinism");

    let first = analyze_json(&root);
    let second = analyze_json(&root);

    assert_eq!(
        first, second,
        "two analyses of identical input produced different JSON"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn language_stats_are_emitted_in_a_stable_order() {
    // The regression above, pinned directly: whatever languages a repository
    // contains, they serialize sorted rather than in hash order.
    let root = scratch_copy("polyglot", "langstats");
    let raw = analyze_json(&root);
    let doc: serde_json::Value = serde_json::from_str(&raw).unwrap();

    let emitted: Vec<&str> = doc["language_stats"]
        .as_object()
        .expect("language_stats is an object")
        .keys()
        .map(String::as_str)
        .collect();

    let mut sorted = emitted.clone();
    sorted.sort_unstable();
    assert_eq!(emitted, sorted, "language_stats are not in sorted order");
    assert!(emitted.contains(&"TypeScript"));
    assert!(emitted.contains(&"Rust"));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_empty_scan_still_produces_a_document() {
    // "Nothing matched" is a result. A consumer that always parses stdout
    // should not have to special-case the one outcome it most needs to detect.
    let root = std::env::temp_dir().join(format!("openinvar-empty-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create empty dir");

    let raw = analyze_json(&root);
    let doc: serde_json::Value = serde_json::from_str(&raw).expect("empty scan emits JSON");

    assert_eq!(doc["schema_version"], 1);
    assert_eq!(doc["stats"]["symbols"], 0);
    assert_eq!(doc["files"].as_array().unwrap().len(), 0);

    let _ = std::fs::remove_dir_all(&root);
}
