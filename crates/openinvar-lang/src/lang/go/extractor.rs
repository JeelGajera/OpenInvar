use openinvar_core::ast::{first_line_of, node_text, start_line, walk};
use openinvar_core::ir::{Diagnostic, DiagnosticCategory, DiagnosticLevel, FileIR, Language, Relationship, RelationshipKind, Resolution, Symbol, SymbolKind};
use std::collections::BTreeSet;

use openinvar_core::symbol_id::{
    make_symbol_id, module_symbol, module_symbol_id, parse_unresolved_import_id,
    unresolved_import_id, unresolved_local_type_id, IMPORT_ALL,
};
use tree_sitter::Node;

use crate::lang::go::parser::ParsedFile;
use crate::lang::go::scope_analyzer::{analyze, is_builtin_type};

pub fn extract_file_ir(parsed: &ParsedFile) -> FileIR {
    let source = parsed.source.as_bytes();
    let root = parsed.tree.root_node();
    let file = parsed.file.as_str();

    let mut symbols = vec![module_symbol(file, Language::Go)];
    let mut relationships = Vec::new();
    let mut diagnostics = parsed.diagnostics.clone();

    let stats = walk(root, &mut |node| match node.kind() {
        "type_declaration" => type_declaration(node, source, file, &mut symbols),
        "function_declaration" => {
            if let Some(name) = field_name(node, source) {
                symbols.push(symbol(file, name, name, SymbolKind::Function, node, source));
            }
        }
        "method_declaration" => method_declaration(node, source, file, &mut symbols),
        "import_declaration" => relationships.extend(imports(node, source, file)),
        _ => {}
    });

    relationships.extend(call_edges(root, source, file, &package_names(&relationships)));

    // Property accesses and type references, both grounded in declared types.
    let analysis = analyze(root, source);

    for (type_name, access) in analysis.accesses {
        if access.properties.is_empty() {
            continue;
        }
        relationships.push(Relationship {
            from: module_symbol_id(file),
            to: unresolved_local_type_id(&type_name),
            kind: RelationshipKind::AccessesProperty,
            alias: Some(type_name),
            properties_accessed: access.properties.into_iter().collect(),
            context: "property access".to_string(),
            file: file.to_string(),
            line: access.first_line,
            resolution: Resolution::default(),
        });
    }

    for (type_name, line) in analysis.referenced_types {
        if is_builtin_type(&type_name) {
            continue;
        }
        relationships.push(Relationship {
            from: module_symbol_id(file),
            to: unresolved_local_type_id(&type_name),
            kind: RelationshipKind::UsesType,
            alias: Some(type_name),
            properties_accessed: Vec::new(),
            context: "type reference".to_string(),
            file: file.to_string(),
            line,
            resolution: Resolution::default(),
        });
    }

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

    FileIR {
        file: parsed.file.clone(),
        language: Language::Go,
        symbols,
        relationships,
        diagnostics,
        re_exports: Vec::new(),
    }
}

// ── calls ────────────────────────────────────────────────────

/// The names this file can use to qualify a reference to another package.
///
/// `import "app/models"` binds `models`; `import m "app/models"` binds `m`.
/// Blank and dot imports bind nothing and are already excluded upstream, where
/// they produce no alias.
///
/// This set is what separates `models.New(..)` — a call into another package —
/// from `user.Save(..)`, a method on a value. Go spells both with a selector,
/// and the file's own import list is the only thing that tells them apart.
fn package_names(relationships: &[Relationship]) -> BTreeSet<String> {
    relationships
        .iter()
        .filter(|rel| rel.kind == RelationshipKind::Imports)
        .filter_map(|rel| {
            if let Some(alias) = &rel.alias {
                return Some(alias.clone());
            }
            let (path, _) = parse_unresolved_import_id(&rel.to)?;
            Some(path.rsplit('/').next().unwrap_or(path).to_string())
        })
        .collect()
}

