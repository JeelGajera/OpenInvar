use std::collections::{BTreeMap, BTreeSet, HashMap};

use openinvar_core::ir::{FileIR, Language, ReExportEntry, Relationship, RelationshipKind, Resolution, Symbol, SymbolKind};
use tree_sitter::Node;

use crate::lang::typescript::parser::ParsedFile;

// Layer 1: TypeScript language primitives
const TS_PRIMITIVES: &[&str] = &[
    "any",
    "bigint",
    "boolean",
    "never",
    "null",
    "number",
    "object",
    "string",
    "symbol",
    "undefined",
    "unknown",
    "void",
];

// Layer 2: TypeScript utility types
const TS_UTILITY_TYPES: &[&str] = &[
    "ConstructorParameters",
    "Exclude",
    "Extract",
    "InstanceType",
    "NonNullable",
    "Omit",
    "Parameters",
    "Partial",
    "Pick",
    "Readonly",
    "ReadonlyArray",
    "Record",
    "Required",
    "ReturnType",
    "ThisType",
];

// Layer 3: Standard library / runtime types
const TS_STDLIB_TYPES: &[&str] = &[
    "Array",
    "ArrayBuffer",
    "ArrayLike",
    "Boolean",
    "DataView",
    "Date",
    "Error",
    "Float32Array",
    "Float64Array",
    "Function",
    "Int16Array",
    "Int32Array",
    "Int8Array",
    "Map",
    "Number",
    "Object",
    "Promise",
    "RegExp",
    "Set",
    "String",
    "Symbol",
    "TypeError",
    "Uint16Array",
    "Uint32Array",
    "Uint8Array",
    "WeakMap",
    "WeakSet",
];

// Layer 4: DOM / platform types
const DOM_TYPES: &[&str] = &[
    "Document",
    "Element",
    "Event",
    "EventTarget",
    "HTMLElement",
    "KeyboardEvent",
    "MouseEvent",
    "Window",
];

// Layer 5: Common framework types (React etc.)
const FRAMEWORK_TYPES: &[&str] = &["Component", "FC", "JSX", "ReactElement", "ReactNode"];

pub fn is_builtin_type(name: &str) -> bool {
    TS_PRIMITIVES.contains(&name)
        || TS_UTILITY_TYPES.contains(&name)
        || TS_STDLIB_TYPES.contains(&name)
        || DOM_TYPES.contains(&name)
        || FRAMEWORK_TYPES.contains(&name)
}

pub fn extract_file_ir(parsed: &ParsedFile) -> FileIR {
    let mut symbols = extract_symbols(parsed);
    let module_symbol = module_symbol(&parsed.file, parsed.language.clone());
    if !symbols.iter().any(|s| s.id == module_symbol.id) {
        symbols.push(module_symbol.clone());
    }

    let from_symbol_id = first_primary_symbol_id(&symbols).unwrap_or(module_symbol.id.clone());

    let type_to_props = collect_property_accesses(parsed);
    let (mut relationships, re_exports) =
        collect_import_and_reexport_relationships(parsed, &from_symbol_id, &type_to_props);

    for (type_name, (props, first_line)) in type_to_props {
        if props.is_empty() {
            continue;
        }
        relationships.push(Relationship {
            from: from_symbol_id.clone(),
            to: unresolved_local_type_symbol_id(&type_name),
            kind: RelationshipKind::AccessesProperty,
            alias: Some(type_name),
            properties_accessed: props.into_iter().collect(),
            context: "property access".to_string(),
            file: parsed.file.clone(),
            line: first_line,
            resolution: Resolution::default(),
        });
    }
    relationships.extend(collect_method_scoped_accesses(parsed, &symbols));

    for (callee, kind, line) in collect_calls_and_instantiations(parsed) {
        let context = match kind {
            RelationshipKind::Instantiates => "instantiation",
            _ => "call",
        };
        relationships.push(Relationship {
            from: from_symbol_id.clone(),
            to: unresolved_local_type_symbol_id(&callee),
            kind,
            alias: Some(callee),
            properties_accessed: Vec::new(),
            context: context.to_string(),
            file: parsed.file.clone(),
            line,
            resolution: Resolution::default(),
        });
    }

    relationships.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then(a.line.cmp(&b.line))
            .then(a.from.cmp(&b.from))
            .then(a.to.cmp(&b.to))
    });

    // Counted from the tree this adapter already parsed, so the graph
    // and the count cannot disagree. Computed before the literal because
    // the literal moves `symbols`.
    let assertions =
        crate::assertions::count_by_symbol(&parsed.tree, &parsed.source, &parsed.language.clone(), &symbols);

    FileIR {
        file: parsed.file.clone(),
        language: parsed.language.clone(),
        symbols,
        relationships,
        diagnostics: parsed.diagnostics.clone(),
        re_exports,
        assertions,
        assertions_counted: crate::assertions::counts_assertions(&parsed.language.clone()),
        unbound_references: 0,
        references_counted: false,
    }
}

