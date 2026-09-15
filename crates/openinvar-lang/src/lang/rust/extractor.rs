use std::collections::BTreeSet;

use openinvar_core::ast::{first_line_of, node_text, start_line, walk};
use openinvar_core::ir::{Diagnostic, DiagnosticCategory, DiagnosticLevel, FileIR, Language, Relationship, RelationshipKind, Resolution, Symbol, SymbolKind};
use openinvar_core::symbol_id::{
    make_symbol_id, module_symbol, module_symbol_id, unresolved_import_id,
    unresolved_local_type_id, IMPORT_ALL,
};
use tree_sitter::Node;

use crate::lang::rust::macro_analyzer::derived_traits;
use crate::lang::rust::parser::ParsedFile;
use crate::lang::rust::scope_analyzer::collect_type_accesses;

pub fn extract_file_ir(parsed: &ParsedFile) -> FileIR {
    let source = parsed.source.as_bytes();
    let root = parsed.tree.root_node();
    let file = parsed.file.as_str();

    let mut symbols = vec![module_symbol(file, Language::Rust)];
    let mut relationships = Vec::new();
    let mut diagnostics = parsed.diagnostics.clone();

    let stats = walk(root, &mut |node| match node.kind() {
        "struct_item" | "union_item" => {
            push_item(node, source, file, SymbolKind::Class, &mut symbols);
            relationships.extend(derive_edges(node, source, file, SymbolKind::Class));
        }
        "enum_item" => {
            push_item(node, source, file, SymbolKind::Enum, &mut symbols);
            relationships.extend(derive_edges(node, source, file, SymbolKind::Enum));
            symbols.extend(enum_variants(node, source, file));
        }
        "trait_item" => push_item(node, source, file, SymbolKind::Interface, &mut symbols),
        "type_item" => push_item(node, source, file, SymbolKind::TypeAlias, &mut symbols),
        "const_item" | "static_item" => {
            push_item(node, source, file, SymbolKind::Variable, &mut symbols)
        }
        "function_item" => {
            // Functions inside an `impl` or `trait` body are methods; they are
            // emitted by `impl_block` so they can be qualified by their owning
            // type, which keeps two `fn new` in one file from colliding.
            if node.parent().map(|p| p.kind()) != Some("declaration_list") {
                push_item(node, source, file, SymbolKind::Function, &mut symbols);
            }
        }
        "impl_item" => impl_block(node, source, file, &mut symbols, &mut relationships),
        "use_declaration" => relationships.extend(use_declaration(node, source, file)),
        _ => {}
    });

    relationships.extend(property_access_edges(root, source, file));
    relationships.extend(call_edges(root, source, file));

    if stats.truncated() {
        diagnostics.push(Diagnostic {
            level: DiagnosticLevel::Warning,
            category: DiagnosticCategory::Parse,
            message: format!(
                "{} deeply nested subtree(s) exceeded the traversal depth limit; \
                 symbols below them were not extracted",
                stats.skipped_subtrees
            ),
            file: Some(parsed.file.clone()),
            line: None,
        });
    }

    // Counted from the tree this adapter already parsed, so the graph
    // and the count cannot disagree. Computed before the literal because
    // the literal moves `symbols`.
    let assertions =
        crate::assertions::count_by_symbol(&parsed.tree, &parsed.source, &Language::Rust, &symbols);

    FileIR {
        file: parsed.file.clone(),
        language: Language::Rust,
        symbols,
        relationships,
        diagnostics,
        re_exports: Vec::new(),
        assertions,
        assertions_counted: crate::assertions::counts_assertions(&Language::Rust),
        unbound_references: 0,
        unbound_outside_repository: 0,
        references_counted: false,
    }
}

// ── calls ────────────────────────────────────────────────────

