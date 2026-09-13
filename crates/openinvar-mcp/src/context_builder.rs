//! Formats graph query results into agent-readable text context.
//!
//! The output format is optimized for LLM consumption — structured, concise,
//! and highlighting aliased imports as high-risk items.

use openinvar_core::graph::InvarGraph;
use openinvar_core::ir::{Language, SymbolKind};
use openinvar_core::query::{self, QueryEdge};

use std::collections::BTreeMap;

// ── blast radius ─────────────────────────────────────────────

pub fn format_blast_radius(
    graph: &InvarGraph,
    symbol: &str,
    file: Option<&str>,
    depth: usize,
    edges: &[QueryEdge],
) -> String {
    let mut out = String::new();

    // header
    let header = symbol_header(graph, symbol, file);
    out.push_str(&header);
    out.push_str(&format!(
        "\nBlast radius ({} dependent(s), depth={}):\n",
        edges.len(),
        depth
    ));

    if edges.is_empty() {
        // An agent acts on this line. "Safe to modify" is a claim about the
        // whole repository, and it holds only if the whole graph was resolved
        // well enough to make it — a structural region cannot see across
        // files, so a reference living there would never have reached the
        // graph at all.
        let structural = openinvar_core::query::structural_files(graph);
        if structural.is_empty() {
            out.push_str("\nNo dependents found — safe to modify.\n");
        } else {
            out.push_str("\nNo dependents found.\n");
            out.push_str(&format!(
                "\nCAUTION: {} file(s) in this repository were analyzed \
                 within-file only (structural resolution), so a reference from \
                 one of them to this symbol would not appear above. Do not \
                 treat this empty result as proof the symbol is unused.\n",
                structural.len()
            ));
        }
        return out;
    }

    let (direct, aliased) = openinvar_core::query::partition_by_alias(graph, edges);

    // direct dependents
    if !direct.is_empty() {
        out.push_str(&format!("\nDIRECT (imports/uses {} directly):\n", symbol));
        for edge in &direct {
            out.push_str(&format_blast_edge(graph, edge));
        }
    }

    // aliased dependents — HIGH RISK
    if !aliased.is_empty() {
        out.push_str("\nALIASED (imports under different name — HIGH RISK):\n");
        for edge in &aliased {
            out.push_str(&format_blast_edge(graph, edge));
        }
    }

    // property summary
    let props = collect_property_summary(edges);
    if !props.is_empty() {
        out.push_str("\nProperties at risk if changed:\n");
        for (prop, count) in &props {
            let aliased_note = if query::is_aliased_only_property(graph, edges, prop) {
                " (aliased import only)"
            } else {
                ""
            };
            out.push_str(&format!(
                "  .{:<16} → referenced in {} file(s){}\n",
                prop, count, aliased_note
            ));
        }
    }

    out
}

fn format_blast_edge(graph: &InvarGraph, edge: &QueryEdge) -> String {
    let mut out = String::new();
    let lang = graph
        .symbols
        .get(&edge.from)
        .map(|s| format_language(&s.language))
        .unwrap_or("??");
    // A structural row sits beside resolved ones in a polyglot result and
    // reads identically otherwise. An agent deciding whether a change is safe
    // needs the difference on the row, not buried in a preamble.
    if edge.resolution.is_gate_safe() {
        out.push_str(&format!("  • {}:{} [{}]\n", edge.file, edge.line, lang));
    } else {
        out.push_str(&format!(
            "  • {}:{} [{}] (structural: within-file only)\n",
            edge.file, edge.line, lang
        ));
    }

    // "renamed to X" and "imported by X" are opposite facts, and both were
    // printed as "imports as X". An agent reading this cannot recover which
    // was meant, on the one line the tool exists to get right.
    if query::is_renamed(graph, edge) {
        if let Some(alias) = &edge.alias {
            out.push_str(&format!("    → renamed to {} ← ALIAS\n", alias));
        }
    } else if let Some(sym) = graph.symbols.get(&edge.from) {
        if sym.kind != SymbolKind::Module {
            out.push_str(&format!("    → referenced by {}\n", sym.name));
        }
    }

    if !edge.properties_accessed.is_empty() {
        let props: Vec<String> = edge
            .properties_accessed
            .iter()
            .map(|p| format!(".{p}"))
            .collect();
        out.push_str(&format!("    → accesses: {}\n", props.join(", ")));
    }

    if !edge.context.is_empty() && edge.context != "import" && edge.context != "property access" {
        let ctx = if edge.context.len() > 80 {
            format!("{}…", &edge.context[..80])
        } else {
            edge.context.clone()
        };
        out.push_str(&format!("    → context: \"{}\"\n", ctx));
    }

    out
}