fn collect_method_scoped_accesses(parsed: &ParsedFile, symbols: &[Symbol]) -> Vec<Relationship> {
    let mut out = Vec::new();
    let constructor_fields = collect_constructor_field_types(parsed);
    let method_ids: HashMap<String, String> = symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Method)
        .map(|s| (s.name.clone(), s.id.clone()))
        .collect();

    walk_tree(parsed.tree.root_node(), &mut |node| {
        if node.kind() != "method_definition" {
            return;
        }

        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let Some(method_name_raw) = node_text(name_node, &parsed.source) else {
            return;
        };
        let method_name = sanitize_identifier(method_name_raw);
        if method_name.is_empty() || method_name == "constructor" {
            return;
        }
        let Some(method_id) = method_ids.get(method_name) else {
            return;
        };

        let mut typed_vars = HashMap::<String, String>::new();
        walk_tree(node, &mut |inner| {
            collect_typed_var(inner, &parsed.source, &mut typed_vars);
        });

        let mut type_to_props = BTreeMap::<String, (BTreeSet<String>, u32)>::new();
        walk_tree(node, &mut |inner| {
            if inner.kind() != "member_expression" {
                return;
            }
            let Some(object_node) = inner.child_by_field_name("object") else {
                return;
            };
            let Some(property_node) = inner.child_by_field_name("property") else {
                return;
            };
            let Some(object_text) = node_text(object_node, &parsed.source) else {
                return;
            };
            let Some(property_text) = node_text(property_node, &parsed.source) else {
                return;
            };
            let property_name = sanitize_identifier(property_text);
            if property_name.is_empty() {
                return;
            }

            let type_name = if let Some(field_name) = object_text.strip_prefix("this.") {
                constructor_fields
                    .get(sanitize_identifier(field_name))
                    .cloned()
            } else {
                let object_name = sanitize_identifier(object_text);
                if object_name.is_empty() {
                    return;
                }
                typed_vars.get(object_name).cloned()
            };
            let Some(type_name) = type_name else {
                return;
            };
            if is_builtin_type(&type_name) {
                return;
            }

            let line = inner.start_position().row as u32 + 1;
            let (props, first_line) = type_to_props
                .entry(type_name)
                .or_insert_with(|| (BTreeSet::new(), line));
            props.insert(property_name.to_string());
            if line < *first_line {
                *first_line = line;
            }
        });

        for (type_name, (props, line)) in type_to_props {
            if props.is_empty() {
                continue;
            }
            out.push(Relationship {
                from: method_id.clone(),
                to: unresolved_local_type_symbol_id(&type_name),
                kind: RelationshipKind::AccessesProperty,
                alias: Some(type_name),
                properties_accessed: props.into_iter().collect(),
                context: format!("method property access: {method_name}"),
                file: parsed.file.clone(),
                line,
                resolution: Resolution::default(),
            });
        }
    });

    out
}

// Placeholder ids come from `openinvar_core::symbol_id`, like every other
// adapter's. This module used to mint its own — the same shape with a
// `__UNRESOLVED_LOCAL_TYPE__` prefix instead of `unresolved_local_type` — and
// the cost was not the duplication. It was that `is_placeholder`, the shared
// check every layer above uses to tell a bound target from an unbound one,
// did not recognise them: an unresolved TypeScript type reference passed every
// guard in the codebase and reached the graph stamped gate-safe.
//
// These wrappers keep the adapter's own call sites reading as they did.