/// Calls and struct constructions whose target is a name the file can resolve.
///
/// Rust spells the three cases differently, and each is recorded as what it
/// actually is:
///
/// - `foo(...)` — a free function, or a tuple-struct constructor. The kind is
///   settled by the resolver from the target's kind, since `Foo(1)` and
///   `foo(1)` are the same node.
/// - `Foo::new(...)` — an associated function. The edge points at the *method*
///   `Foo::new`, not at `Foo`: that is the symbol that runs. Calling it an
///   instantiation would be reading meaning into the name `new`, which returns
///   `Self` only by convention.
/// - `Foo { .. }` — a struct literal, which is Rust's actual construction
///   syntax, so this is the one shape that is `Instantiates` outright.
///
/// `obj.method(...)` records nothing, matching TypeScript and Python: it is
/// already a property access on the receiver, and a call edge to the receiver
/// would claim the receiver was called. `println!(...)` is a macro invocation
/// rather than a call node, so macro-generated calls stay outside the graph —
/// consistent with macro bodies being token trees everywhere else.
///
/// Returns `(callee_name, kind, line)`, sorted and deduplicated, so identical
/// input yields identical output.
fn call_edges(root: Node<'_>, source: &[u8], file: &str) -> Vec<Relationship> {
    let mut found: Vec<(String, RelationshipKind, u32)> = Vec::new();

    walk(root, &mut |node| {
        let (name_node, kind) = match node.kind() {
            // `Foo { .. }` — construction, unambiguously.
            "struct_expression" => match node.child_by_field_name("name") {
                Some(n) => (n, RelationshipKind::Instantiates),
                None => return,
            },
            // A callee that is neither a bare name nor a path is a method call
            // or a computed expression, both covered by property access.
            "call_expression" => match node.child_by_field_name("function") {
                Some(n) if matches!(n.kind(), "identifier" | "scoped_identifier") => {
                    (n, RelationshipKind::Calls)
                }
                _ => return,
            },
            _ => return,
        };

        let Some(text) = node_text(name_node, source) else {
            return;
        };
        let name = text.trim();
        if name.is_empty() {
            return;
        }

        found.push((name.to_string(), kind, start_line(name_node)));
    });

    // One row per (callee, kind, line): a name called twice on one line is one
    // fact about that line, and the order must not depend on traversal.
    found.sort();
    found.dedup();

    found
        .into_iter()
        .map(|(callee, kind, line)| Relationship {
            from: module_symbol_id(file),
            to: unresolved_local_type_id(&callee),
            kind: kind.clone(),
            alias: Some(callee),
            properties_accessed: Vec::new(),
            context: match kind {
                RelationshipKind::Instantiates => "instantiation".to_string(),
                _ => "call".to_string(),
            },
            file: file.to_string(),
            line,
            resolution: Resolution::default(),
        })
        .collect()
}

// ── symbols ──────────────────────────────────────────────────

fn push_item(
    node: Node<'_>,
    source: &[u8],
    file: &str,
    kind: SymbolKind,
    symbols: &mut Vec<Symbol>,
) {
    let Some(name) = node
        .child_by_field_name("name")
        .and_then(|n| node_text(n, source))
    else {
        return;
    };
    symbols.push(symbol(file, name, name, kind, node, source));
}

/// Build a symbol whose id may be qualified (`Type::method`) while its
/// searchable name stays bare (`method`).
fn symbol(
    file: &str,
    id_name: &str,
    display_name: &str,
    kind: SymbolKind,
    node: Node<'_>,
    source: &[u8],
) -> Symbol {
    Symbol {
        id: make_symbol_id(file, id_name, &kind),
        name: display_name.to_string(),
        kind,
        language: Language::Rust,
        file: file.to_string(),
        line_start: start_line(node),
        line_end: node.end_position().row as u32 + 1,
        signature: first_line_of(node, source),
    }
}

fn enum_variants(node: Node<'_>, source: &[u8], file: &str) -> Vec<Symbol> {
    let Some(body) = node.child_by_field_name("body") else {
        return Vec::new();
    };
    let enum_name = node
        .child_by_field_name("name")
        .and_then(|n| node_text(n, source))
        .unwrap_or("");

    let mut out = Vec::new();
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() != "enum_variant" {
            continue;
        }
        let Some(name) = child
            .child_by_field_name("name")
            .and_then(|n| node_text(n, source))
        else {
            continue;
        };
        // Qualified id keeps `Status::Active` distinct from `State::Active`.
        out.push(symbol(
            file,
            &format!("{enum_name}::{name}"),
            name,
            SymbolKind::EnumVariant,
            child,
            source,
        ));
    }
    out
}