fn format_language(lang: &Language) -> &'static str {
    match lang {
        Language::TypeScript => "TS",
        Language::JavaScript => "JS",
        Language::Python => "PY",
        Language::Rust => "RS",
        Language::Go => "GO",
        Language::C => "C",
        Language::Cpp => "C++",
        _ => "??",
    }
}

// ── dependencies ─────────────────────────────────────────────

pub fn format_dependencies(
    graph: &InvarGraph,
    symbol: &str,
    file: Option<&str>,
    depth: usize,
    edges: &[QueryEdge],
) -> String {
    let mut out = String::new();

    let header = symbol_header(graph, symbol, file);
    out.push_str(&header);
    out.push_str(&format!(
        "\nDependencies ({} found, depth={}):\n",
        edges.len(),
        depth
    ));

    if edges.is_empty() {
        out.push_str("\nNo dependencies found — this symbol is self-contained.\n");
        return out;
    }

    for edge in edges {
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

        out.push_str(&format!(
            "  • {} [{}] — {}:{}\n",
            dep_name, dep_kind, edge.file, edge.line
        ));
        if let Some(alias) = &edge.alias {
            out.push_str(&format!("    → via alias {}\n", alias));
        }
        if edge.hop > 1 {
            out.push_str(&format!("    → (hop {})\n", edge.hop));
        }
    }

    out
}

// ── symbol usages ────────────────────────────────────────────

pub fn format_symbol_usages(
    graph: &InvarGraph,
    symbol: &str,
    file: Option<&str>,
    edges: &[QueryEdge],
) -> String {
    let mut out = String::new();

    let header = symbol_header(graph, symbol, file);
    out.push_str(&header);
    out.push_str(&format!(
        "\nUsages ({} found, including aliases):\n",
        edges.len()
    ));

    if edges.is_empty() {
        out.push_str("\nNo usages found.\n");
        return out;
    }

    for edge in edges {
        out.push_str(&format!("  • {}:{}\n", edge.file, edge.line));
        // Only a genuine rename is worth an ALIAS marker: adapters record
        // the local name whether or not it differs from the symbol's own.
        if query::is_renamed(graph, edge) {
            if let Some(alias) = &edge.alias {
                out.push_str(&format!("    → renamed to {} ← ALIAS\n", alias));
            }
        }
        if !edge.properties_accessed.is_empty() {
            let props: Vec<String> = edge
                .properties_accessed
                .iter()
                .map(|p| format!(".{p}"))
                .collect();
            out.push_str(&format!("    → accesses: {}\n", props.join(", ")));
        }
        if !edge.context.is_empty() && edge.context != "import" && edge.context != "property access"
        {
            let ctx = if edge.context.len() > 80 {
                format!("{}…", &edge.context[..80])
            } else {
                edge.context.clone()
            };
            out.push_str(&format!("    → context: \"{}\"\n", ctx));
        }
    }

    out
}

// ── helpers ──────────────────────────────────────────────────

fn symbol_header(graph: &InvarGraph, symbol: &str, file: Option<&str>) -> String {
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
                return format!(
                    "Symbol: {} [{}] — {}:{}",
                    sym.name,
                    format_kind(&sym.kind),
                    sym.file,
                    sym.line_start,
                );
            }
        }
    }
    format!("Symbol: {}", symbol)
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