/// Calls and composite literals whose target is a name the file can resolve.
///
/// Go separates the two cleanly, unlike Python and Rust:
///
/// - `New(..)` and `models.New(..)` are calls. The second is the shape that
///   matters most, because a cross-package call in Go is *always* written
///   through the package name — skipping selectors, as the other languages do,
///   would leave call edges that never cross a file boundary.
/// - `Foo{..}` and `models.Foo{..}` are composite literals, which is Go's
///   construction syntax, so they are `Instantiates` outright. `&Foo{..}` is
///   the same literal under a unary `&` and is reached the same way.
///
/// `user.Save(..)` records nothing: the operand is a value, not a package, so
/// it is a method call — already a property access on the receiver, and a call
/// edge to the receiver would claim the receiver was called.
///
/// A composite literal for a slice, map or array (`[]string{..}`) has a
/// composite type rather than a name, and names no symbol to point at.
///
/// Returns relationships sorted and deduplicated by the caller's key, so
/// identical input yields identical output.
fn call_edges(
    root: Node<'_>,
    source: &[u8],
    file: &str,
    packages: &BTreeSet<String>,
) -> Vec<Relationship> {
    let mut found: BTreeSet<(String, u32, bool)> = BTreeSet::new();

    walk(root, &mut |node| match node.kind() {
        "call_expression" => {
            let Some(function) = node.child_by_field_name("function") else {
                return;
            };
            let name = match function.kind() {
                "identifier" => node_text(function, source).map(str::to_string),
                "selector_expression" => qualified_package_name(function, source, packages),
                _ => None,
            };
            if let Some(name) = name {
                found.insert((name, start_line(function), false));
            }
        }
        "composite_literal" => {
            let Some(ty) = node.child_by_field_name("type") else {
                return;
            };
            let name = match ty.kind() {
                "type_identifier" => node_text(ty, source).map(str::to_string),
                "qualified_type" => qualified_type_name(ty, source, packages),
                _ => None,
            };
            if let Some(name) = name {
                found.insert((name, start_line(ty), true));
            }
        }
        _ => {}
    });

    found
        .into_iter()
        .map(|(name, line, constructs)| Relationship {
            from: module_symbol_id(file),
            to: unresolved_local_type_id(&name),
            kind: if constructs {
                RelationshipKind::Instantiates
            } else {
                RelationshipKind::Calls
            },
            alias: Some(name),
            properties_accessed: Vec::new(),
            context: if constructs { "composite literal" } else { "call" }.to_string(),
            file: file.to_string(),
            line,
            resolution: Resolution::default(),
        })
        .collect()
}

/// `models.New` from a selector, but only when `models` is an imported package.
fn qualified_package_name(
    selector: Node<'_>,
    source: &[u8],
    packages: &BTreeSet<String>,
) -> Option<String> {
    let operand = selector.child_by_field_name("operand")?;
    if operand.kind() != "identifier" {
        return None;
    }
    let package = node_text(operand, source)?;
    if !packages.contains(package) {
        return None;
    }
    let field = selector.child_by_field_name("field")?;
    Some(format!("{package}.{}", node_text(field, source)?))
}

/// `models.Foo` from a qualified type in a composite literal.
fn qualified_type_name(
    ty: Node<'_>,
    source: &[u8],
    packages: &BTreeSet<String>,
) -> Option<String> {
    let package = ty
        .child_by_field_name("package")
        .and_then(|n| node_text(n, source))?;
    if !packages.contains(package) {
        return None;
    }
    let name = ty
        .child_by_field_name("name")
        .and_then(|n| node_text(n, source))?;
    Some(format!("{package}.{name}"))
}

fn field_name<'a>(node: Node<'_>, source: &'a [u8]) -> Option<&'a str> {
    node.child_by_field_name("name")
        .and_then(|n| node_text(n, source))
}

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
        language: Language::Go,
        file: file.to_string(),
        line_start: start_line(node),
        line_end: node.end_position().row as u32 + 1,
        signature: first_line_of(node, source),
    }
}

