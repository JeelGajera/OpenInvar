//! Binding receivers to their declared types.
//!
//! This is what separates Tier 1 from a tags query. `service.handle()` is, to
//! a tags query, a call to *some* `handle`. Java declares the type of every
//! variable, so the receiver's type is written down a few lines up — and once
//! the type is known, the call has exactly one meaning.
//!
//! Java is the friendliest language this project has for the job. Nothing is
//! inferred, nothing is monkey-patched, and a local's type is a token rather
//! than a dataflow result. The whole analysis is a scoped symbol table.
//!
//! # What it does not attempt
//!
//! Chained access is attributed to the first receiver only — `a.b.c` records
//! `b` against the type of `a`, not `c` against the type of `a.b`. That is a
//! documented limit of the whole project rather than of this adapter, and it
//! is respected here rather than quietly exceeded.

use std::collections::BTreeMap;

use openinvar_core::ir::{Relationship, RelationshipKind, Resolution};
use openinvar_core::symbol_id::{
    make_symbol_id, unresolved_import_id, unresolved_local_type_id,
};
use openinvar_core::ir::SymbolKind;
use tree_sitter::Node;

use super::extractor::{descendants_of_kind, text};
use super::parser::ParsedFile;

/// Variable name to the simple name of its declared type.
type Scope = BTreeMap<String, String>;

pub fn analyze(parsed: &ParsedFile) -> Vec<Relationship> {
    let file = parsed.file.as_str();
    let source = parsed.source.as_str();
    let mut out = Vec::new();

    for type_node in type_declarations(parsed.tree.root_node()) {
        let Some(name_node) = type_node.child_by_field_name("name") else {
            continue;
        };
        let owner = text(name_node, source).to_string();

        // Fields are in scope for every method of the type, and `this` names
        // the type itself.
        let mut type_scope = Scope::new();
        for field in descendants_of_kind(type_node, "field_declaration") {
            let Some(declared) = declared_type(field, source) else {
                continue;
            };
            for declarator in descendants_of_kind(field, "variable_declarator") {
                if let Some(name) = declarator.child_by_field_name("name") {
                    type_scope.insert(text(name, source).to_string(), declared.clone());
                }
            }
        }
        type_scope.insert("this".to_string(), owner.clone());

        for method in methods_of(type_node) {
            let Some(method_name) = method.child_by_field_name("name") else {
                continue;
            };
            let from = make_symbol_id(
                file,
                &format!("{owner}::{}", text(method_name, source)),
                &SymbolKind::Method,
            );

            let mut scope = type_scope.clone();
            for param in descendants_of_kind(method, "formal_parameter") {
                if let (Some(declared), Some(name)) =
                    (declared_type(param, source), param.child_by_field_name("name"))
                {
                    scope.insert(text(name, source).to_string(), declared);
                }
            }
            for local in descendants_of_kind(method, "local_variable_declaration") {
                let Some(declared) = declared_type(local, source) else {
                    continue;
                };
                for declarator in descendants_of_kind(local, "variable_declarator") {
                    if let Some(name) = declarator.child_by_field_name("name") {
                        scope.insert(text(name, source).to_string(), declared.clone());
                    }
                }
            }

            out.extend(calls_in(method, file, source, &from, &owner, &scope));
            out.extend(property_access_in(method, file, source, &from, &scope));
        }
    }

    out
}

fn calls_in(
    method: Node<'_>,
    file: &str,
    source: &str,
    from: &str,
    owner: &str,
    scope: &Scope,
) -> Vec<Relationship> {
    let mut out = Vec::new();

    for call in descendants_of_kind(method, "method_invocation") {
        let Some(name_node) = call.child_by_field_name("name") else {
            continue;
        };
        let member = text(name_node, source).to_string();
        let line = call.start_position().row as u32 + 1;

        // No receiver: a call to a method of the enclosing type, or to a
        // statically imported one. Both are named the same way here and the
        // resolver decides, because only it knows what was imported.
        let target_type = match call.child_by_field_name("object") {
            None => owner.to_string(),
            Some(object) => {
                let receiver = text(object, source);
                match scope.get(receiver) {
                    Some(declared) => declared.clone(),
                    // An unknown receiver is left alone rather than guessed at.
                    // It is usually a static call on a type this file imported,
                    // in which case the receiver *is* the type name.
                    None if starts_uppercase(receiver) => receiver.to_string(),
                    None => continue,
                }
            }
        };

        out.push(Relationship {
            from: from.to_string(),
            // `(owner type, member)` rather than `(module, symbol)`. The shape
            // is the same pair and the resolver reads it the same way; reusing
            // it keeps one placeholder vocabulary rather than inventing a
            // second that `is_placeholder` would then have to learn.
            to: unresolved_import_id(&target_type, &member),
            kind: RelationshipKind::Calls,
            alias: None,
            properties_accessed: vec![],
            context: "call".to_string(),
            file: file.to_string(),
            line,
            resolution: Resolution::default(),
        });
    }

    out
}