fn impl_block(
    node: Node<'_>,
    source: &[u8],
    file: &str,
    symbols: &mut Vec<Symbol>,
    relationships: &mut Vec<Relationship>,
) {
    let impl_type = node
        .child_by_field_name("type")
        .and_then(|t| type_name_of(t, source));

    // `impl Trait for Type` is an implementation edge.
    if let (Some(trait_node), Some(type_name)) =
        (node.child_by_field_name("trait"), impl_type.as_deref())
    {
        if let Some(trait_name) = type_name_of(trait_node, source) {
            relationships.push(Relationship {
                from: make_symbol_id(file, type_name, &SymbolKind::Class),
                to: unresolved_local_type_id(&trait_name),
                kind: RelationshipKind::Implements,
                alias: Some(trait_name),
                properties_accessed: Vec::new(),
                context: first_line_of(node, source).unwrap_or_default(),
                file: file.to_string(),
                line: start_line(node),
                resolution: Resolution::default(),
            });
        }
    }

    let Some(body) = node.child_by_field_name("body") else {
        return;
    };
    let owner = impl_type.as_deref().unwrap_or("impl");

    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() != "function_item" {
            continue;
        }
        let Some(name) = child
            .child_by_field_name("name")
            .and_then(|n| node_text(n, source))
        else {
            continue;
        };
        let mut method = symbol(
            file,
            &format!("{owner}::{name}"),
            name,
            SymbolKind::Method,
            child,
            source,
        );
        method.signature = Some(format!(
            "impl {owner} :: {}",
            first_line_of(child, source).unwrap_or_else(|| name.to_string())
        ));
        symbols.push(method);
    }
}

/// Traits brought in by `#[derive(..)]`, as implementation edges.
fn derive_edges(node: Node<'_>, source: &[u8], file: &str, kind: SymbolKind) -> Vec<Relationship> {
    let Some(name) = node
        .child_by_field_name("name")
        .and_then(|n| node_text(n, source))
    else {
        return Vec::new();
    };

    derived_traits(node, source)
        .into_iter()
        // Prelude traits are on nearly every type and resolve to nothing local.
        .filter(|d| !d.is_std)
        .map(|d| Relationship {
            from: make_symbol_id(file, name, &kind),
            to: unresolved_local_type_id(&d.name),
            kind: RelationshipKind::Implements,
            alias: Some(d.name.clone()),
            properties_accessed: Vec::new(),
            context: format!("#[derive({})]", d.name),
            file: file.to_string(),
            line: d.line,
            resolution: Resolution::default(),
        })
        .collect()
}

/// The name a member access or trait reference resolves against.
fn type_name_of(node: Node<'_>, source: &[u8]) -> Option<String> {
    match node.kind() {
        "type_identifier" => node_text(node, source).map(str::to_string),
        "generic_type" => node
            .child_by_field_name("type")
            .and_then(|inner| type_name_of(inner, source)),
        "scoped_type_identifier" => node
            .child_by_field_name("name")
            .and_then(|n| node_text(n, source))
            .map(str::to_string),
        "reference_type" | "pointer_type" => node
            .child_by_field_name("type")
            .and_then(|inner| type_name_of(inner, source)),
        _ => None,
    }
}

// ── imports ──────────────────────────────────────────────────

fn use_declaration(node: Node<'_>, source: &[u8], file: &str) -> Vec<Relationship> {
    let mut out = Vec::new();
    let context = first_line_of(node, source).unwrap_or_default();
    let line = start_line(node);
    if let Some(tree) = node.child_by_field_name("argument") {
        collect_use_tree(tree, source, "", file, line, &context, &mut out);
    }
    out
}

