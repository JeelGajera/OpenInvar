use openinvar_core::ir::Resolution;
use std::collections::BTreeMap;

use openinvar_core::graph::InvarGraph;
use openinvar_core::ir::SymbolKind;
use openinvar_core::query::{self, QueryEdge, RelationshipKindMask};

use crate::output;

/// Resolve the `--kind` values a user passed into a traversal mask.
///
/// An unrecognised name is rejected rather than ignored. Quietly dropping it
/// would answer a narrower question than the one asked while presenting the
/// result as the answer to theirs.
pub fn mask_from_args(kinds: &[String]) -> Result<RelationshipKindMask, String> {
    if kinds.is_empty() {
        return Ok(RelationshipKindMask::all());
    }
    let mut mask = RelationshipKindMask::none();
    for name in kinds {
        let kind = query::parse_kind(name).ok_or_else(|| {
            let known: Vec<&str> = query::ALL_KINDS.iter().map(query::kind_name).collect();
            format!(
                "unknown --kind '{name}'\n  known kinds: {}",
                known.join(", ")
            )
        })?;
        mask = mask.with(kind);
    }
    Ok(mask)
}

/// Warn when a filter cannot match anything in *this* graph.
///
/// Returning a silent empty result for `--kind calls` would read as "nothing
/// calls this", which is the one answer OpenInvar must never give without
/// warrant. The check is against the analyzed graph rather than a list of
/// unimplemented kinds, so it stays true as languages gain call edges at
/// different times: a Python-only repository is told that no call edge exists
/// in it, which is the fact the user needs, without asserting anything about
/// what OpenInvar supports elsewhere.
fn warn_about_absent_kinds(mask: &RelationshipKindMask, graph: &InvarGraph) {
    if mask.is_all() {
        return;
    }
    let requested = mask.kinds();
    let present = query::kinds_present(graph);
    let absent: Vec<&str> = requested
        .iter()
        .filter(|k| !present.contains(k))
        .map(query::kind_name)
        .collect();
    if absent.is_empty() {
        return;
    }

    if absent.len() == requested.len() {
        output::warning(&format!(
            "no {} edge exists in this graph",
            absent.join(" or ")
        ));
        output::dim_line("  This result is empty for that reason alone.");
        output::dim_line("  Do not read it as evidence that nothing depends on the symbol.");
    } else {
        output::warning(&format!(
            "{} matched nothing: no such edge in this graph",
            absent.join(", ")
        ));
    }
    output::blank();
}

/// Show the active filter, so a short result is never mistaken for a small
/// blast radius.
fn report_filter(mask: &RelationshipKindMask) {
    if mask.is_all() {
        return;
    }
    let names: Vec<&str> = mask.kinds().iter().map(query::kind_name).collect();
    output::stat("Kinds", &names.join(", "));
}

// ── blast-radius ─────────────────────────────────────────────

/// Parse `--min-confidence` into a resolution floor.
///
/// The flag is named for the concept the gate commands use rather than for the
/// enum, so the vocabulary stays stable if the ladder grows a rung between the
/// two it has today.
fn resolution_floor(name: &str) -> Result<Resolution, Box<dyn std::error::Error>> {
    match name {
        "structural" => Ok(Resolution::Structural),
        "resolved" => Ok(Resolution::Resolved),
        other => Err(format!(
            "unknown --min-confidence '{other}'. Expected 'structural' or 'resolved'."
        )
        .into()),
    }
}

