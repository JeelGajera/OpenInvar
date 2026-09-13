//! The detectors against real analysed code.
//!
//! The unit fixtures build graphs by hand, which proves the rules but not that
//! they survive contact with a real adapter. This runs the whole pipeline —
//! source on disk, parsed and resolved — over an ordinary change and asserts
//! that nothing fires.
//!
//! That negative is the number that decides whether anyone leaves the gate
//! switched on. A detector which is correct on a synthetic graph and noisy on
//! real code is a detector that gets disabled in week two.

use std::path::{Path, PathBuf};

use openinvar_core::audit::detectors;
use openinvar_core::audit::{AuditContext, Finding};
use openinvar_core::delta;
use openinvar_core::graph::InvarGraph;
use openinvar_core::incremental::replace_file_ir;

fn scratch(name: &str) -> PathBuf {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/test-edges");
    let dest =
        std::env::temp_dir().join(format!("openinvar-audit-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dest);
    copy_tree(&src, &dest);
    dest
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create dir");
    for entry in std::fs::read_dir(from).expect("read dir") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy");
        }
    }
}

/// Analyse a directory with the real pipeline and build a graph from it.
fn analyze(root: &Path) -> (InvarGraph, Vec<String>) {
    let mut files = Vec::new();
    collect(root, &mut files);
    files.sort();
    let repo = openinvar_lang::analyze_files(root, &files).expect("analysis succeeds");

    let mut graph = InvarGraph::new();
    // Two passes: an edge whose target is not yet a node is dropped, so a
    // single pass loses every forward reference.
    for file_ir in &repo.files {
        let mut symbols_only = file_ir.clone();
        symbols_only.relationships = vec![];
        replace_file_ir(&mut graph, &symbols_only);
    }
    for file_ir in &repo.files {
        for relationship in &file_ir.relationships {
            graph.add_relationship(relationship);
        }
    }
    let names = repo.files.iter().map(|f| f.file.clone()).collect();
    (graph, names)
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read dir") {
        let entry = entry.expect("entry");
        if entry.path().is_dir() {
            collect(&entry.path(), out);
        } else {
            out.push(entry.path());
        }
    }
}

fn audit(before: &InvarGraph, after: &InvarGraph, files: Vec<String>) -> Vec<Finding> {
    let computed = delta::compute(before, after);
    let in_scope = files.into_iter().collect();
    let context = AuditContext {
        before,
        after,
        delta: &computed,
        tier_one_files: &in_scope,
    };
    let mut findings = Vec::new();
    for detector in detectors::all() {
        for finding in detector.detect(&context) {
            if context.in_scope(&finding.file) {
                findings.push(finding);
            }
        }
    }
    findings
}

#[test]
fn no_detector_fires_on_an_unchanged_repository() {
    let root = scratch("unchanged");
    let (before, files) = analyze(&root);
    let (after, _) = analyze(&root);

    let findings = audit(&before, &after, files);
    assert!(findings.is_empty(), "fired on no change at all: {findings:#?}");
}

#[test]
fn no_detector_fires_when_a_function_and_its_test_change_together() {
    // The single most common shape of honest work. If anything fires here the
    // gate is unusable, whatever it scores on a synthetic fixture.
    let root = scratch("ordinary");
    let (before, _) = analyze(&root);

    let payload = root.join("src/payload.ts");
    let source = std::fs::read_to_string(&payload).expect("read");
    std::fs::write(
        &payload,
        source.replace(
            "export function encode(payload: Payload): string {\n  return payload.userId;\n}",
            "export function encode(payload: Payload, upper: boolean): string {\n  \
             return upper ? payload.userId : payload.email;\n}",
        ),
    )
    .expect("write");

    let test = root.join("src/payload.test.ts");
    let test_source = std::fs::read_to_string(&test).expect("read");
    std::fs::write(
        &test,
        test_source.replace("return encode(p);", "return encode(p, true);"),
    )
    .expect("write");

    let (after, files) = analyze(&root);
    let findings = audit(&before, &after, files);

    assert!(
        findings.is_empty(),
        "ordinary work was reported as reward-hacking: {findings:#?}"
    );
}

#[test]
fn no_detector_fires_when_new_code_is_added_with_its_test() {
    let root = scratch("addition");
    let (before, _) = analyze(&root);

    let payload = root.join("src/payload.ts");
    let source = std::fs::read_to_string(&payload).expect("read");
    std::fs::write(
        &payload,
        format!("{source}\nexport function decode(raw: string): string {{\n  return raw;\n}}\n"),
    )
    .expect("write");

    let (after, files) = analyze(&root);
    let findings = audit(&before, &after, files);

    assert!(findings.is_empty(), "adding code was reported: {findings:#?}");
}

#[test]
fn deleting_a_function_whose_caller_survives_is_caught_on_real_code() {
    // The one positive here, so the suite cannot pass by detecting nothing.
    let root = scratch("erosion");
    let (before, _) = analyze(&root);

    let payload = root.join("src/payload.ts");
    let source = std::fs::read_to_string(&payload).expect("read");
    std::fs::write(
        &payload,
        source.replace(
            "export function encode(payload: Payload): string {\n  return payload.userId;\n}",
            "",
        ),
    )
    .expect("write");

    let (after, files) = analyze(&root);
    let findings = audit(&before, &after, files);

    assert!(
        findings.iter().any(|f| f.detector == "contract-erosion"),
        "removing a called function was not caught: {findings:#?}"
    );
}