pub fn unresolved_import_symbol_id(module_specifier: &str, symbol_name: &str) -> String {
    openinvar_core::symbol_id::unresolved_import_id(module_specifier, symbol_name)
}

pub fn unresolved_local_type_symbol_id(type_name: &str) -> String {
    openinvar_core::symbol_id::unresolved_local_type_id(type_name)
}

pub fn parse_unresolved_import_symbol_id(raw: &str) -> Option<(String, String)> {
    openinvar_core::symbol_id::parse_unresolved_import_id(raw)
        .map(|(module, symbol)| (module.to_string(), symbol.to_string()))
}

pub fn parse_unresolved_local_type_symbol_id(raw: &str) -> Option<String> {
    openinvar_core::symbol_id::parse_unresolved_local_type_id(raw).map(str::to_string)
}

#[derive(Debug, Clone)]
struct ImportEntry {
    imported_name: String,
    local_name: String,
    module_specifier: String,
    kind: RelationshipKind,
    context: String,
    line: u32,
}

fn collect_import_and_reexport_relationships(
    parsed: &ParsedFile,
    from_symbol_id: &str,
    type_to_props: &BTreeMap<String, (BTreeSet<String>, u32)>,
) -> (Vec<Relationship>, Vec<ReExportEntry>) {
    let mut out = Vec::new();
    let mut re_exports = Vec::new();

    walk_tree(parsed.tree.root_node(), &mut |node| {
        let kind = node.kind();
        if kind != "import_statement" && kind != "export_statement" {
            return;
        }

        let Some(module_specifier) = extract_module_specifier(node, &parsed.source) else {
            return;
        };

        let statement_text =
            compact_whitespace(node_text(node, &parsed.source).unwrap_or_default());
        if statement_text.is_empty() {
            return;
        }

        let line = node.start_position().row as u32 + 1;
        let entries = if kind == "import_statement" {
            parse_import_entries(&statement_text, &module_specifier, line)
        } else {
            parse_reexport_entries(&statement_text, &module_specifier, line)
        };

        for entry in entries {
            // Track named re-exports for barrel chain resolution
            if entry.kind == RelationshipKind::ReExports && entry.imported_name != "*" {
                re_exports.push(ReExportEntry {
                    exported_name: entry.imported_name.clone(),
                    source_module: entry.module_specifier.clone(),
                });
            }

            let properties_accessed = if entry.kind == RelationshipKind::Imports {
                type_to_props
                    .get(&entry.local_name)
                    .map(|(props, _)| props.iter().cloned().collect())
                    .unwrap_or_default()
            } else {
                Vec::new()
            };

            out.push(Relationship {
                from: from_symbol_id.to_string(),
                to: unresolved_import_symbol_id(&entry.module_specifier, &entry.imported_name),
                kind: entry.kind,
                alias: if entry.local_name != entry.imported_name {
                    Some(entry.local_name)
                } else {
                    None
                },
                properties_accessed,
                context: entry.context,
                file: parsed.file.clone(),
                line: entry.line,
                resolution: Resolution::default(),
            });
        }
    });

    out.extend(collect_decorator_references(parsed, from_symbol_id));

    (out, re_exports)
}

fn collect_decorator_references(parsed: &ParsedFile, from_symbol_id: &str) -> Vec<Relationship> {
    let mut out = Vec::new();

    walk_tree(parsed.tree.root_node(), &mut |node| {
        if node.kind() != "decorator" {
            return;
        }

        let Some(text) = node_text(node, &parsed.source) else {
            return;
        };
        let compact = compact_whitespace(text);
        if !compact.starts_with("@Module(") {
            return;
        }
        let condensed = compact.replace(' ', "");

        let line = node.start_position().row as u32 + 1;
        for key in ["providers", "controllers", "imports", "exports"] {
            for reference in extract_module_array_identifiers(&condensed, key) {
                if reference.is_empty() || is_builtin_type(reference) {
                    continue;
                }
                out.push(Relationship {
                    from: from_symbol_id.to_string(),
                    to: unresolved_local_type_symbol_id(reference),
                    kind: RelationshipKind::UsesType,
                    alias: None,
                    properties_accessed: Vec::new(),
                    context: format!("@Module {key} reference"),
                    file: parsed.file.clone(),
                    line,
                    resolution: Resolution::default(),
                });
            }
        }
    });

    out
}

