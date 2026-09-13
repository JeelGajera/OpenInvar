//! Binding C and C++ identifiers to their declared types.
//!
//! Replaces a text scan that looked for `<alias>.` and `<alias>->` and,
//! failing that, compared the receiver against the literal string `"data"`.
//! Bindings now come from declarators, so `p->user_id` is attributed to
//! whatever type `p` was actually declared as.

use std::collections::{BTreeMap, BTreeSet};

use openinvar_core::ast::{node_text, walk};
use tree_sitter::Node;

/// Built-in and ubiquitous standard-library types.
const C_BUILTIN_TYPES: &[&str] = &[
    "void",
    "char",
    "short",
    "int",
    "long",
    "float",
    "double",
    "signed",
    "unsigned",
    "bool",
    "size_t",
    "ssize_t",
    "ptrdiff_t",
    "intptr_t",
    "uintptr_t",
    "int8_t",
    "int16_t",
    "int32_t",
    "int64_t",
    "uint8_t",
    "uint16_t",
    "uint32_t",
    "uint64_t",
    "wchar_t",
    "char16_t",
    "char32_t",
    "FILE",
    "auto",
    "string",
    "wstring",
    "vector",
    "map",
    "unordered_map",
    "set",
    "unique_ptr",
    "shared_ptr",
    "weak_ptr",
    "optional",
    "variant",
    "pair",
    "tuple",
    "ostream",
    "istream",
];

pub fn is_builtin_type(name: &str) -> bool {
    C_BUILTIN_TYPES.contains(&name)
}

#[derive(Debug, Clone, Default)]
pub struct TypeAccess {
    pub properties: BTreeSet<String>,
    pub first_line: u32,
}

pub type TypeAccesses = BTreeMap<String, TypeAccess>;

/// Members reached through declared values, keyed by type name.
///
/// Bindings are collected per function so a name reused across functions with
/// different types keeps its accesses separate.
pub fn collect_type_accesses(root: Node<'_>, source: &[u8]) -> TypeAccesses {
    let mut out: TypeAccesses = BTreeMap::new();

    walk(root, &mut |node| {
        if node.kind() != "function_definition" {
            return;
        }
        let bindings = bindings_for_function(node, source);
        if bindings.is_empty() {
            return;
        }
        collect_accesses(node, source, &bindings, &mut out);
    });

    out
}

type Bindings = BTreeMap<String, String>;

fn bindings_for_function(func: Node<'_>, source: &[u8]) -> Bindings {
    let mut bindings = Bindings::new();

    // Parameters live under the function's declarator.
    if let Some(declarator) = func.child_by_field_name("declarator") {
        walk(declarator, &mut |node| {
            if node.kind() == "parameter_declaration" {
                bind_declaration(node, source, &mut bindings);
            }
        });
    }

    // Locals.
    if let Some(body) = func.child_by_field_name("body") {
        walk(body, &mut |node| {
            if node.kind() == "declaration" {
                bind_declaration(node, source, &mut bindings);
            }
        });
    }

    bindings
}

/// Bind every name introduced by one declaration to its type.
///
/// `struct UserPayload *a, b;` declares two names of the same type; both are
/// recorded, pointer depth being irrelevant to which members exist.
fn bind_declaration(node: Node<'_>, source: &[u8], bindings: &mut Bindings) {
    let Some(type_name) = node
        .child_by_field_name("type")
        .and_then(|t| type_name_of(t, source))
    else {
        return;
    };

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.id()
            == node
                .child_by_field_name("type")
                .map(|t| t.id())
                .unwrap_or(0)
        {
            continue;
        }
        if let Some(name) = declarator_name(child, source) {
            bindings.insert(name, type_name.clone());
        }
    }
}

/// Strip pointers, references, arrays and initialisers down to the declared name.
pub fn declarator_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    match node.kind() {
        "identifier" | "field_identifier" => node_text(node, source).map(str::to_string),
        "pointer_declarator"
        | "reference_declarator"
        | "array_declarator"
        | "init_declarator"
        | "parenthesized_declarator" => {
            let inner = node
                .child_by_field_name("declarator")
                .or_else(|| node.named_child(0))?;
            declarator_name(inner, source)
        }
        // A function declarator names a function, not a value.
        "function_declarator" => None,
        _ => None,
    }
}

