//! Symbols and unresolved references, from one Java file.
//!
//! Everything here is within-file work. A reference to a type leaves this
//! module as a *placeholder* naming the simple identifier that was written —
//! `unresolved_local_type_id("UserService")` — because which `UserService` that
//! is depends on the file's package and imports, and those are
//! [`import_resolver`](super::import_resolver)'s business.
//!
//! Splitting it this way is what makes the resolution order testable on its
//! own. Java's rule is specific and has to be followed exactly, and a
//! resolver that also did extraction would make it much harder to see whether
//! it had been.

use openinvar_core::ir::{
    FileIR, Language, Relationship, RelationshipKind, Resolution, Symbol, SymbolKind,
};
use openinvar_core::symbol_id::{
    make_symbol_id, module_symbol, module_symbol_id, unresolved_local_type_id,
};
use std::collections::BTreeSet;

use tree_sitter::Node;

use super::parser::ParsedFile;

/// What a file says about where its names come from.
///
/// Carried beside the `FileIR` rather than inside it: this is input to
/// resolution, not part of the graph, and putting it in the IR would mean
/// serializing scaffolding into every snapshot.
#[derive(Debug, Default, Clone)]
pub struct FileFacts {
    pub file: String,
    /// `com.example.app`, or empty for the default package.
    pub package: String,
    /// Simple name to fully-qualified name, from single-type imports.
    pub imports: Vec<(String, String)>,
    /// Packages imported on demand — `import com.example.util.*`.
    pub wildcard_packages: Vec<String>,
    /// Simple member name to the fully-qualified type declaring it.
    pub static_imports: Vec<(String, String)>,
    /// Types declared in this file, by simple name.
    pub declared_types: Vec<String>,
}

pub fn text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    source.get(node.start_byte()..node.end_byte()).unwrap_or("")
}

fn line_of(node: Node<'_>) -> u32 {
    node.start_position().row as u32 + 1
}

/// A named child of `kind`, searching only direct children.
fn child_of_kind<'t>(node: Node<'t>, kind: &str) -> Option<Node<'t>> {
    children_of_kind(node, kind).into_iter().next()
}

fn children_of_kind<'t>(node: Node<'t>, kind: &str) -> Vec<Node<'t>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|c| c.kind() == kind)
        .collect()
}

