//! Binding receivers to their declared types.
//!
//! C# declares the type of every field, parameter and local, so a receiver's
//! type is written down a few lines up — the same property that makes Java
//! tractable.
//!
//! # `var`, and the line this draws
//!
//! `var` is idiomatic C#, so dropping every `var`-typed receiver would lose
//! most real calls. But there are two different things hiding under one
//! keyword:
//!
//! - `var x = new Foo()` **states** its type. The initialiser names the
//!   constructor, so reading it is reading, not inferring. Taken.
//! - `var x = Something()` does not. Working that out means resolving a return
//!   type and following it — type inference, a second and weaker type system
//!   beside the real one, disagreeing with it exactly where it matters. Not
//!   taken; the receiver binds to nothing and records no edge.
//!
//! The first case covers the common idiom without the second's risk.

use std::collections::BTreeMap;

use openinvar_core::ir::{Relationship, RelationshipKind, Resolution, SymbolKind};
use openinvar_core::symbol_id::{make_symbol_id, unresolved_import_id, unresolved_local_type_id};
use tree_sitter::Node;

use super::extractor::{declared_type, descendants_of_kind, direct_children, text, type_name};
use super::parser::ParsedFile;

type Scope = BTreeMap<String, String>;

const TYPE_KINDS: &[&str] = &[
    "class_declaration",
    "interface_declaration",
    "struct_declaration",
    "record_declaration",
    "record_struct_declaration",
];

pub fn analyze(parsed: &ParsedFile) -> Vec<Relationship> {
    let file = parsed.file.as_str();
    let source = parsed.source.as_str();
    let mut out = Vec::new();

    for kind in TYPE_KINDS {
        for type_node in descendants_of_kind(parsed.tree.root_node(), kind) {
            let Some(name_node) = type_node.child_by_field_name("name") else {
                continue;
            };
            let owner = text(name_node, source).to_string();

            // Fields and properties are in scope for every member of the type,
            // and `this` names the type itself.
            let mut type_scope = Scope::new();
            for field in descendants_of_kind(type_node, "field_declaration") {
                let Some(declaration) = direct_children(field)
                    .into_iter()
                    .find(|c| c.kind() == "variable_declaration")
                else {
                    continue;
                };
                let Some(declared) = declared_type(declaration, source) else {
                    continue;
                };
                for declarator in descendants_of_kind(declaration, "variable_declarator") {
                    if let Some(n) = direct_children(declarator)
                        .into_iter()
                        .find(|c| c.kind() == "identifier")
                    {
                        type_scope.insert(text(n, source).to_string(), declared.clone());
                    }
                }
            }
            for property in descendants_of_kind(type_node, "property_declaration") {
                if let (Some(name), Some(declared)) = (
                    property.child_by_field_name("name"),
                    property.child_by_field_name("type").and_then(|t| type_name(t, source)),
                ) {
                    type_scope.insert(text(name, source).to_string(), declared);
                }
            }
            type_scope.insert("this".to_string(), owner.clone());

            for member in members_of(type_node) {
                let Some(member_name) = member.child_by_field_name("name") else {
                    continue;
                };
                let from = make_symbol_id(
                    file,
                    &format!("{owner}::{}", text(member_name, source)),
                    &SymbolKind::Method,
                );

                let mut scope = type_scope.clone();
                for param in descendants_of_kind(member, "parameter") {
                    if let (Some(declared), Some(name)) = (
                        param.child_by_field_name("type").and_then(|t| type_name(t, source)),
                        param.child_by_field_name("name"),
                    ) {
                        scope.insert(text(name, source).to_string(), declared);
                    }
                }
                for local in descendants_of_kind(member, "variable_declaration") {
                    let declared = declared_type(local, source);
                    for declarator in descendants_of_kind(local, "variable_declarator") {
                        let Some(n) = direct_children(declarator)
                            .into_iter()
                            .find(|c| c.kind() == "identifier")
                        else {
                            continue;
                        };
                        // `var x = new Foo()` states its type in the
                        // initialiser, so reading it is not inference — the
                        // constructor names the type outright. That case is
                        // taken; `var x = Something()` is not, because working
                        // *that* out means inferring a return type, which is a
                        // second type system beside the real one.
                        let from_new = direct_children(declarator)
                            .into_iter()
                            .find(|c| c.kind() == "object_creation_expression")
                            .and_then(|c| c.child_by_field_name("type"))
                            .and_then(|t| type_name(t, source));
                        if let Some(bound) = from_new.or_else(|| declared.clone()) {
                            scope.insert(text(n, source).to_string(), bound);
                        }
                    }
                }

                out.extend(calls_in(member, file, source, &from, &owner, &scope));
                out.extend(property_access_in(member, file, source, &from, &scope));
            }
        }
    }

    out
}

