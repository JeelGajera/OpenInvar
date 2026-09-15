use openinvar_core::ast::{first_line_of, node_text, start_line, walk};
use openinvar_core::ir::{Diagnostic, DiagnosticCategory, DiagnosticLevel, FileIR, Language, ReExportEntry, Relationship, RelationshipKind, Resolution, Symbol, SymbolKind};
use openinvar_core::symbol_id::{
    make_symbol_id, module_symbol, module_symbol_id, unresolved_import_id,
    unresolved_local_type_id, IMPORT_ALL,
};
use tree_sitter::Node;

use crate::lang::python::framework::{classify, ClassRole};
use crate::lang::python::parser::ParsedFile;
use crate::lang::python::scope_analyzer::{base_type_name, collect_type_accesses};

pub fn extract_file_ir(parsed: &ParsedFile) -> FileIR {
    let source = parsed.source.as_bytes();
    let root = parsed.tree.root_node();
    let file = parsed.file.as_str();

    let mut symbols = vec![module_symbol(file, Language::Python)];
    let mut relationships = Vec::new();
    let mut re_exports = Vec::new();
    let mut diagnostics = parsed.diagnostics.clone();

    let stats = walk(root, &mut |node| match node.kind() {
        "class_definition" => {
            class_definition(node, source, file, &mut symbols, &mut relationships)
        }
        "function_definition" => {
            if let Some(sym) = function_definition(node, source, file) {
                symbols.push(sym);
            }
        }
        "import_statement" => relationships.extend(plain_import(node, source, file)),
        "import_from_statement" => relationships.extend(from_import(node, source, file)),
        "expression_statement" => {
            re_exports.extend(dunder_all(node, source));
            if let Some(sym) = module_constant(node, source, file) {
                symbols.push(sym);
            }
        }
        _ => {}
    });

    for (callee, line) in collect_direct_calls(root, source) {
        relationships.push(Relationship {
            from: module_symbol_id(file),
            to: unresolved_local_type_id(&callee),
            kind: RelationshipKind::Calls,
            alias: Some(callee),
            properties_accessed: Vec::new(),
            context: "call".to_string(),
            file: file.to_string(),
            line,
            resolution: Resolution::default(),
        });
    }

    for (type_name, access) in collect_type_accesses(root, source) {
        if access.properties.is_empty() {
            continue;
        }
        relationships.push(Relationship {
            from: module_symbol_id(file),
            to: unresolved_local_type_id(&type_name),
            kind: RelationshipKind::AccessesProperty,
            alias: Some(type_name),
            properties_accessed: access.properties.into_iter().collect(),
            context: "attribute access".to_string(),
            file: file.to_string(),
            line: access.first_line,
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

    // Counted from the tree this adapter already parsed, so the graph
    // and the count cannot disagree. Computed before the literal because
    // the literal moves `symbols`.
    let assertions =
        crate::assertions::count_by_symbol(&parsed.tree, &parsed.source, &Language::Python, &symbols);

    FileIR {
        file: parsed.file.clone(),
        language: Language::Python,
        symbols,
        relationships,
        diagnostics,
        re_exports,
        assertions,
        assertions_counted: crate::assertions::counts_assertions(&Language::Python),
        unbound_references: 0,
        references_counted: false,
    }
}

// ── calls ────────────────────────────────────────────────────

/// Direct calls whose callee is a bare name the file can resolve.
///
/// # Why only bare names
///
/// A call edge earns its place only if it says *which* function ran. `foo()`
/// where `foo` was imported or defined in this file names a symbol; a bare
/// `foo()` bound to nothing does not, and matching it by leaf name across the
/// repository is the bug 0.2.0 fixed in the Rust adapter. The resolver drops
/// what it cannot bind rather than pointing an edge at a name.
///
/// `obj.method()` is deliberately **not** a call edge, matching TypeScript: it
/// is already recorded as a property access on the receiver, which is the
/// honest statement — the receiver is touched at that line. This also excludes
/// `self.method()`, `super().__init__()` and `module.function()`, all of which
/// are attribute calls.
///
/// # Why the kind is decided later
///
/// Python does not distinguish construction from invocation at the call site:
/// `Foo()` and `foo()` are the same node. Nothing here can tell them apart,
/// and guessing from capitalization would be a convention, not a fact. So this
/// emits `Calls` for every one of them and the resolver promotes an edge to
/// `Instantiates` once the target turns out to be a class — a decision made
/// from the resolved symbol rather than from the spelling of the name.
///
/// Returns `(callee_name, line)`, sorted and deduplicated, so identical input
/// yields identical output.
fn collect_direct_calls(root: Node<'_>, source: &[u8]) -> Vec<(String, u32)> {
    let mut out: Vec<(String, u32)> = Vec::new();

    walk(root, &mut |node| {
        if node.kind() != "call" {
            return;
        }
        let Some(function) = node.child_by_field_name("function") else {
            return;
        };
        if function.kind() != "identifier" {
            return;
        }
        let Some(name) = node_text(function, source) else {
            return;
        };
        if name.is_empty() {
            return;
        }
        out.push((name.to_string(), start_line(function)));
    });

    // One row per (callee, line): a name called twice on one line is one fact
    // about that line, and the order must not depend on traversal.
    out.sort();
    out.dedup();
    out
}

// ── symbols ──────────────────────────────────────────────────

fn class_definition(
    node: Node<'_>,
    source: &[u8],
    file: &str,
    symbols: &mut Vec<Symbol>,
    relationships: &mut Vec<Relationship>,
) {
    let Some(name) = node
        .child_by_field_name("name")
        .and_then(|n| node_text(n, source))
    else {
        return;
    };

    let bases = base_class_names(node, source);
    let decorators = decorator_names(node, source);

    let kind = match classify(&bases, &decorators) {
        ClassRole::Interface => SymbolKind::Interface,
        // A model's fields are its contract, but it is still a class; the
        // distinction drives nothing downstream, so it stays a Class.
        ClassRole::Model | ClassRole::Plain => SymbolKind::Class,
    };

    symbols.push(symbol(file, name, name, kind.clone(), node, source));

    // Inheritance is a real dependency: changing a base class's fields or
    // methods changes every subclass. The dotted form is kept intact — the
    // resolver needs the `models` in `models.Model` to find the package it
    // came from.
    for base in &bases {
        relationships.push(Relationship {
            from: make_symbol_id(file, name, &kind),
            to: unresolved_local_type_id(base),
            kind: RelationshipKind::Extends,
            alias: Some(base.clone()),
            properties_accessed: Vec::new(),
            context: format!("class {name}({base})"),
            file: file.to_string(),
            line: start_line(node),
            resolution: Resolution::default(),
        });
    }

    // Class-level annotated fields — the model contract.
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for statement in body.children(&mut cursor) {
            let Some(field) = annotated_field(statement, source) else {
                continue;
            };
            symbols.push(Symbol {
                id: make_symbol_id(file, &format!("{name}::{field}"), &SymbolKind::Property),
                name: field,
                kind: SymbolKind::Property,
                language: Language::Python,
                file: file.to_string(),
                line_start: start_line(statement),
                line_end: statement.end_position().row as u32 + 1,
                signature: first_line_of(statement, source),
            });
        }
    }
}

/// `user_id: str` at class level.
fn annotated_field(statement: Node<'_>, source: &[u8]) -> Option<String> {
    if statement.kind() != "expression_statement" {
        return None;
    }
    let assignment = statement.named_child(0)?;
    if assignment.kind() != "assignment" {
        return None;
    }
    assignment.child_by_field_name("type")?;
    let target = assignment.child_by_field_name("left")?;
    if target.kind() != "identifier" {
        return None;
    }
    node_text(target, source).map(str::to_string)
}

fn base_class_names(node: Node<'_>, source: &[u8]) -> Vec<String> {
    let Some(args) = node.child_by_field_name("superclasses") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut cursor = args.walk();
    for arg in args.named_children(&mut cursor) {
        // Skip `metaclass=...` and similar keyword arguments.
        if arg.kind() == "keyword_argument" {
            continue;
        }
        // Keep the source spelling: `models.Model` carries the qualifier the
        // resolver needs, while `base_type_name` would reduce it to `Model`.
        if let Some(name) = node_text(arg, source)
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty() && !t.contains(|c: char| c.is_whitespace() || c == '('))
            .or_else(|| base_type_name(arg, source))
        {
            out.push(name);
        }
    }
    out
}

fn decorator_names(node: Node<'_>, source: &[u8]) -> Vec<String> {
    let Some(parent) = node.parent() else {
        return Vec::new();
    };
    if parent.kind() != "decorated_definition" {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut cursor = parent.walk();
    for child in parent.children(&mut cursor) {
        if child.kind() != "decorator" {
            continue;
        }
        let Some(text) = node_text(child, source) else {
            continue;
        };
        // `@dataclass`, `@dataclass(frozen=True)`, `@app.get("/x")`
        let name = text
            .trim_start_matches('@')
            .split('(')
            .next()
            .unwrap_or("")
            .trim();
        if !name.is_empty() {
            out.push(name.to_string());
        }
    }
    out
}

fn function_definition(node: Node<'_>, source: &[u8], file: &str) -> Option<Symbol> {
    let name = node
        .child_by_field_name("name")
        .and_then(|n| node_text(n, source))?;

    // A function whose grandparent is a class is a method.
    let owner = node.parent().and_then(|block| {
        (block.kind() == "block")
            .then(|| block.parent())
            .flatten()
            .filter(|p| p.kind() == "class_definition")
            .and_then(|c| c.child_by_field_name("name"))
            .and_then(|n| node_text(n, source))
    });

    let (kind, id_name) = match owner {
        Some(class) => (SymbolKind::Method, format!("{class}::{name}")),
        None => (SymbolKind::Function, name.to_string()),
    };

    Some(symbol(file, &id_name, name, kind, node, source))
}

/// A module-level `NAME = value` constant.
fn module_constant(node: Node<'_>, source: &[u8], file: &str) -> Option<Symbol> {
    if node.parent().map(|p| p.kind()) != Some("module") {
        return None;
    }
    let assignment = node.named_child(0)?;
    if assignment.kind() != "assignment" {
        return None;
    }
    let target = assignment.child_by_field_name("left")?;
    if target.kind() != "identifier" {
        return None;
    }
    let name = node_text(target, source)?;
    // `__all__` is re-export metadata, handled separately.
    if name.starts_with("__") {
        return None;
    }
    Some(symbol(file, name, name, SymbolKind::Variable, node, source))
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
        language: Language::Python,
        file: file.to_string(),
        line_start: start_line(node),
        line_end: node.end_position().row as u32 + 1,
        signature: first_line_of(node, source),
    }
}

// ── imports ──────────────────────────────────────────────────

/// `import a.b.c` / `import a.b as ab`
fn plain_import(node: Node<'_>, source: &[u8], file: &str) -> Vec<Relationship> {
    let context = first_line_of(node, source).unwrap_or_default();
    let line = start_line(node);

    let mut out = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        let (module, alias) = match child.kind() {
            "dotted_name" => (node_text(child, source), None),
            "aliased_import" => (
                child
                    .child_by_field_name("name")
                    .and_then(|n| node_text(n, source)),
                child
                    .child_by_field_name("alias")
                    .and_then(|a| node_text(a, source)),
            ),
            _ => continue,
        };
        let Some(module) = module else { continue };

        out.push(Relationship {
            from: module_symbol_id(file),
            to: unresolved_import_id(module, IMPORT_ALL),
            kind: RelationshipKind::Imports,
            alias: alias.map(str::to_string),
            properties_accessed: Vec::new(),
            context: context.clone(),
            file: file.to_string(),
            line,
            resolution: Resolution::default(),
        });
    }
    out
}