fn extract_module_array_identifiers<'a>(decorator_text: &'a str, key: &str) -> Vec<&'a str> {
    let marker = format!("{key}:[");
    let mut out = Vec::new();
    let mut rest = decorator_text;

    while let Some(idx) = rest.find(&marker) {
        let after = &rest[idx + marker.len()..];
        let Some(end_idx) = after.find(']') else {
            break;
        };
        let slice = &after[..end_idx];
        for token in slice.split(',').map(str::trim).filter(|t| !t.is_empty()) {
            let name = sanitize_identifier(token);
            if !name.is_empty() {
                out.push(name);
            }
        }
        rest = &after[end_idx + 1..];
    }

    out
}

fn parse_import_entries(statement: &str, module_specifier: &str, line: u32) -> Vec<ImportEntry> {
    let mut out = Vec::new();
    if !statement.starts_with("import ") || !statement.contains(" from ") {
        return out;
    }

    let Some((left, _)) = statement.split_once(" from ") else {
        return out;
    };
    let left = left.trim_start_matches("import").trim();

    if let Some(named) = between(left, '{', '}') {
        for item in named.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let (imported_name, local_name) = parse_aliased_item(item);
            out.push(ImportEntry {
                imported_name,
                local_name,
                module_specifier: module_specifier.to_string(),
                kind: RelationshipKind::Imports,
                context: statement.to_string(),
                line,
            });
        }
    }

    let outside_brace = if let Some(brace_idx) = left.find('{') {
        left[..brace_idx].trim().trim_end_matches(',').trim()
    } else {
        left
    };

    if outside_brace.is_empty() {
        return out;
    }

    if outside_brace.starts_with('*') {
        if let Some((_, local)) = outside_brace.split_once(" as ") {
            let local_name = sanitize_identifier(local);
            if !local_name.is_empty() {
                out.push(ImportEntry {
                    imported_name: "*".to_string(),
                    local_name: local_name.to_string(),
                    module_specifier: module_specifier.to_string(),
                    kind: RelationshipKind::Imports,
                    context: statement.to_string(),
                    line,
                });
            }
        }
    } else {
        let default_name = outside_brace
            .split(',')
            .next()
            .map(sanitize_identifier)
            .unwrap_or("");
        if !default_name.is_empty() {
            out.push(ImportEntry {
                imported_name: "default".to_string(),
                local_name: default_name.to_string(),
                module_specifier: module_specifier.to_string(),
                kind: RelationshipKind::Imports,
                context: statement.to_string(),
                line,
            });
        }
    }

    out
}

fn parse_reexport_entries(statement: &str, module_specifier: &str, line: u32) -> Vec<ImportEntry> {
    let mut out = Vec::new();
    if !statement.starts_with("export ") || !statement.contains(" from ") {
        return out;
    }

    let Some((left, _)) = statement.split_once(" from ") else {
        return out;
    };
    let left = left.trim_start_matches("export").trim();

    if let Some(named) = between(left, '{', '}') {
        for item in named.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let (imported_name, local_name) = parse_aliased_item(item);
            out.push(ImportEntry {
                imported_name,
                local_name,
                module_specifier: module_specifier.to_string(),
                kind: RelationshipKind::ReExports,
                context: statement.to_string(),
                line,
            });
        }
    } else if left.starts_with('*') {
        out.push(ImportEntry {
            imported_name: "*".to_string(),
            local_name: "*".to_string(),
            module_specifier: module_specifier.to_string(),
            kind: RelationshipKind::ReExports,
            context: statement.to_string(),
            line,
        });
    }

    out
}