pub fn run_blast_radius(
    symbol: &str,
    file: Option<&str>,
    depth: usize,
    path: &str,
    kinds: &[String],
    min_confidence: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mask = mask_from_args(kinds)?;
    let floor = resolution_floor(min_confidence)?;
    let root = super::normalize_path(
        &std::fs::canonicalize(path).map_err(|e| format!("cannot access '{}': {}", path, e))?,
    );
    let graph = super::analyze::load_graph(&root)?;

    output::banner("blast-radius");

    // look up the canonical symbol to display metadata
    let edges = query::blast_radius(&graph, symbol, file, Some(depth), mask, floor)
        .map_err(|e| format_query_error(e, symbol))?;

    print_symbol_header(&graph, symbol, file);
    output::stat("Depth", &depth.to_string());
    report_filter(&mask);
    output::blank();
    warn_about_absent_kinds(&mask, &graph);

    if edges.is_empty() {
        // Never claim safety on a filtered query: the caller narrowed the
        // question, and an empty answer to a narrow question says nothing
        // about the rest of the graph.
        if mask.is_all() {
            // "Safe to modify" is a claim about the whole repository. It holds
            // only if everything in the graph was resolved well enough to make
            // it: a structural region cannot see across files, so a reference
            // living there would never have reached the graph, and the
            // emptiness is partly an artefact of how much was resolved.
            let structural = query::structural_files(&graph);
            if structural.is_empty() {
                output::success("No dependents found — safe to modify.");
            } else {
                output::success("No dependents found.");
                output::blank();
                output::warning("not safe to conclude: this graph has structural regions");
                output::dim_line(&format!(
                    "  {} file(s) were analyzed within-file only, so a reference",
                    structural.len()
                ));
                output::dim_line("  from one of them to this symbol would not appear here.");
                for file in structural.iter().take(3) {
                    output::dim_line(&format!("    {file}"));
                }
                if structural.len() > 3 {
                    output::dim_line(&format!("    … and {} more", structural.len() - 3));
                }
            }
        } else {
            output::success("No dependents found under the selected kinds.");
            output::dim_line("  Other relationship kinds were not searched.");
        }
        output::blank();
        return Ok(());
    }

    let (direct, aliased) = openinvar_core::query::partition_by_alias(&graph, &edges);
    let total = edges.len();
    output::section(&format!("{total} dependent(s) found"));

    if !direct.is_empty() {
        println!("  {}", output::bold(&format!("DIRECT ({})", direct.len())));
        output::blank();
        for (i, edge) in direct.iter().enumerate() {
            print_edge(i + 1, edge, &graph);
        }
    }

    if !aliased.is_empty() {
        println!(
            "  {} {}",
            output::bold_yellow(&format!("ALIASED ({})", aliased.len())),
            output::yellow("— HIGH RISK: different name in source")
        );
        output::blank();
        let offset = direct.len();
        for (i, edge) in aliased.iter().enumerate() {
            print_edge(offset + i + 1, edge, &graph);
        }
    }

    // property summary
    let props = collect_property_summary(&edges);
    if !props.is_empty() {
        output::section("Properties at Risk");
        for (prop, count) in &props {
            let aliased_note = if query::is_aliased_only_property(&graph, &edges, prop) {
                output::dim(" (aliased only)")
            } else {
                String::new()
            };
            println!(
                "  {}  {:<4} reference(s){}",
                output::property_name(prop),
                count,
                aliased_note,
            );
        }
        output::blank();
    }

    Ok(())
}

// ── usages ───────────────────────────────────────────────────

pub fn run_usages(
    symbol: &str,
    file: Option<&str>,
    path: &str,
    kinds: &[String],
    min_confidence: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mask = mask_from_args(kinds)?;
    let floor = resolution_floor(min_confidence)?;
    let root = super::normalize_path(
        &std::fs::canonicalize(path).map_err(|e| format!("cannot access '{}': {}", path, e))?,
    );
    let graph = super::analyze::load_graph(&root)?;

    output::banner("usages");
    print_symbol_header(&graph, symbol, file);
    report_filter(&mask);
    output::blank();
    warn_about_absent_kinds(&mask, &graph);

    let edges = query::symbol_usages(&graph, symbol, file, true, mask, floor)
        .map_err(|e| format_query_error(e, symbol))?;

    if edges.is_empty() {
        if mask.is_all() {
            output::success("No usages found.");
        } else {
            output::success("No usages found under the selected kinds.");
            output::dim_line("  Other relationship kinds were not searched.");
        }
        output::blank();
        return Ok(());
    }

    output::section(&format!("{} usage(s) found", edges.len()));
    for (i, edge) in edges.iter().enumerate() {
        print_edge(i + 1, edge, &graph);
    }

    Ok(())
}