fn property_access_in(
    method: Node<'_>,
    file: &str,
    source: &str,
    from: &str,
    scope: &Scope,
) -> Vec<Relationship> {
    // Grouped by the type accessed, so one edge carries every property read on
    // that type rather than one edge per read — which is how `properties_accessed`
    // is meant to be used and what keeps the graph legible.
    let mut by_type: BTreeMap<String, (Vec<String>, u32)> = BTreeMap::new();

    for access in descendants_of_kind(method, "field_access") {
        let (Some(object), Some(field)) = (
            access.child_by_field_name("object"),
            access.child_by_field_name("field"),
        ) else {
            continue;
        };
        let receiver = text(object, source);
        let Some(declared) = scope.get(receiver) else {
            continue;
        };
        let line = access.start_position().row as u32 + 1;
        let entry = by_type
            .entry(declared.clone())
            .or_insert_with(|| (Vec::new(), line));
        let name = text(field, source).to_string();
        if !entry.0.contains(&name) {
            entry.0.push(name);
        }
        entry.1 = entry.1.min(line);
    }

    by_type
        .into_iter()
        .map(|(type_name, (mut properties, line))| {
            properties.sort();
            Relationship {
                from: from.to_string(),
                to: unresolved_local_type_id(&type_name),
                kind: RelationshipKind::AccessesProperty,
                alias: Some(type_name),
                properties_accessed: properties,
                context: "property access".to_string(),
                file: file.to_string(),
                line,
                resolution: Resolution::default(),
            }
        })
        .collect()
}

/// A bare uppercase identifier in receiver position is a type, by Java's
/// naming convention — `Helpers.log()`. Convention rather than certainty, so
/// it only ever produces a *candidate* the resolver still has to find.
fn starts_uppercase(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_uppercase())
}

fn declared_type(node: Node<'_>, source: &str) -> Option<String> {
    let type_node = node.child_by_field_name("type")?;
    match type_node.kind() {
        "type_identifier" => Some(text(type_node, source).to_string()),
        "generic_type" => direct_children(type_node)
            .into_iter()
            .find(|c| c.kind() == "type_identifier")
            .map(|n| text(n, source).to_string()),
        "scoped_type_identifier" => text(type_node, source).rsplit('.').next().map(str::to_string),
        _ => None,
    }
}

fn type_declarations(root: Node<'_>) -> Vec<Node<'_>> {
    const KINDS: &[&str] = &[
        "class_declaration",
        "interface_declaration",
        "enum_declaration",
        "record_declaration",
    ];
    let mut out = Vec::new();
    for kind in KINDS {
        out.extend(descendants_of_kind(root, kind));
        if root.kind() == *kind {
            out.push(root);
        }
    }
    out.sort_by_key(|n| n.start_byte());
    out
}

/// Methods and constructors declared directly on `type_node`.
///
/// Direct members only: a nested type's methods belong to the nested type, and
/// crediting them to the outer one would attribute a call to the wrong symbol.
fn direct_children<'t>(node: Node<'t>) -> Vec<Node<'t>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).collect()
}

fn methods_of(type_node: Node<'_>) -> Vec<Node<'_>> {
    let children = direct_children(type_node);
    let body = children.into_iter().find(|c| {
        matches!(
            c.kind(),
            "class_body" | "interface_body" | "enum_body" | "record_body"
        )
    });
    let Some(body) = body else { return Vec::new() };

    direct_children(body)
        .into_iter()
        .filter(|c| {
            matches!(
                c.kind(),
                "method_declaration" | "constructor_declaration" | "compact_constructor_declaration"
            )
        })
        .collect()
}
