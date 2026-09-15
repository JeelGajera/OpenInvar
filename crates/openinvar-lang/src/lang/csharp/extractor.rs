//! Symbols and unresolved references, from one C# file.
//!
//! Within-file work only. Every type reference leaves here as a placeholder
//! naming the identifier that was written, because which type that names
//! depends on the file's namespace, its `using` directives and its aliases —
//! and those are [`import_resolver`](super::import_resolver)'s business.
//!
//! # What C# adds over Java
//!
//! A file's namespace comes in two spellings: the block form
//! `namespace A { … }`, which can nest, and the file-scoped form
//! `namespace A;`, which applies to everything after it. Both are handled here
//! because a type's namespace decides its fully-qualified name, and getting it
//! wrong silently moves the type somewhere nothing will find it.

use std::collections::BTreeMap;

use openinvar_core::ir::{
    FileIR, Language, Relationship, RelationshipKind, Resolution, Symbol, SymbolKind,
};
use openinvar_core::symbol_id::{
    make_symbol_id, module_symbol, module_symbol_id, unresolved_local_type_id,
};
use tree_sitter::Node;

use super::parser::ParsedFile;

/// What a file says about where its names come from.
#[derive(Debug, Default, Clone)]
pub struct FileFacts {
    pub file: String,
    /// Namespaces imported with a plain `using`. C#'s `using` names a
    /// *namespace*, not a type, so this is the on-demand set — closer to Java's
    /// wildcard import than to its single-type one.
    pub usings: Vec<String>,
    /// `using Fmt = App.Util.Formatter` — an explicit alias for one type.
    pub aliases: Vec<(String, String)>,
    /// `using static App.Util.Helpers` — the type whose members come into scope.
    pub static_usings: Vec<String>,
    /// Each declared type's simple name and the namespace it was declared in.
    pub declared: Vec<(String, String)>,
}

pub fn text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    source.get(node.start_byte()..node.end_byte()).unwrap_or("")
}

fn line_of(node: Node<'_>) -> u32 {
    node.start_position().row as u32 + 1
}

pub fn direct_children<'t>(node: Node<'t>) -> Vec<Node<'t>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).collect()
}

fn child_of_kind<'t>(node: Node<'t>, kind: &str) -> Option<Node<'t>> {
    direct_children(node).into_iter().find(|c| c.kind() == kind)
}

pub fn descendants_of_kind<'t>(node: Node<'t>, kind: &str) -> Vec<Node<'t>> {
    let mut out = Vec::new();
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        if current.kind() == kind && current.id() != node.id() {
            out.push(current);
        }
        for child in direct_children(current) {
            stack.push(child);
        }
    }
    out.sort_by_key(|n| n.start_byte());
    out
}

const TYPE_KINDS: &[&str] = &[
    "class_declaration",
    "interface_declaration",
    "struct_declaration",
    "record_declaration",
    "record_struct_declaration",
    "enum_declaration",
];

fn symbol_kind_for(node_kind: &str) -> SymbolKind {
    match node_kind {
        "interface_declaration" => SymbolKind::Interface,
        "enum_declaration" => SymbolKind::Enum,
        _ => SymbolKind::Class,
    }
}

pub fn extract(parsed: &ParsedFile) -> (FileIR, FileFacts) {
    let file = parsed.file.as_str();
    let source = parsed.source.as_str();
    let root = parsed.tree.root_node();

    let mut facts = FileFacts {
        file: file.to_string(),
        ..Default::default()
    };
    let mut symbols = vec![module_symbol(file, Language::CSharp)];
    let mut relationships = Vec::new();

    collect_usings(root, source, file, &mut facts, &mut relationships);
    // The compilation unit is the outermost namespace scope, and it is the
    // empty one: a type declared outside any `namespace` lives in the global
    // namespace and is named by its simple name alone.
    walk_scope(root, "", file, source, &mut symbols, &mut relationships, &mut facts);

    (
        FileIR {
            file: file.to_string(),
            language: Language::CSharp,
            symbols,
            relationships,
            diagnostics: parsed.diagnostics.clone(),
            re_exports: Vec::new(),
            assertions: Default::default(),
            assertions_counted: false,
            unbound_references: 0,
            unbound_outside_repository: 0,
            references_counted: false,
        },
        facts,
    )
}