fn type_declaration(node: Node<'_>, source: &[u8], file: &str, symbols: &mut Vec<Symbol>) {
    let mut cursor = node.walk();
    for spec in node.children(&mut cursor) {
        if !matches!(spec.kind(), "type_spec" | "type_alias") {
            continue;
        }
        let Some(name) = field_name(spec, source) else {
            continue;
        };
        let body = spec.child_by_field_name("type");
        let kind = match (spec.kind(), body.map(|n| n.kind())) {
            (_, Some("interface_type")) => SymbolKind::Interface,
            ("type_alias", _) => SymbolKind::TypeAlias,
            _ => SymbolKind::Class,
        };

        let is_interface = kind == SymbolKind::Interface;
        symbols.push(symbol(file, name, name, kind, spec, source));

        // Interface methods become symbols owned by the interface, which is how
        // `interface_detector` later learns the method set it requires without
        // re-parsing signature text.
        if is_interface {
            if let Some(body) = body {
                symbols.extend(interface_methods(body, source, file, name));
            }
        }
    }
}

fn interface_methods(body: Node<'_>, source: &[u8], file: &str, interface: &str) -> Vec<Symbol> {
    let mut out = Vec::new();
    let mut cursor = body.walk();
    for member in body.children(&mut cursor) {
        if member.kind() != "method_elem" && member.kind() != "method_spec" {
            continue;
        }
        let Some(name) = field_name(member, source) else {
            continue;
        };
        out.push(symbol(
            file,
            &format!("{interface}::{name}"),
            name,
            SymbolKind::Method,
            member,
            source,
        ));
    }
    out
}

fn method_declaration(node: Node<'_>, source: &[u8], file: &str, symbols: &mut Vec<Symbol>) {
    let Some(name) = field_name(node, source) else {
        return;
    };
    // The receiver type owns the method; qualifying the id keeps two `Map`
    // methods on different types from colliding on one node.
    let owner = node
        .child_by_field_name("receiver")
        .and_then(|r| receiver_type(r, source))
        .unwrap_or_else(|| "func".to_string());

    symbols.push(symbol(
        file,
        &format!("{owner}::{name}"),
        name,
        SymbolKind::Method,
        node,
        source,
    ));
}

fn receiver_type(receiver: Node<'_>, source: &[u8]) -> Option<String> {
    let mut cursor = receiver.walk();
    for param in receiver.children(&mut cursor) {
        if param.kind() != "parameter_declaration" {
            continue;
        }
        let ty = param.child_by_field_name("type")?;
        return bare_type_name(ty, source);
    }
    None
}

fn bare_type_name(ty: Node<'_>, source: &[u8]) -> Option<String> {
    match ty.kind() {
        "type_identifier" => node_text(ty, source).map(str::to_string),
        "pointer_type" | "generic_type" => ty
            .named_child(0)
            .and_then(|inner| bare_type_name(inner, source)),
        _ => None,
    }
}

/// One relationship per `import` spec.
///
/// Go imports a package, never a symbol, so the placeholder always names the
/// whole package. Which member is used is recorded separately, by the qualified
/// references the scope analyzer collects.
fn imports(node: Node<'_>, source: &[u8], file: &str) -> Vec<Relationship> {
    let mut out = Vec::new();

    walk(node, &mut |spec| {
        if spec.kind() != "import_spec" {
            return;
        }
        let Some(raw_path) = spec
            .child_by_field_name("path")
            .and_then(|p| node_text(p, source))
        else {
            return;
        };
        let package_path = raw_path.trim_matches('"');

        // `_ "pkg"` is a blank import for side effects and `. "pkg"` dumps the
        // package into file scope; neither introduces a usable local name.
        let alias = match spec
            .child_by_field_name("name")
            .and_then(|n| node_text(n, source))
        {
            Some("_") | Some(".") | None => None,
            Some(name) => Some(name.to_string()),
        };

        out.push(Relationship {
            from: module_symbol_id(file),
            to: unresolved_import_id(package_path, IMPORT_ALL),
            kind: RelationshipKind::Imports,
            alias,
            properties_accessed: Vec::new(),
            context: node_text(spec, source).unwrap_or_default().to_string(),
            file: file.to_string(),
            line: start_line(spec),
            resolution: Resolution::default(),
        });
    });

    out
}