/// Flatten a `use` tree into one relationship per imported name.
///
/// `use a::{b::C, d as E, f::*}` yields three, each carrying the module path it
/// came from so the resolver can look the name up in the right module rather
/// than searching the whole repository for a matching leaf.
fn collect_use_tree(
    node: Node<'_>,
    source: &[u8],
    prefix: &str,
    file: &str,
    line: u32,
    context: &str,
    out: &mut Vec<Relationship>,
) {
    let join = |head: &str, tail: &str| -> String {
        if head.is_empty() {
            tail.to_string()
        } else {
            format!("{head}::{tail}")
        }
    };

    match node.kind() {
        "use_as_clause" => {
            let (Some(path), Some(alias)) = (
                node.child_by_field_name("path")
                    .and_then(|p| node_text(p, source)),
                node.child_by_field_name("alias")
                    .and_then(|a| node_text(a, source)),
            ) else {
                return;
            };
            let full = join(prefix, path);
            let (module, symbol) = split_path(&full);
            out.push(import(file, &module, &symbol, Some(alias), context, line));
        }
        "use_list" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if !matches!(child.kind(), "{" | "}" | ",") {
                    collect_use_tree(child, source, prefix, file, line, context, out);
                }
            }
        }
        "scoped_use_list" => {
            let path = node
                .child_by_field_name("path")
                .and_then(|p| node_text(p, source))
                .unwrap_or("");
            let inner = join(prefix, path);
            if let Some(list) = node.child_by_field_name("list") {
                collect_use_tree(list, source, &inner, file, line, context, out);
            }
        }
        "use_wildcard" => {
            // `use foo::*` — the module itself, with no single named symbol.
            let path = node
                .named_child(0)
                .and_then(|p| node_text(p, source))
                .unwrap_or("");
            let module = join(prefix, path);
            out.push(import(file, &module, IMPORT_ALL, None, context, line));
        }
        "self" => {
            // `use foo::{self, Bar}` — `self` is the module `foo`.
            if !prefix.is_empty() {
                out.push(import(file, prefix, IMPORT_ALL, None, context, line));
            }
        }
        "identifier" | "scoped_identifier" | "crate" | "super" => {
            let Some(path) = node_text(node, source) else {
                return;
            };
            let full = join(prefix, path);
            let (module, symbol) = split_path(&full);
            out.push(import(file, &module, &symbol, None, context, line));
        }
        _ => {}
    }
}

/// Split `a::b::C` into the module `a::b` and the name `C`.
fn split_path(path: &str) -> (String, String) {
    match path.rfind("::") {
        Some(cut) => (path[..cut].to_string(), path[cut + 2..].to_string()),
        // A bare `use foo;` imports the module `foo` itself.
        None => (path.to_string(), IMPORT_ALL.to_string()),
    }
}

fn import(
    file: &str,
    module: &str,
    symbol: &str,
    alias: Option<&str>,
    context: &str,
    line: u32,
) -> Relationship {
    Relationship {
        from: module_symbol_id(file),
        to: unresolved_import_id(module, symbol),
        kind: RelationshipKind::Imports,
        alias: alias.map(str::to_string),
        properties_accessed: Vec::new(),
        context: context.to_string(),
        file: file.to_string(),
        line,
        resolution: Resolution::default(),
    }
}

// ── property access ──────────────────────────────────────────

/// One edge per type whose members this file touches.
///
/// Grouping by resolved type — rather than merging every access in the file —
/// is what keeps one struct's fields from being attributed to another.
fn property_access_edges(root: Node<'_>, source: &[u8], file: &str) -> Vec<Relationship> {
    collect_type_accesses(root, source)
        .into_iter()
        .filter(|(_, access)| !access.properties.is_empty())
        .map(|(type_name, access)| Relationship {
            from: module_symbol_id(file),
            to: unresolved_local_type_id(&type_name),
            kind: RelationshipKind::AccessesProperty,
            alias: Some(type_name),
            properties_accessed: access
                .properties
                .into_iter()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            context: "property access".to_string(),
            file: file.to_string(),
            line: access.first_line,
            resolution: Resolution::default(),
        })
        .collect()
}