fn collect_usings(
    root: Node<'_>,
    source: &str,
    file: &str,
    facts: &mut FileFacts,
    relationships: &mut Vec<Relationship>,
) {
    for directive in descendants_of_kind(root, "using_directive") {
        let raw = text(directive, source);
        let named: Vec<Node<'_>> = direct_children(directive)
            .into_iter()
            .filter(|c| matches!(c.kind(), "identifier" | "qualified_name"))
            .collect();

        // `using Alias = Namespace.Type;` is the only form with two names, and
        // it is the only one that binds a single type. The rest name a
        // namespace, which is why C# has no direct equivalent of Java's
        // single-type import.
        if named.len() == 2 {
            facts.aliases.push((
                text(named[0], source).to_string(),
                text(named[1], source).to_string(),
            ));
            continue;
        }
        let Some(name_node) = named.first() else {
            continue;
        };
        let path = text(*name_node, source).to_string();

        if raw.starts_with("using static") {
            facts.static_usings.push(path);
            continue;
        }

        facts.usings.push(path.clone());
        relationships.push(Relationship {
            from: module_symbol_id(file),
            // The whole namespace, not a member of it. `IMPORT_ALL` is the
            // established spelling for that.
            to: openinvar_core::symbol_id::unresolved_import_id(
                &path,
                openinvar_core::symbol_id::IMPORT_ALL,
            ),
            kind: RelationshipKind::Imports,
            alias: None,
            properties_accessed: vec![],
            context: "using".to_string(),
            file: file.to_string(),
            line: line_of(directive),
            resolution: Resolution::default(),
        });
    }
}

/// Walk a namespace scope, recursing into nested namespaces and types.
fn walk_scope(
    node: Node<'_>,
    namespace: &str,
    file: &str,
    source: &str,
    symbols: &mut Vec<Symbol>,
    relationships: &mut Vec<Relationship>,
    facts: &mut FileFacts,
) {
    for child in direct_children(node) {
        match child.kind() {
            "namespace_declaration" | "file_scoped_namespace_declaration" => {
                let name = child
                    .child_by_field_name("name")
                    .map(|n| text(n, source).to_string())
                    .unwrap_or_default();
                let inner = if namespace.is_empty() {
                    name
                } else {
                    format!("{namespace}.{name}")
                };
                // A file-scoped namespace has no body: everything after it in
                // the compilation unit belongs to it, so the remaining
                // siblings are walked in the inner scope rather than the outer.
                if child.kind() == "file_scoped_namespace_declaration" {
                    walk_scope(node, &inner, file, source, symbols, relationships, facts);
                    return;
                }
                if let Some(body) = child_of_kind(child, "declaration_list") {
                    walk_scope(body, &inner, file, source, symbols, relationships, facts);
                }
            }
            k if TYPE_KINDS.contains(&k) => {
                collect_type(child, namespace, file, source, symbols, relationships, facts);
            }
            _ => {}
        }
    }
}

fn collect_type(
    node: Node<'_>,
    namespace: &str,
    file: &str,
    source: &str,
    symbols: &mut Vec<Symbol>,
    relationships: &mut Vec<Relationship>,
    facts: &mut FileFacts,
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let name = text(name_node, source).to_string();
    let kind = symbol_kind_for(node.kind());
    let id = make_symbol_id(file, &name, &kind);

    facts.declared.push((name.clone(), namespace.to_string()));
    symbols.push(Symbol {
        id: id.clone(),
        name: name.clone(),
        kind: kind.clone(),
        language: Language::CSharp,
        file: file.to_string(),
        line_start: line_of(node),
        line_end: node.end_position().row as u32 + 1,
        signature: Some(first_line(text(node, source))),
    });

    // `: Base, IAuditable` — C# does not distinguish a base class from an
    // implemented interface syntactically, and neither does this. Both are
    // recorded as Implements rather than guessing which one a name is, because
    // the guess would be wrong exactly when a repository names them alike.
    if let Some(bases) = child_of_kind(node, "base_list") {
        for base in direct_children(bases) {
            if !matches!(base.kind(), "identifier" | "qualified_name" | "generic_name") {
                continue;
            }
            let target = simple_name(text(base, source));
            relationships.push(reference(
                file,
                &id,
                &target,
                RelationshipKind::Implements,
                "base list",
                line_of(bases),
            ));
        }
    }

    let Some(body) = child_of_kind(node, "declaration_list").or_else(|| child_of_kind(node, "enum_member_declaration_list")) else {
        return;
    };

    for member in direct_children(body) {
        match member.kind() {
            "field_declaration" => collect_field(member, file, source, &name, symbols, relationships),
            "property_declaration" => {
                collect_property(member, file, source, &name, symbols, relationships)
            }
            "method_declaration" | "constructor_declaration" => {
                collect_method(member, file, source, &name, symbols, relationships)
            }
            k if TYPE_KINDS.contains(&k) => {
                collect_type(member, namespace, file, source, symbols, relationships, facts)
            }
            _ => {}
        }
    }
}

fn collect_field(
    node: Node<'_>,
    file: &str,
    source: &str,
    owner: &str,
    symbols: &mut Vec<Symbol>,
    relationships: &mut Vec<Relationship>,
) {
    let Some(declaration) = child_of_kind(node, "variable_declaration") else {
        return;
    };
    let declared = declared_type(declaration, source);

    for declarator in direct_children(declaration) {
        if declarator.kind() != "variable_declarator" {
            continue;
        }
        let Some(name_node) = direct_children(declarator)
            .into_iter()
            .find(|c| c.kind() == "identifier")
        else {
            continue;
        };
        let name = text(name_node, source).to_string();
        let id = make_symbol_id(file, &format!("{owner}::{name}"), &SymbolKind::Property);
        symbols.push(member_symbol(&id, &name, SymbolKind::Property, file, node, source));

        if let Some(declared) = &declared {
            relationships.push(reference(
                file,
                &id,
                declared,
                RelationshipKind::UsesType,
                "field type",
                line_of(node),
            ));
        }
    }
}

