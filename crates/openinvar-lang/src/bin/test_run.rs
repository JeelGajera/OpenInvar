use openinvar_core::scan::{walk_source_files_with_config, ScanConfig};
use openinvar_lang::lang::typescript::analyze_files;
use openinvar_lang::lang::typescript::language::is_supported_source_file;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: test_run <repo_path>");
        std::process::exit(1);
    }
    let root = PathBuf::from(&args[1]);
    println!("Scanning repository: {:?}", root);

    let files = walk_source_files_with_config(
        &root,
        &ScanConfig::default_enabled(),
        is_supported_source_file,
    )
    .unwrap();
    println!("Found {} files to analyze", files.len());

    let repo_ir = analyze_files(&root, &files).expect("Analysis failed completely");

    println!("--- Analysis Complete ---");
    let external_relationships_count = repo_ir
        .files
        .iter()
        .flat_map(|f| f.relationships.iter())
        .filter(|r| r.to.starts_with("ext::"))
        .count();
    println!(
        "Total External Dependencies Detected: {}",
        external_relationships_count
    );

    let mut parse_errors = 0;
    let mut resolution_warnings = 0;
    for file in &repo_ir.files {
        for diag in &file.diagnostics {
            match diag.category {
                openinvar_core::ir::DiagnosticCategory::Parse => parse_errors += 1,
                openinvar_core::ir::DiagnosticCategory::Resolution => {
                    resolution_warnings += 1;
                    println!(
                        "Warning in {}: {}",
                        diag.file.as_deref().unwrap_or("?"),
                        diag.message
                    );
                }
                _ => {}
            }
        }
    }

    println!("Parse Errors: {}", parse_errors);
    println!(
        "Resolution Warnings (cycles, unresolvable paths, etc): {}",
        resolution_warnings
    );

    let total_symbols: usize = repo_ir.files.iter().map(|f| f.symbols.len()).sum();
    println!("Total Symbols Extracted: {}", total_symbols);
}