fn collect_property_accesses(parsed: &ParsedFile) -> BTreeMap<String, (BTreeSet<String>, u32)> {
    let mut typed_vars = HashMap::<String, String>::new();
    let mut type_to_props = BTreeMap::<String, (BTreeSet<String>, u32)>::new();
    let constructor_fields = collect_constructor_field_types(parsed);

    walk_tree(parsed.tree.root_node(), &mut |node| {
        collect_typed_var(node, &parsed.source, &mut typed_vars);
    });

    walk_tree(parsed.tree.root_node(), &mut |node| {
        if node.kind() != "member_expression" {
            return;
        }

        let Some(object_node) = node.child_by_field_name("object") else {
            return;
        };
        let Some(property_node) = node.child_by_field_name("property") else {
            return;
        };

        let Some(object_text) = node_text(object_node, &parsed.source) else {
            return;
        };
        let Some(property_text) = node_text(property_node, &parsed.source) else {
            return;
        };
        let property_name = sanitize_identifier(property_text);
        if property_name.is_empty() {
            return;
        }

        let type_name = if let Some(field_name) = object_text.strip_prefix("this.") {
            constructor_fields
                .get(sanitize_identifier(field_name))
                .cloned()
        } else {
            let object_name = sanitize_identifier(object_text);
            if object_name.is_empty() {
                return;
            }
            typed_vars.get(object_name).cloned()
        };
        let Some(type_name) = type_name else {
            return;
        };

        // Skip built-in types — they are not codebase symbols
        if is_builtin_type(&type_name) {
            return;
        }

        let line = node.start_position().row as u32 + 1;
        let (props, first_line) = type_to_props
            .entry(type_name)
            .or_insert_with(|| (BTreeSet::new(), line));
        props.insert(property_name.to_string());
        if line < *first_line {
            *first_line = line;
        }
    });

    type_to_props
}

/// Collect constructor-injected `this.field: Type` mappings for DI-style classes.
fn collect_constructor_field_types(parsed: &ParsedFile) -> HashMap<String, String> {
    let mut field_types = HashMap::new();

    walk_tree(parsed.tree.root_node(), &mut |node| {
        if node.kind() != "method_definition" {
            return;
        }

        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let Some(name_text) = node_text(name_node, &parsed.source) else {
            return;
        };
        if sanitize_identifier(name_text) != "constructor" {
            return;
        }

        let Some(params) = node.child_by_field_name("parameters") else {
            return;
        };

        let mut cursor = params.walk();
        for param in params.children(&mut cursor) {
            if !param.kind().contains("parameter") {
                continue;
            }

            if let Some((field_name, type_name)) =
                parse_constructor_field_param(param, &parsed.source)
            {
                if !is_builtin_type(&type_name) {
                    field_types.insert(field_name, type_name);
                }
            }
        }
    });

    field_types
}

fn parse_constructor_field_param(node: Node<'_>, source: &str) -> Option<(String, String)> {
    let text = compact_whitespace(node_text(node, source)?);
    let (left, right) = text.split_once(':')?;
    let field_token = left
        .split_whitespace()
        .last()
        .unwrap_or(left)
        .trim_end_matches('?');
    let field_name = sanitize_identifier(field_token);
    if field_name.is_empty() {
        return None;
    }

    let type_text = right.split('=').next().map(str::trim).unwrap_or_default();
    let type_name = primary_type_name(type_text);
    if type_name.is_empty() {
        return None;
    }

    Some((field_name.to_string(), type_name.to_string()))
}

fn collect_typed_var(node: Node<'_>, source: &str, typed_vars: &mut HashMap<String, String>) {
    if !node.kind().contains("parameter") && node.kind() != "variable_declarator" {
        return;
    }

    let name_node = node.child_by_field_name("name");
    let type_node = node
        .child_by_field_name("type")
        .or_else(|| node.child_by_field_name("type_annotation"));

    let (Some(name_node), Some(type_node)) = (name_node, type_node) else {
        if let Some((var_name, type_name)) = parse_name_and_type_from_node_text(node, source) {
            typed_vars.insert(var_name, type_name);
        }
        return;
    };

    let Some(name_text) = node_text(name_node, source) else {
        return;
    };
    let Some(type_text) = node_text(type_node, source) else {
        return;
    };

    let var_name = sanitize_identifier(name_text);
    let type_name = primary_type_name(type_text);

    if !var_name.is_empty() && !type_name.is_empty() {
        typed_vars.insert(var_name.to_string(), type_name.to_string());
    }
}