// ── deps ─────────────────────────────────────────────────────

pub fn run_deps(
    symbol: &str,
    file: Option<&str>,
    depth: usize,
    path: &str,
    kinds: &[String],
    min_confidence: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mask = mask_from_args(kinds)?;
    let floor = resolution_floor(min_confidence)?;
    let root = super::normalize_path(
        &std::fs::canonicalize(path).map_err(|e| format!("cannot access '{}': {}", path, e))?,
    );
    let graph = super::analyze::load_graph(&root)?;

    output::banner("dependencies");
    print_symbol_header(&graph, symbol, file);
    output::stat("Depth", &depth.to_string());
    report_filter(&mask);
    output::blank();
    warn_about_absent_kinds(&mask, &graph);

    let edges = query::dependencies(&graph, symbol, file, Some(depth), mask, floor)
        .map_err(|e| format_query_error(e, symbol))?;

    if edges.is_empty() {
        if mask.is_all() {
            output::success("No dependencies found — this symbol is self-contained.");
        } else {
            output::success("No dependencies found under the selected kinds.");
            output::dim_line("  Other relationship kinds were not searched.");
        }
        output::blank();
        return Ok(());
    }

    let noun = if edges.len() == 1 {
        "dependency"
    } else {
        "dependencies"
    };
    output::section(&format!("{} {} found", edges.len(), noun));
    for (i, edge) in edges.iter().enumerate() {
        print_dep_edge(i + 1, edge, &graph);
    }

    Ok(())
}

// ── formatting helpers ───────────────────────────────────────

fn print_symbol_header(graph: &InvarGraph, symbol: &str, file: Option<&str>) {
    // try to find the canonical symbol for metadata
    if let Some(ids) = graph.name_index.get(symbol) {
        let target_id = if let Some(file) = file {
            ids.iter().find(|id| {
                graph
                    .symbols
                    .get(*id)
                    .map(|s| s.file == file)
                    .unwrap_or(false)
            })
        } else {
            ids.first()
        };

        if let Some(id) = target_id {
            if let Some(sym) = graph.symbols.get(id) {
                output::stat_highlight(
                    "Symbol",
                    &format!(
                        "{} {}",
                        output::symbol_name(&sym.name),
                        output::dim(&format!("[{}]", format_kind(&sym.kind))),
                    ),
                );
                output::stat(
                    "File",
                    &format!("{}:{}", output::file_path(&sym.file), sym.line_start),
                );
                return;
            }
        }
    }
    // fallback
    output::stat_highlight("Symbol", symbol);
}

