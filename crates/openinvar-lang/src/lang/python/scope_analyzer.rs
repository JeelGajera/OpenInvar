//! Binding Python names to the types they are annotated or constructed with.
//!
//! The previous implementation ran two regexes over raw function text: one for
//! `name: Type` and one for `obj.attr`. Both matched inside strings, comments
//! and unrelated dict literals, and the `name: Type` pattern also matched
//! dictionary keys. Bindings here come from the AST — parameter annotations,
//! annotated assignments, and constructor calls.

use std::collections::{BTreeMap, BTreeSet};

use openinvar_core::ast::{node_text, walk};
use tree_sitter::Node;

/// Builtins and typing constructs that never resolve to a repository symbol.
const PY_BUILTIN_TYPES: &[&str] = &[
    "int",
    "float",
    "str",
    "bool",
    "bytes",
    "complex",
    "list",
    "dict",
    "set",
    "frozenset",
    "tuple",
    "object",
    "type",
    "None",
    "Any",
    "Optional",
    "Union",
    "List",
    "Dict",
    "Set",
    "Tuple",
    "Callable",
    "Iterable",
    "Iterator",
    "Sequence",
    "Mapping",
    "Awaitable",
    "Coroutine",
    "Self",
    "Literal",
    "Final",
    "ClassVar",
    "Annotated",
    "TypeVar",
    "Generic",
    "Protocol",
];

pub fn is_builtin_type(name: &str) -> bool {
    PY_BUILTIN_TYPES.contains(&name)
}

#[derive(Debug, Clone, Default)]
pub struct TypeAccess {
    pub properties: BTreeSet<String>,
    pub first_line: u32,
}

pub type TypeAccesses = BTreeMap<String, TypeAccess>;

/// Attributes reached through annotated values, keyed by the annotated type.
pub fn collect_type_accesses(root: Node<'_>, source: &[u8]) -> TypeAccesses {
    let mut out: TypeAccesses = BTreeMap::new();

    walk(root, &mut |node| {
        if !matches!(node.kind(), "function_definition" | "lambda") {
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

    // `self` refers to the enclosing class.
    if let Some(class) = enclosing_class_name(func, source) {
        bindings.insert("self".to_string(), class);
    }

    if let Some(params) = func.child_by_field_name("parameters") {
        let mut cursor = params.walk();
        for param in params.children(&mut cursor) {
            // `name: Type` and `name: Type = default`
            if !matches!(param.kind(), "typed_parameter" | "typed_default_parameter") {
                continue;
            }
            let Some(type_name) = param
                .child_by_field_name("type")
                .and_then(|t| base_type_name(t, source))
            else {
                continue;
            };
            let Some(name) = parameter_name(param, source) else {
                continue;
            };
            bindings.insert(name, type_name);
        }
    }

    if let Some(body) = func.child_by_field_name("body") {
        walk(body, &mut |node| {
            if node.kind() != "assignment" {
                return;
            }
            let Some(target) = node
                .child_by_field_name("left")
                .and_then(|l| node_text(l, source))
            else {
                return;
            };
            // `x: Foo = ...` carries an explicit annotation; otherwise infer
            // from a constructor call.
            let ty = node
                .child_by_field_name("type")
                .and_then(|t| base_type_name(t, source))
                .or_else(|| {
                    node.child_by_field_name("right")
                        .and_then(|r| constructed_type_name(r, source))
                });
            if let Some(ty) = ty {
                bindings.insert(target.to_string(), ty);
            }
        });
    }

    bindings
}

fn parameter_name(param: Node<'_>, source: &[u8]) -> Option<String> {
    // The grammar exposes the name as the first child for both typed forms.
    let name = param
        .child_by_field_name("name")
        .or_else(|| param.named_child(0))?;
    node_text(name, source).map(str::to_string)
}

fn enclosing_class_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "class_definition" {
            return parent
                .child_by_field_name("name")
                .and_then(|n| node_text(n, source))
                .map(str::to_string);
        }
        current = parent.parent();
    }
    None
}

/// Reduce an annotation to the name attribute access resolves against.
///
/// `Optional[UserPayload]` yields `UserPayload`: the annotation describes a
/// value that is either the type or `None`, and attribute access targets the
/// type. This differs from Rust and Go, where `Vec<T>` really is a `Vec`.
pub fn base_type_name(annotation: Node<'_>, source: &[u8]) -> Option<String> {
    match annotation.kind() {
        "identifier" => node_text(annotation, source).map(str::to_string),
        // `pkg.Model` — the final attribute is the type.
        "attribute" => annotation
            .child_by_field_name("attribute")
            .and_then(|a| node_text(a, source))
            .map(str::to_string),
        // `Optional[X]`, `list[X]` — unwrap wrappers that pass attributes through.
        "subscript" => {
            let value = annotation.child_by_field_name("value")?;
            let container = node_text(value, source).unwrap_or("");
            if matches!(container, "Optional" | "Final" | "ClassVar" | "Annotated") {
                let inner = annotation.child_by_field_name("subscript")?;
                return base_type_name(inner, source);
            }
            base_type_name(value, source)
        }
        // `type` node wrapping the real annotation.
        "type" => annotation
            .named_child(0)
            .and_then(|inner| base_type_name(inner, source)),
        // `"UserPayload"` — a forward reference in quotes.
        "string" => node_text(annotation, source)
            .map(|t| t.trim_matches(|c| c == '"' || c == '\'').to_string())
            .filter(|t| !t.is_empty() && t.chars().all(|c| c.is_alphanumeric() || c == '_')),
        _ => None,
    }
}

/// The class named by a constructor call, for `x = Foo()`.
fn constructed_type_name(value: Node<'_>, source: &[u8]) -> Option<String> {
    if value.kind() != "call" {
        return None;
    }
    let function = value.child_by_field_name("function")?;
    let name = match function.kind() {
        "identifier" => node_text(function, source)?.to_string(),
        "attribute" => node_text(function.child_by_field_name("attribute")?, source)?.to_string(),
        _ => return None,
    };
    // Only a name that looks like a class; calling a function returns something
    // whose type we cannot know syntactically.
    name.chars().next()?.is_uppercase().then_some(name)
}

fn collect_accesses(func: Node<'_>, source: &[u8], bindings: &Bindings, out: &mut TypeAccesses) {
    let Some(body) = func.child_by_field_name("body") else {
        return;
    };

    walk(body, &mut |node| {
        if node.kind() != "attribute" {
            return;
        }
        let (Some(object), Some(attribute)) = (
            node.child_by_field_name("object"),
            node.child_by_field_name("attribute"),
        ) else {
            return;
        };
        if object.kind() != "identifier" {
            return; // chained access needs the intermediate attribute's type
        }

        let Some(name) = node_text(object, source) else {
            return;
        };
        let Some(type_name) = bindings.get(name) else {
            return;
        };
        if is_builtin_type(type_name) {
            return;
        }
        let Some(attr) = node_text(attribute, source) else {
            return;
        };

        let line = node.start_position().row as u32 + 1;
        let entry = out.entry(type_name.clone()).or_insert_with(|| TypeAccess {
            properties: BTreeSet::new(),
            first_line: line,
        });
        entry.properties.insert(attr.to_string());
        entry.first_line = entry.first_line.min(line);
    });
}