pub fn extract(parsed: &ParsedFile) -> (FileIR, FileFacts) {
    let file = parsed.file.as_str();
    let source = parsed.source.as_str();
    let root = parsed.tree.root_node();

    let mut facts = FileFacts {
        file: file.to_string(),
        ..Default::default()
    };
    let mut symbols = vec![module_symbol(file, Language::Java)];
    let mut relationships = Vec::new();

    // ── package and imports ──────────────────────────────────
    if let Some(decl) = child_of_kind(root, "package_declaration") {
        if let Some(name) = child_of_kind(decl, "scoped_identifier").or_else(|| child_of_kind(decl, "identifier")) {
            facts.package = text(name, source).to_string();
        }
    }

    for decl in children_of_kind(root, "import_declaration") {
        let raw = text(decl, source);
        let is_static = raw.starts_with("import static");
        let is_wildcard = child_of_kind(decl, "asterisk").is_some();

        let Some(path_node) =
            child_of_kind(decl, "scoped_identifier").or_else(|| child_of_kind(decl, "identifier"))
        else {
            continue;
        };
        let path = text(path_node, source).to_string();

        if is_wildcard {
            // `import com.example.util.*` — on demand. Which names it brings
            // into scope is not knowable from this file alone; the resolver
            // answers that against the types it actually found.
            facts.wildcard_packages.push(path);
            continue;
        }

        let Some((owner, member)) = path.rsplit_once('.') else {
            continue;
        };

        if is_static {
            // `import static com.example.util.Helpers.log` — `log` now names a
            // member of `Helpers`, not a type.
            facts.static_imports.push((member.to_string(), owner.to_string()));
            continue;
        }

        facts.imports.push((member.to_string(), path.clone()));
        relationships.push(Relationship {
            from: module_symbol_id(file),
            to: openinvar_core::symbol_id::unresolved_import_id(owner, member),
            kind: RelationshipKind::Imports,
            alias: None,
            properties_accessed: vec![],
            context: "import".to_string(),
            file: file.to_string(),
            line: line_of(decl),
            resolution: Resolution::default(),
        });
    }

    // ── type declarations ────────────────────────────────────
    let mut cursor = root.walk();
    for node in root.children(&mut cursor) {
        collect_type(node, file, source, &mut symbols, &mut relationships, &mut facts);
    }

    (
        FileIR {
            file: file.to_string(),
            language: Language::Java,
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

const TYPE_KINDS: &[&str] = &[
    "class_declaration",
    "interface_declaration",
    "enum_declaration",
    "record_declaration",
    "annotation_type_declaration",
];

fn symbol_kind_for(node_kind: &str) -> SymbolKind {
    match node_kind {
        "interface_declaration" | "annotation_type_declaration" => SymbolKind::Interface,
        "enum_declaration" => SymbolKind::Enum,
        _ => SymbolKind::Class,
    }
}

/// Walk one type declaration, recursing into nested types.
fn collect_type(
    node: Node<'_>,
    file: &str,
    source: &str,
    symbols: &mut Vec<Symbol>,
    relationships: &mut Vec<Relationship>,
    facts: &mut FileFacts,
) {
    if !TYPE_KINDS.contains(&node.kind()) {
        // Types can be nested inside blocks and other declarations, so keep
        // descending rather than assuming they are all at the top level.
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect_type(child, file, source, symbols, relationships, facts);
        }
        return;
    }

    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let name = text(name_node, source).to_string();
    let kind = symbol_kind_for(node.kind());
    let id = make_symbol_id(file, &name, &kind);

    facts.declared_types.push(name.clone());
    symbols.push(Symbol {
        id: id.clone(),
        name: name.clone(),
        kind: kind.clone(),
        language: Language::Java,
        file: file.to_string(),
        line_start: line_of(node),
        line_end: node.end_position().row as u32 + 1,
        signature: Some(first_line(text(node, source))),
    });

    // extends / implements
    //
    // `class Box<T> extends Holder<T>` names `Holder` and `T`, and only the
    // first is a type this repository could contain.
    let generics = generic_parameter_names(node, source);
    if let Some(superclass) = child_of_kind(node, "superclass") {
        for target in type_references(superclass, source, &generics) {
            relationships.push(reference(
                file,
                &id,
                &target,
                RelationshipKind::Extends,
                "extends",
                line_of(superclass),
            ));
        }
    }
    // An interface extending interfaces uses `extends_interfaces`; a class
    // implementing them uses `super_interfaces`. Both mean the same edge here.
    for container in ["super_interfaces", "extends_interfaces"] {
        if let Some(supers) = child_of_kind(node, container) {
            for target in type_references(supers, source, &generics) {
                relationships.push(reference(
                    file,
                    &id,
                    &target,
                    RelationshipKind::Implements,
                    container,
                    line_of(supers),
                ));
            }
        }
    }

    let body = child_of_kind(node, "class_body")
        .or_else(|| child_of_kind(node, "interface_body"))
        .or_else(|| child_of_kind(node, "enum_body"))
        .or_else(|| child_of_kind(node, "annotation_type_body"));
    let Some(body) = body else { return };

    let mut cursor = body.walk();
    for member in body.children(&mut cursor) {
        match member.kind() {
            "field_declaration" => {
                collect_field(member, file, source, &name, symbols, relationships);
            }
            "method_declaration" | "constructor_declaration" | "compact_constructor_declaration" => {
                collect_method(member, file, source, &name, symbols, relationships);
            }
            k if TYPE_KINDS.contains(&k) => {
                collect_type(member, file, source, symbols, relationships, facts);
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
    // A field typed by the class's own `<T>` names no type this repository
    // could contain. `node.parent()` reaches the class body and then the class,
    // which is where the parameter was declared.
    let generics = generic_parameter_names(node, source);
    let declared = declared_type_name(node, source).filter(|d| !generics.contains(d));

    for declarator in children_of_kind(node, "variable_declarator") {
        let Some(name_node) = declarator.child_by_field_name("name") else {
            continue;
        };
        let name = text(name_node, source).to_string();
        let id = make_symbol_id(file, &format!("{owner}::{name}"), &SymbolKind::Property);
        symbols.push(Symbol {
            id: id.clone(),
            name: name.clone(),
            kind: SymbolKind::Property,
            language: Language::Java,
            file: file.to_string(),
            line_start: line_of(node),
            line_end: node.end_position().row as u32 + 1,
            signature: Some(first_line(text(node, source))),
        });

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

    symbols.push(Symbol {
        id: id.clone(),
        name: name.clone(),
        kind: SymbolKind::Method,
        language: Language::Java,
        file: file.to_string(),
        line_start: line_of(node),
        line_end: node.end_position().row as u32 + 1,
        signature: Some(first_line(text(node, source))),
    });

    // Return type and parameter types are references the method makes.
    // A method adds its own `<T>` to whatever its class declared.
    let generics = generic_parameter_names(node, source);
    if let Some(return_type) = node.child_by_field_name("type") {
        for target in type_references(return_type, source, &generics) {
            relationships.push(reference(
                file,
                &id,
                &target,
                RelationshipKind::UsesType,
                "return type",
                line_of(return_type),
            ));
        }
    }
    if let Some(params) = child_of_kind(node, "formal_parameters") {
        for param in children_of_kind(params, "formal_parameter") {
            if let Some(declared) =
                declared_type_name(param, source).filter(|d| !generics.contains(d))
            {
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

    // `new Foo()` anywhere in the body.
    if let Some(body) = child_of_kind(node, "block").or_else(|| child_of_kind(node, "constructor_body")) {
        for created in descendants_of_kind(body, "object_creation_expression") {
            for target in type_references(created, source, &generics) {
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
}

/// The declared type of a field, parameter or local variable.
///
/// `List<User>` yields `List`; the type argument is recorded separately by
/// [`type_identifiers`] where the caller wants it. Generic erasure is not
/// modelled, and claiming otherwise would be the kind of widened claim this
/// project documents rather than makes.
fn declared_type_name(node: Node<'_>, source: &str) -> Option<String> {
    let type_node = node.child_by_field_name("type")?;
    match type_node.kind() {
        "type_identifier" => Some(text(type_node, source).to_string()),
        "generic_type" => child_of_kind(type_node, "type_identifier")
            .map(|n| text(n, source).to_string()),
        "scoped_type_identifier" => text(type_node, source)
            .rsplit('.')
            .next()
            .map(|s| s.to_string()),
        // array_type, primitives, void: nothing a repository declares.
        "array_type" => type_node
            .child_by_field_name("element")
            .and_then(|e| (e.kind() == "type_identifier").then(|| text(e, source).to_string())),
        _ => None,
    }
}

/// Type-parameter names in scope at `node`.
///
/// `class Box<T>` and `<T> T unwrap()` both declare `T`, and both are in scope
/// for the method's body, so this walks outward from the node rather than
/// reading one declaration. A method inside a generic class sees the class's
/// parameters and its own.
///
/// Only the direct `type_identifier` child of each `type_parameter` is a name.
/// A bound — `<T extends Comparable>` — puts `Comparable` in a `type_bound`
/// child, and `Comparable` is a real type rather than a placeholder. Nothing
/// records that reference today, so sweeping the subtree would cost nothing
/// immediately; it is kept out anyway, so that teaching the extractor to read
/// bounds later does not run into a filter that silently eats them.
fn generic_parameter_names(node: Node<'_>, source: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut current = Some(node);
    while let Some(n) = current {
        if let Some(params) = n.child_by_field_name("type_parameters") {
            for param in children_of_kind(params, "type_parameter") {
                if let Some(name) = child_of_kind(param, "type_identifier") {
                    out.insert(text(name, source).to_string());
                }
            }
        }
        current = n.parent();
    }
    out
}

/// Every `type_identifier` under a node that is not a type parameter in scope.
fn type_references(node: Node<'_>, source: &str, generics: &BTreeSet<String>) -> Vec<String> {
    type_identifiers(node, source)
        .into_iter()
        .filter(|name| !generics.contains(name))
        .collect()
}

/// Every `type_identifier` under a node, in source order.
fn type_identifiers(node: Node<'_>, source: &str) -> Vec<String> {
    descendants_of_kind(node, "type_identifier")
        .into_iter()
        .map(|n| text(n, source).to_string())
        .collect()
}

pub fn descendants_of_kind<'t>(node: Node<'t>, kind: &str) -> Vec<Node<'t>> {
    let mut out = Vec::new();
    let mut stack = vec![node];
    let mut cursor = node.walk();
    while let Some(current) = stack.pop() {
        if current.kind() == kind && current.id() != node.id() {
            out.push(current);
        }
        for child in current.children(&mut cursor) {
            stack.push(child);
        }
    }
    // Source order, so two runs over one tree agree and a reader sees the
    // references in the order they appear.
    out.sort_by_key(|n| n.start_byte());
    out
}

fn reference(
    file: &str,
    from: &str,
    target_simple_name: &str,
    kind: RelationshipKind,
    context: &str,
    line: u32,
) -> Relationship {
    Relationship {
        from: from.to_string(),
        to: unresolved_local_type_id(target_simple_name),
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