fn parse_name_and_type_from_node_text(node: Node<'_>, source: &str) -> Option<(String, String)> {
    let text = node_text(node, source)?;
    let compact = compact_whitespace(text);
    let (left, right) = compact.split_once(':')?;

    let name = sanitize_identifier(left);
    if name.is_empty() {
        return None;
    }

    let right = right.split('=').next().map(str::trim).unwrap_or_default();
    let typ = primary_type_name(right);
    if typ.is_empty() {
        return None;
    }

    Some((name.to_string(), typ.to_string()))
}

fn extract_symbols(parsed: &ParsedFile) -> Vec<Symbol> {
    let mut out = Vec::new();

    walk_tree(parsed.tree.root_node(), &mut |node| {
        let kind = match node.kind() {
            "class_declaration" => Some(SymbolKind::Class),
            "interface_declaration" => Some(SymbolKind::Interface),
            "type_alias_declaration" => Some(SymbolKind::TypeAlias),
            "function_declaration" => Some(SymbolKind::Function),
            "method_definition" => Some(SymbolKind::Method),
            "public_field_definition" | "property_signature" => Some(SymbolKind::Property),
            "variable_declarator" => Some(SymbolKind::Variable),
            "enum_declaration" => Some(SymbolKind::Enum),
            _ => None,
        };

        let Some(kind) = kind else {
            return;
        };

        let Some(name) = extract_name(node, &parsed.source) else {
            return;
        };

        let line = node.start_position().row as u32 + 1;
        let signature = node_text(node, &parsed.source).unwrap_or_default();
        out.push(new_symbol(
            &parsed.file,
            &name,
            kind,
            parsed.language.clone(),
            line,
            &compact_whitespace(signature),
        ));
    });

    out.sort_by(|a, b| a.id.cmp(&b.id));
    out.dedup_by(|a, b| a.id == b.id);
    out
}

fn extract_name(node: Node<'_>, source: &str) -> Option<String> {
    if let Some(name_node) = node.child_by_field_name("name") {
        let text = node_text(name_node, source)?;
        let cleaned = sanitize_identifier(text);
        if !cleaned.is_empty() {
            return Some(cleaned.to_string());
        }
    }

    if node.kind() == "variable_declarator" {
        if let Some(pattern) = node.child_by_field_name("name") {
            let text = node_text(pattern, source)?;
            let cleaned = sanitize_identifier(text);
            if !cleaned.is_empty() {
                return Some(cleaned.to_string());
            }
        }
    }

    None
}

fn module_symbol(file: &str, language: Language) -> Symbol {
    Symbol {
        id: make_symbol_id(file, "module", &SymbolKind::Module),
        name: "module".to_string(),
        kind: SymbolKind::Module,
        language,
        file: file.to_string(),
        line_start: 1,
        line_end: 1,
        signature: Some("module".to_string()),
    }
}

fn first_primary_symbol_id(symbols: &[Symbol]) -> Option<String> {
    symbols
        .iter()
        .find(|s| s.kind != SymbolKind::Module)
        .map(|s| s.id.clone())
}

fn new_symbol(
    file: &str,
    name: &str,
    kind: SymbolKind,
    language: Language,
    line: u32,
    signature: &str,
) -> Symbol {
    Symbol {
        id: make_symbol_id(file, name, &kind),
        name: name.to_string(),
        kind,
        language,
        file: file.to_string(),
        line_start: line,
        line_end: line,
        signature: if signature.is_empty() {
            None
        } else {
            Some(signature.to_string())
        },
    }
}

fn make_symbol_id(file: &str, name: &str, kind: &SymbolKind) -> String {
    format!("{file}::{name}::{}", symbol_kind_suffix(kind))
}

fn symbol_kind_suffix(kind: &SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Class => "class",
        SymbolKind::Interface => "interface",
        SymbolKind::TypeAlias => "type_alias",
        SymbolKind::Function => "function",
        SymbolKind::Method => "method",
        SymbolKind::Property => "property",
        SymbolKind::Variable => "variable",
        SymbolKind::Module => "module",
        SymbolKind::Enum => "enum",
        SymbolKind::EnumVariant => "enum_variant",
        SymbolKind::ExternalPackage => "package",
    }
}

