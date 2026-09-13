// Exercises the typescript module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "typescript")]

use std::path::PathBuf;

use openinvar_core::ir::Resolution;
use openinvar_core::graph::InvarGraph;
use openinvar_core::query::{blast_radius, RelationshipKindMask};
use openinvar_core::scan::{walk_source_files_with_config, ScanConfig};
use openinvar_lang::lang::typescript::analyze_files;
use openinvar_lang::lang::typescript::language::is_supported_source_file;

#[allow(dead_code)]
fn analyze_repo(
    root: &std::path::Path,
) -> Result<openinvar_core::ir::RepoIR, openinvar_lang::lang::typescript::AdapterTsError> {
    let files = walk_source_files_with_config(
        root,
        &ScanConfig::default_enabled(),
        is_supported_source_file,
    )
    .expect("scan should succeed for DI fixture");
    analyze_files(root, &files)
}

fn build_graph(repo_ir: &openinvar_core::ir::RepoIR) -> InvarGraph {
    let mut graph = InvarGraph::new();
    for file in &repo_ir.files {
        for symbol in &file.symbols {
            graph.add_symbol(symbol.clone());
        }
    }
    for file in &repo_ir.files {
        for relationship in &file.relationships {
            graph.add_relationship(relationship);
        }
    }
    graph
}

#[test]
fn test_di_constructor_injection_property_access_tracked() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/adapter-ts/di_injection/src");
    let repo_ir = analyze_repo(&root).expect("analysis should succeed for DI fixture");

    let payment = repo_ir
        .files
        .iter()
        .find(|f| f.file.ends_with("payment.service.ts"))
        .expect("payment service file should exist in analyzed output");

    let user_repo_rel = payment
        .relationships
        .iter()
        .find(|r| r.to.ends_with("user.repository.ts::UserRepository::class"))
        .expect("payment service should import UserRepository");
    assert!(
        user_repo_rel
            .properties_accessed
            .iter()
            .any(|p| p == "findById"),
        "UserRepository relationship should include findById property access"
    );

    let email_service_rel = payment
        .relationships
        .iter()
        .find(|r| r.to.ends_with("email.service.ts::EmailService::class"))
        .expect("payment service should import EmailService");
    assert!(
        email_service_rel
            .properties_accessed
            .iter()
            .any(|p| p == "sendReceipt"),
        "EmailService relationship should include sendReceipt property access"
    );
}

#[test]
fn test_blast_radius_of_user_repository_includes_payment_service() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/adapter-ts/di_injection/src");
    let repo_ir = analyze_repo(&root).expect("analysis should succeed for DI fixture");
    let graph = build_graph(&repo_ir);

    let edges = blast_radius(
        &graph,
        "UserRepository",
        None,
        Some(2),
        RelationshipKindMask::all(),
        Resolution::Structural,
    )
    .expect("blast radius query should resolve UserRepository");
    assert!(
        edges.iter().any(|edge| edge.from.contains("PaymentService")),
        "PaymentService should appear in UserRepository blast radius from constructor injection usage"
    );
}