/// `from .mod import A, B as C` / `from .mod import *`
fn from_import(node: Node<'_>, source: &[u8], file: &str) -> Vec<Relationship> {
    let context = first_line_of(node, source).unwrap_or_default();
    let line = start_line(node);

    let Some(module_node) = node.child_by_field_name("module_name") else {
        return Vec::new();
    };
    // Relative imports keep their leading dots; the resolver interprets them
    // against the importing file's package.
    let module = node_text(module_node, source).unwrap_or("").to_string();

    let mut out = Vec::new();
    let mut push = |symbol: &str, alias: Option<&str>| {
        out.push(Relationship {
            from: module_symbol_id(file),
            to: unresolved_import_id(&module, symbol),
            kind: RelationshipKind::Imports,
            alias: alias.map(str::to_string),
            properties_accessed: Vec::new(),
            context: context.clone(),
            file: file.to_string(),
            line,
            resolution: Resolution::default(),
        });
    };

    let mut cursor = node.walk();
    let mut saw_name = false;
    for child in node.named_children(&mut cursor) {
        if child.id() == module_node.id() {
            continue;
        }
        match child.kind() {
            "wildcard_import" => {
                push(IMPORT_ALL, None);
                saw_name = true;
            }
            "dotted_name" | "identifier" => {
                if let Some(name) = node_text(child, source) {
                    push(name, None);
                    saw_name = true;
                }
            }
            "aliased_import" => {
                let name = child
                    .child_by_field_name("name")
                    .and_then(|n| node_text(n, source));
                let alias = child
                    .child_by_field_name("alias")
                    .and_then(|a| node_text(a, source));
                if let Some(name) = name {
                    push(name, alias);
                    saw_name = true;
                }
            }
            _ => {}
        }
    }

    // `from . import x` with no recognised names still imports the package.
    if !saw_name {
        push(IMPORT_ALL, None);
    }

    out
}

/// `__all__ = ["a", "b"]` — the module's declared public surface.
fn dunder_all(node: Node<'_>, source: &[u8]) -> Vec<ReExportEntry> {
    let Some(assignment) = node.named_child(0) else {
        return Vec::new();
    };
    if assignment.kind() != "assignment" {
        return Vec::new();
    }
    let is_dunder_all = assignment
        .child_by_field_name("left")
        .and_then(|l| node_text(l, source))
        .map(|name| name == "__all__")
        .unwrap_or(false);
    if !is_dunder_all {
        return Vec::new();
    }
    let Some(value) = assignment.child_by_field_name("right") else {
        return Vec::new();
    };

    let mut out = Vec::new();
    let mut cursor = value.walk();
    for element in value.named_children(&mut cursor) {
        if element.kind() != "string" {
            continue;
        }
        let Some(text) = node_text(element, source) else {
            continue;
        };
        let name = text.trim_matches(|c| c == '"' || c == '\'');
        if !name.is_empty() {
            out.push(ReExportEntry {
                exported_name: name.to_string(),
                source_module: ".".to_string(),
            });
        }
    }
    out
}