fn calls_in(
    member: Node<'_>,
    file: &str,
    source: &str,
    from: &str,
    owner: &str,
    scope: &Scope,
) -> Vec<Relationship> {
    let mut out = Vec::new();

    for call in descendants_of_kind(member, "invocation_expression") {
        let Some(function) = call.child_by_field_name("function") else {
            continue;
        };
        let line = call.start_position().row as u32 + 1;

        let (target_type, method) = match function.kind() {
            // A bare call: a member of the enclosing type, or one brought in by
            // `using static`. The resolver decides, because only it knows what
            // was imported.
            "identifier" => (owner.to_string(), text(function, source).to_string()),
            "member_access_expression" => {
                let (Some(object), Some(name)) = (
                    function.child_by_field_name("expression"),
                    function.child_by_field_name("name"),
                ) else {
                    continue;
                };
                let receiver = text(object, source);
                let method = text(name, source).to_string();
                match scope.get(receiver) {
                    Some(declared) => (declared.clone(), method),
                    // An unknown receiver that looks like a type name is a
                    // static call. Convention rather than certainty, so it only
                    // ever produces a candidate the resolver must still find.
                    None if starts_uppercase(receiver) && !receiver.contains('.') => {
                        (receiver.to_string(), method)
                    }
                    None => continue,
                }
            }
            _ => continue,
        };

        out.push(Relationship {
            from: from.to_string(),
            to: unresolved_import_id(&target_type, &method),
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
    member: Node<'_>,
    file: &str,
    source: &str,
    from: &str,
    scope: &Scope,
) -> Vec<Relationship> {
    let mut by_type: BTreeMap<String, (Vec<String>, u32)> = BTreeMap::new();

    for access in descendants_of_kind(member, "member_access_expression") {
        // The receiver of a call is handled as a call, not as a property read.
        if access
            .parent()
            .is_some_and(|p| p.kind() == "invocation_expression")
        {
            continue;
        }
        let (Some(object), Some(name)) = (
            access.child_by_field_name("expression"),
            access.child_by_field_name("name"),
        ) else {
            continue;
        };
        let Some(declared) = scope.get(text(object, source)) else {
            continue;
        };
        let line = access.start_position().row as u32 + 1;
        let entry = by_type
            .entry(declared.clone())
            .or_insert_with(|| (Vec::new(), line));
        let property = text(name, source).to_string();
        if !entry.0.contains(&property) {
            entry.0.push(property);
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

fn starts_uppercase(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_uppercase())
}

/// Methods, constructors and properties declared directly on `type_node`.
fn members_of(type_node: Node<'_>) -> Vec<Node<'_>> {
    let Some(body) = direct_children(type_node)
        .into_iter()
        .find(|c| c.kind() == "declaration_list")
    else {
        return Vec::new();
    };
    direct_children(body)
        .into_iter()
        .filter(|c| {
            matches!(
                c.kind(),
                "method_declaration" | "constructor_declaration" | "property_declaration"
            )
        })
        .collect()
}