fn collect_property(
    node: Node<'_>,
    file: &str,
    source: &str,
    owner: &str,
    symbols: &mut Vec<Symbol>,
    relationships: &mut Vec<Relationship>,
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let name = text(name_node, source).to_string();
    let id = make_symbol_id(file, &format!("{owner}::{name}"), &SymbolKind::Property);
    symbols.push(member_symbol(&id, &name, SymbolKind::Property, file, node, source));

    if let Some(declared) = node.child_by_field_name("type").and_then(|t| type_name(t, source)) {
        relationships.push(reference(
            file,
            &id,
            &declared,
            RelationshipKind::UsesType,
            "property type",
            line_of(node),
        ));
    }
}

fn collect_method(
    node: Node<'_>,
    file: &str,
    source: &str,
    owner: &str,
    symbols: &mut Vec<Symbol>,
    relationships: &mut Vec<Relationship>,
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let name = text(name_node, source).to_string();
    let id = make_symbol_id(file, &format!("{owner}::{name}"), &SymbolKind::Method);
    symbols.push(member_symbol(&id, &name, SymbolKind::Method, file, node, source));

    if let Some(declared) = node.child_by_field_name("type").and_then(|t| type_name(t, source)) {
        relationships.push(reference(
            file,
            &id,
            &declared,
            RelationshipKind::UsesType,
            "return type",
            line_of(node),
        ));
    }
    if let Some(params) = child_of_kind(node, "parameter_list") {
        for param in direct_children(params) {
            if param.kind() != "parameter" {
                continue;
            }
            if let Some(declared) = param.child_by_field_name("type").and_then(|t| type_name(t, source)) {
                relationships.push(reference(
                    file,
                    &id,
                    &declared,
                    RelationshipKind::UsesType,
                    "parameter type",
                    line_of(param),
                ));
            }
        }
    }

    for created in descendants_of_kind(node, "object_creation_expression") {
        if let Some(target) = created.child_by_field_name("type").and_then(|t| type_name(t, source)) {
            relationships.push(reference(
                file,
                &id,
                &target,
                RelationshipKind::Instantiates,
                "instantiation",
                line_of(created),
            ));
        }
    }
}

fn member_symbol(
    id: &str,
    name: &str,
    kind: SymbolKind,
    file: &str,
    node: Node<'_>,
    source: &str,
) -> Symbol {
    Symbol {
        id: id.to_string(),
        name: name.to_string(),
        kind,
        language: Language::CSharp,
        file: file.to_string(),
        line_start: line_of(node),
        line_end: node.end_position().row as u32 + 1,
        signature: Some(first_line(text(node, source))),
    }
}

/// The declared type of a `variable_declaration`, which C# writes as the first
/// child rather than behind a field name.
pub fn declared_type(declaration: Node<'_>, source: &str) -> Option<String> {
    declaration
        .child_by_field_name("type")
        .and_then(|t| type_name(t, source))
        .or_else(|| {
            direct_children(declaration)
                .into_iter()
                .find(|c| matches!(c.kind(), "identifier" | "qualified_name" | "generic_name"))
                .map(|n| simple_name(text(n, source)))
        })
}

/// A type node's simple name, or `None` for anything the repository cannot
/// declare.
///
/// `predefined_type` covers `string`, `int` and the rest: they are keywords, no
/// repository declares them, and an edge to one could only be invented.
pub fn type_name(node: Node<'_>, source: &str) -> Option<String> {
    match node.kind() {
        "identifier" | "qualified_name" => Some(simple_name(text(node, source))),
        "generic_name" => child_of_kind(node, "identifier").map(|n| text(n, source).to_string()),
        "nullable_type" | "array_type" => node
            .child_by_field_name("type")
            .and_then(|inner| type_name(inner, source)),
        _ => None,
    }
}

/// `App.Util.Formatter` names `Formatter`.
pub fn simple_name(path: &str) -> String {
    path.rsplit('.').next().unwrap_or(path).to_string()
}

fn reference(
    file: &str,
    from: &str,
    target: &str,
    kind: RelationshipKind,
    context: &str,
    line: u32,
) -> Relationship {
    Relationship {
        from: from.to_string(),
        to: unresolved_local_type_id(target),
        kind,
        alias: None,
        properties_accessed: vec![],
        context: context.to_string(),
        file: file.to_string(),
        line,
        resolution: Resolution::default(),
    }
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or("").trim().to_string()
}

/// Every type declared in the file, as `(simple name, namespace)`.
///
/// Exposed for the resolver, which needs a file's own declarations before it
/// can decide what a bare name in that file refers to.
pub fn declared_index(facts: &FileFacts) -> BTreeMap<String, String> {
    facts.declared.iter().cloned().collect()
}