fn print_edge(index: usize, edge: &QueryEdge, graph: &InvarGraph) {
    let num = output::dim(&format!("{index:>3}."));
    let location = format!("{}:{}", output::file_path(&edge.file), edge.line,);

    // A structural row is marked, because in a polyglot result it sits beside
    // resolved ones and reads identically otherwise — and the two mean very
    // different things to anyone deciding whether a change is safe.
    let tag = if edge.resolution.is_gate_safe() {
        output::dim(&format!("[{}]", query::kind_name(&edge.kind)))
    } else {
        output::dim(&format!(
            "[{}] [structural: within-file only]",
            query::kind_name(&edge.kind)
        ))
    };
    println!("  {num} {location} {tag}");

    // How this reference names the symbol.
    //
    // Both branches used to print "imports as X" — for "renamed to X" and for
    // "imported by X", which are opposite meanings. In the tool whose whole
    // pitch is alias awareness, that made the one line a reader most needs to
    // trust unreadable.
    if query::is_renamed(graph, edge) {
        if let Some(alias) = &edge.alias {
            println!(
                "       {} renamed to {}",
                output::dim("→"),
                output::alias_tag(alias),
            );
        }
    } else if let Some(sym) = graph.symbols.get(&edge.from) {
        // A module-scope referrer carries no information the location line
        // does not already give, and printing "referenced by module" was the
        // bulk of what the old branch emitted.
        if sym.kind != SymbolKind::Module {
            println!(
                "       {} referenced by {}",
                output::dim("→"),
                output::symbol_name(&sym.name),
            );
        }
    }

    // properties
    if !edge.properties_accessed.is_empty() {
        let props: Vec<String> = edge
            .properties_accessed
            .iter()
            .map(|p| output::property_name(p))
            .collect();
        println!("       {} accesses {}", output::dim("→"), props.join(", "),);
    }

    // context (truncated)
    if !edge.context.is_empty() && edge.context != "import" && edge.context != "property access" {
        let ctx = if edge.context.len() > 72 {
            format!("{}…", &edge.context[..72])
        } else {
            edge.context.clone()
        };
        println!(
            "       {} {}",
            output::dim("→"),
            output::dim(&format!("\"{}\"", ctx)),
        );
    }

    // hop distance
    if edge.hop > 1 {
        println!(
            "       {} {}",
            output::dim("→"),
            output::dim(&format!("(hop {})", edge.hop)),
        );
    }

    println!();
}

fn print_dep_edge(index: usize, edge: &QueryEdge, graph: &InvarGraph) {
    let num = output::dim(&format!("{index:>3}."));

    // for deps, the `to` is what we depend on
    let dep_name = graph
        .symbols
        .get(&edge.to)
        .map(|s| s.name.clone())
        .unwrap_or_else(|| edge.to.clone());

    let dep_kind = graph
        .symbols
        .get(&edge.to)
        .map(|s| format_kind(&s.kind).to_string())
        .unwrap_or_default();

    let location = format!("{}:{}", output::file_path(&edge.file), edge.line,);

    println!(
        "  {num} {} {} {}",
        output::symbol_name(&dep_name),
        output::dim(&format!("[{dep_kind}]")),
        output::dim(&format!("— {location}")),
    );

    if let Some(alias) = &edge.alias {
        println!(
            "       {} via alias {}",
            output::dim("→"),
            output::alias_tag(alias),
        );
    }

    if edge.hop > 1 {
        println!(
            "       {} {}",
            output::dim("→"),
            output::dim(&format!("(hop {})", edge.hop)),
        );
    }

    println!();
}

fn collect_property_summary(edges: &[QueryEdge]) -> Vec<(String, usize)> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for edge in edges {
        for prop in &edge.properties_accessed {
            *counts.entry(prop.clone()).or_insert(0) += 1;
        }
    }
    let mut sorted: Vec<_> = counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    sorted
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

fn format_query_error(err: openinvar_core::InvarError, symbol: &str) -> String {
    match err {
        openinvar_core::InvarError::SymbolNotFound(_) => {
            format!(
                "Symbol '{}' not found in the graph.\n  \
                 Hint: run `openinvar analyze` to index the repo, or check the symbol name.\n  \
                 If the symbol is a method, try: `openinvar query blast-radius ClassName`.",
                symbol
            )
        }
        openinvar_core::InvarError::AmbiguousSymbol { candidates, .. } => {
            let files = candidates
                .iter()
                .map(|f| format!("    • {}", output::file_path(f)))
                .collect::<Vec<_>>()
                .join("\n");
            format!(
                "Symbol '{}' is ambiguous. Found in:\n{}\n  \
                 Hint: use --file to narrow the search.",
                symbol, files
            )
        }
        other => format!("{other}"),
    }
}
