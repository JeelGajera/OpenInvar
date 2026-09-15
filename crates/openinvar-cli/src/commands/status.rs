use std::collections::BTreeMap;

use openinvar_core::coverage;
use openinvar_core::ir::SymbolKind;

use crate::output;

pub fn run(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let root = super::normalize_path(
        &std::fs::canonicalize(path).map_err(|e| format!("cannot access '{}': {}", path, e))?,
    );
    let graph = super::analyze::load_graph(&root)?;

    output::banner("status");
    output::info(&format!(
        "Graph: {}",
        output::file_path(&root.join(".openinvar/db").display().to_string()),
    ));
    output::blank();

    // ── core stats ───────────────────────────────────────────
    output::section("Graph Overview");
    output::stat_highlight("Symbols", &graph.symbols.len().to_string());
    output::stat_highlight("Relationships", &graph.graph.edge_count().to_string());
    output::stat_highlight("Files indexed", &graph.file_index.len().to_string());
    output::stat_highlight(
        "Aliases",
        &format!(
            "{} (across {} symbol(s))",
            graph.alias_count(),
            graph.aliased_symbol_count()
        ),
    );

    // ── language support ─────────────────────────────────────
    //
    // Tier is reported next to the language because a Tier 2 result and a
    // Tier 1 result look identical in the output and mean very different
    // things. "No usages found" from a structural language is a statement
    // about one file, not about the repository.
    output::section("Language Support");
    let mut any_structural = false;
    for support in openinvar_lang::supported_languages() {
        if support.tier == openinvar_lang::Tier::Structural {
            any_structural = true;
        }
        output::stat(
            &format!("  {}", support.name),
            &format!("tier {} ({})", support.tier.number(), support.tier.as_str()),
        );
    }
    if any_structural {
        output::dim_line("  Tier 2 sees symbols and intra-file references only — no imports,");
        output::dim_line("  aliases or declared types. Gates must not draw conclusions from it.");
    }

    // ── resolution coverage ──────────────────────────────────
    //
    // The number that decides how much of this graph a gate may act on. An
    // enforcing tool that says "nothing broke" is making a claim about the
    // edges it resolved, not about the repository, and the difference between
    // 98% and 40% is the difference between a gate and a coin flip.
    //
    // Reported per language because coverage is rarely uniform: one badly
    // resolved language drags the total down and an overall figure hides which.
    output::section("Resolution Coverage");

    let overall = coverage::overall(&graph);
    match overall.percent() {
        Some(percent) => output::stat_highlight(
            "Resolved",
            &format!(
                "{percent:.1}% ({} of {} edge(s))",
                overall.resolved,
                overall.total()
            ),
        ),
        None => output::stat("Resolved", "no relationships in this graph"),
    }

    let mut per_language: BTreeMap<&'static str, coverage::Coverage> = BTreeMap::new();
    let mut unattributed = coverage::Coverage::default();
    for (file, file_coverage) in coverage::by_file(&graph) {
        match openinvar_lang::for_path(&file) {
            Some(spec) => {
                let entry = per_language.entry(spec.name()).or_default();
                entry.resolved += file_coverage.resolved;
                entry.structural += file_coverage.structural;
            }
            None => {
                unattributed.resolved += file_coverage.resolved;
                unattributed.structural += file_coverage.structural;
            }
        }
    }

    for (language, language_coverage) in &per_language {
        let detail = match language_coverage.percent() {
            Some(percent) => format!(
                "{percent:.1}% of {} edge(s)",
                language_coverage.total()
            ),
            None => "no relationships".to_string(),
        };
        output::stat(&format!("  {language}"), &detail);
    }
    if unattributed.total() > 0 {
        // Edges whose file this build carries no language for. Named rather
        // than folded into the total, so the total stays a statement about
        // languages that were actually analysed.
        output::stat(
            "  (no language)",
            &format!("{} edge(s)", unattributed.total()),
        );
    }

    // ── reference-level coverage ─────────────────────────────
    //
    // The figure above is the share of *recorded edges* that resolved, and an
    // edge exists only once something bound it. A reference the analysis could
    // not place leaves no edge, so it never enters that denominator — which
    // means failing to bind more references raises the percentage. On this
    // repository the two figures are 99.8% and 69.6%, and only the second is a
    // statement about the source.
    //
    // Reported second and named differently, because they answer different
    // questions: "can a gate act on what is here" and "how much of the source
    // is here at all".
    let unbound: u32 = graph.unbound_references.iter().map(|e| *e.value()).sum();
    let counted_files: usize = graph.unbound_references.len();
    if counted_files > 0 {
        let bound = overall.resolved as u32;
        let attempted = bound + unbound;
        if attempted > 0 {
            output::stat_highlight(
                "References bound",
                &format!(
                    "{:.1}% ({bound} of {attempted} reference(s))",
                    bound as f64 * 100.0 / attempted as f64
                ),
            );
            output::dim_line(&format!(
                "  {unbound} reference(s) named in source bound to nothing in the graph."
            ));
            output::dim_line(
                "  Includes code outside the repository — a type from an unanalysed",
            );
            output::dim_line(
                "  package is indistinguishable from one that should have been found.",
            );
        }
    }

    if overall.structural > 0 {
        output::dim_line(&format!(
            "  {} edge(s) are structural: matched by name within one file.",
            overall.structural
        ));
        output::dim_line("  A gate must not act on them. Known blind regions, all structural or");
        output::dim_line("  absent: Tier 2 languages, C++ template instantiations, Rust macro");
        output::dim_line("  bodies, cross-language imports, and chained access past the first");
        output::dim_line("  receiver.");
    }

    // ── symbol breakdown ─────────────────────────────────────
    let mut kind_counts: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    for entry in graph.symbols.iter() {
        *kind_counts
            .entry(format_kind(&entry.value().kind).to_string())
            .or_insert(0) += 1;
    }

    if !kind_counts.is_empty() {
        output::section("Symbol Kinds");
        for (kind, count) in &kind_counts {
            output::stat(&format!("  {kind}"), &count.to_string());
        }
    }

    // ── files by symbol count ────────────────────────────────
    let mut file_counts: Vec<(String, usize)> = graph
        .file_index
        .iter()
        .map(|entry| (entry.key().clone(), entry.value().len()))
        .collect();
    file_counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    if !file_counts.is_empty() {
        output::section("Files by Symbol Count");
        for (file, count) in file_counts.iter().take(10) {
            output::stat(
                &format!("  {}", output::file_path(file)),
                &format!("{count} symbol(s)"),
            );
        }
        if file_counts.len() > 10 {
            output::dim_line(&format!("  … and {} more file(s)", file_counts.len() - 10));
        }
    }

    // ── alias chains detail ──────────────────────────────────
    if !graph.alias_chains.is_empty() {
        output::section("Alias Chains");
        for entry in graph.alias_chains.iter() {
            let canonical_id = entry.key();
            let canonical_name = graph
                .symbols
                .get(canonical_id.as_str())
                .map(|s| s.name.clone())
                .unwrap_or_else(|| canonical_id.clone());

            let aliases: Vec<String> = entry
                .value()
                .iter()
                .map(|a| output::alias_tag(&a.alias_name))
                .collect();
            println!(
                "  {} → {}",
                output::symbol_name(&canonical_name),
                aliases.join(", "),
            );
        }
    }

    output::blank();
    Ok(())
}

fn format_kind(kind: &SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Class => "class",
        SymbolKind::Interface => "interface",
        SymbolKind::TypeAlias => "type",
        SymbolKind::Function => "function",
        SymbolKind::Method => "method",
        SymbolKind::Property => "property",
        SymbolKind::Variable => "variable",
        SymbolKind::Module => "module",
        SymbolKind::Enum => "enum",
        SymbolKind::EnumVariant => "variant",
        SymbolKind::ExternalPackage => "external",
    }
}