/// The name a member access resolves against.
pub fn type_name_of(ty: Node<'_>, source: &[u8]) -> Option<String> {
    match ty.kind() {
        "type_identifier" | "primitive_type" => node_text(ty, source).map(str::to_string),
        // `struct Foo` used as a type — the name without the tag keyword.
        "struct_specifier" | "union_specifier" | "enum_specifier" | "class_specifier" => ty
            .child_by_field_name("name")
            .and_then(|n| node_text(n, source))
            .map(str::to_string),
        // `ns::Foo` — the final segment.
        "qualified_identifier" => ty
            .child_by_field_name("name")
            .and_then(|n| type_name_of(n, source))
            .or_else(|| {
                node_text(ty, source)?
                    .rsplit("::")
                    .next()
                    .map(str::to_string)
            }),
        // `std::vector<Foo>` — the container owns the members.
        "template_type" => ty
            .child_by_field_name("name")
            .and_then(|n| type_name_of(n, source)),
        "sized_type_specifier" => node_text(ty, source).map(|t| t.trim().to_string()),
        // The C++ grammar wraps the right-hand side of `using X = Y;` and of a
        // cast in a `type_descriptor`, which holds the real type underneath.
        "type_descriptor" => ty
            .child_by_field_name("type")
            .or_else(|| ty.named_child(0))
            .and_then(|inner| type_name_of(inner, source)),
        _ => None,
    }
}

/// Names introduced by a `typedef`.
///
/// The new name is a `type_identifier`, not the `identifier` an ordinary
/// declarator uses, and one `typedef` may introduce several
/// (`typedef struct Foo A, *PA;`).
pub fn typedef_alias_names(node: Node<'_>, source: &[u8]) -> Vec<String> {
    let type_field_id = node.child_by_field_name("type").map(|t| t.id());

    let mut out = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if Some(child.id()) == type_field_id {
            continue;
        }
        if let Some(name) = alias_name(child, source) {
            out.push(name);
        }
    }
    out
}

fn alias_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    match node.kind() {
        "type_identifier" => node_text(node, source).map(str::to_string),
        "pointer_declarator" | "array_declarator" | "parenthesized_declarator" => node
            .child_by_field_name("declarator")
            .or_else(|| node.named_child(0))
            .and_then(|inner| alias_name(inner, source)),
        // `typedef int (*Callback)(void);` names a function pointer type.
        "function_declarator" => node
            .child_by_field_name("declarator")
            .and_then(|inner| alias_name(inner, source)),
        _ => None,
    }
}

fn collect_accesses(func: Node<'_>, source: &[u8], bindings: &Bindings, out: &mut TypeAccesses) {
    let Some(body) = func.child_by_field_name("body") else {
        return;
    };

    walk(body, &mut |node| {
        if node.kind() != "field_expression" {
            return;
        }
        let (Some(argument), Some(field)) = (
            node.child_by_field_name("argument"),
            node.child_by_field_name("field"),
        ) else {
            return;
        };
        // Only direct `name.field` / `name->field`; a chained access would need
        // the field's own type.
        if !matches!(argument.kind(), "identifier" | "this") {
            return;
        }

        let Some(name) = node_text(argument, source) else {
            return;
        };
        let Some(type_name) = bindings.get(name) else {
            return;
        };
        if is_builtin_type(type_name) {
            return;
        }
        let Some(field_name) = node_text(field, source) else {
            return;
        };

        let line = node.start_position().row as u32 + 1;
        let entry = out.entry(type_name.clone()).or_insert_with(|| TypeAccess {
            properties: BTreeSet::new(),
            first_line: line,
        });
        entry.properties.insert(field_name.to_string());
        entry.first_line = entry.first_line.min(line);
    });
}