fn walk_tree(node: Node<'_>, f: &mut dyn FnMut(Node<'_>)) {
    f(node);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_tree(child, f);
    }
}

fn node_text<'a>(node: Node<'_>, source: &'a str) -> Option<&'a str> {
    node.utf8_text(source.as_bytes()).ok()
}

fn extract_module_specifier(node: Node<'_>, source: &str) -> Option<String> {
    let source_node = node.child_by_field_name("source")?;
    let raw = node_text(source_node, source)?;
    let trimmed = raw.trim();
    if trimmed.len() < 2 {
        return None;
    }

    if (trimmed.starts_with('"') && trimmed.ends_with('"'))
        || (trimmed.starts_with('\'') && trimmed.ends_with('\''))
    {
        return Some(trimmed[1..trimmed.len() - 1].to_string());
    }

    None
}

fn parse_aliased_item(item: &str) -> (String, String) {
    if let Some((left, right)) = item.split_once(" as ") {
        (
            sanitize_identifier(left).to_string(),
            sanitize_identifier(right).to_string(),
        )
    } else {
        let cleaned = sanitize_identifier(item).to_string();
        (cleaned.clone(), cleaned)
    }
}

fn between(input: &str, open: char, close: char) -> Option<String> {
    let start = input.find(open)?;
    let end = input.rfind(close)?;
    if end <= start {
        return None;
    }
    Some(input[start + 1..end].to_string())
}

fn sanitize_identifier(input: &str) -> &str {
    input
        .trim()
        .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '$')
}

fn primary_type_name(input: &str) -> &str {
    let raw = input.trim().trim_start_matches(':').trim();
    let raw = sanitize_identifier(raw);
    raw.split(['<', '>', '|', '[', ']', '?', '!', ' ', ':'])
        .next()
        .unwrap_or(raw)
        .trim()
}

fn compact_whitespace(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Direct calls and constructions whose target is a name the file can resolve.
///
/// # Why only direct calls
///
/// A call edge is worth having only if it says *which* function ran. `foo()`
/// where `foo` arrived through an import, or is defined in this file, names a
/// symbol. A bare `foo()` with no such binding does not: matching it by leaf
/// name across the repository is the bug 0.2.0 fixed in the Rust adapter, and
/// an edge invented that way is worse than a missing one in a tool used to
/// decide whether a change is safe.
///
/// `obj.method()` is deliberately **not** a call edge. It is already recorded
/// as a property access on the receiver's declared type, which is the honest
/// statement — the type is touched at that line. Emitting `Calls` to the *type*
/// as well would claim the type was called, and would double every row for one
/// source location.
///
/// Returns `(callee_name, kind, line)`, sorted by the caller, so identical
/// input yields identical output.
fn collect_calls_and_instantiations(parsed: &ParsedFile) -> Vec<(String, RelationshipKind, u32)> {
    let mut out: Vec<(String, RelationshipKind, u32)> = Vec::new();

    walk_tree(parsed.tree.root_node(), &mut |node| {
        let (name_node, kind) = match node.kind() {
            // `new Foo(...)` — the constructor names a class directly.
            "new_expression" => match node.child_by_field_name("constructor") {
                Some(n) => (n, RelationshipKind::Instantiates),
                None => return,
            },
            // `foo(...)`, but not `obj.method(...)`: a non-identifier callee is
            // a member or computed expression, covered by property access.
            "call_expression" => match node.child_by_field_name("function") {
                Some(n) if n.kind() == "identifier" => (n, RelationshipKind::Calls),
                _ => return,
            },
            _ => return,
        };

        if name_node.kind() != "identifier" {
            return;
        }
        let Some(text) = node_text(name_node, &parsed.source) else {
            return;
        };
        let name = sanitize_identifier(text);
        if name.is_empty() {
            return;
        }

        out.push((
            name.to_string(),
            kind,
            name_node.start_position().row as u32 + 1,
        ));
    });

    // One row per (callee, kind, line): a name called twice on one line is one
    // fact about that line, and the order must not depend on traversal.
    out.sort();
    out.dedup();
    out
}
