//! Binding Go identifiers to the types they hold.
//!
//! Go makes this harder than it looks, because `a.B` is spelled the same
//! whether `a` is a variable (field access) or an imported package (qualified
//! reference). The previous implementation emitted a property-access edge for
//! every selector in the file — including `fmt.Println` — and then decided
//! which ones counted by comparing the receiver against the literal `"data"`.
//!
//! Here the two cases are separated by what the receiver actually is: a name
//! bound by a parameter, receiver or short variable declaration is a value, and
//! anything else that matches an import alias is a package.

use std::collections::{BTreeMap, BTreeSet};

use openinvar_core::ast::{node_text, walk};
use tree_sitter::Node;

/// Predeclared identifiers and ubiquitous standard-library types.
const GO_BUILTIN_TYPES: &[&str] = &[
    "bool",
    "string",
    "int",
    "int8",
    "int16",
    "int32",
    "int64",
    "uint",
    "uint8",
    "uint16",
    "uint32",
    "uint64",
    "uintptr",
    "byte",
    "rune",
    "float32",
    "float64",
    "complex64",
    "complex128",
    "error",
    "any",
    "interface{}",
];

/// Callable names the language provides.
///
/// The call resolver reaches `len(x)` and, by its own comment, could not say
/// whether that is a builtin or a local function it missed. Both are unbound;
/// only one is a gap in the graph. Builtin types are callable as conversions —
/// `int(x)` — so the check below consults both lists.
const GO_BUILTIN_CALLABLES: &[&str] = &[
    "len", "cap", "make", "new", "append", "copy", "delete", "panic", "recover", "print",
    "println", "close", "complex", "real", "imag", "min", "max", "clear",
];

/// True if `name` is a builtin callable rather than repository code.
pub fn is_builtin_callable(name: &str) -> bool {
    GO_BUILTIN_CALLABLES.contains(&name) || is_builtin_type(name)
}

pub fn is_builtin_type(name: &str) -> bool {
    GO_BUILTIN_TYPES.contains(&name)
}

/// What one file does with one type.
#[derive(Debug, Clone, Default)]
pub struct TypeAccess {
    pub properties: BTreeSet<String>,
    pub first_line: u32,
}

/// Type name (bare, or `pkg.Name` when qualified) to the members reached on it.
pub type TypeAccesses = BTreeMap<String, TypeAccess>;

/// Result of analysing one file's function bodies.
#[derive(Debug, Default)]
pub struct ScopeAnalysis {
    /// Members reached through values, keyed by the value's declared type.
    pub accesses: TypeAccesses,
    /// Types named in signatures and declarations, as `pkg.Name` or `Name`.
    pub referenced_types: BTreeMap<String, u32>,
}

/// Analyse every function in the file.
pub fn analyze(root: Node<'_>, source: &[u8]) -> ScopeAnalysis {
    let mut out = ScopeAnalysis::default();

    walk(root, &mut |node| {
        if !matches!(
            node.kind(),
            "function_declaration" | "method_declaration" | "func_literal"
        ) {
            return;
        }

        let bindings = bindings_for_function(node, source, &mut out.referenced_types);
        if bindings.is_empty() {
            return;
        }
        collect_accesses(node, source, &bindings, &mut out.accesses);
    });

    collect_type_references(root, source, &mut out.referenced_types);
    out
}

/// Every position where a type is named, not just inside function bodies.
///
/// Struct fields, return types and composite literals are all real dependencies
/// — a constructor returning `*MemoryStore` depends on it just as much as a
/// parameter would, and the type may live in a different file of the same
/// package.
fn collect_type_references(root: Node<'_>, source: &[u8], referenced: &mut BTreeMap<String, u32>) {
    let mut record = |node: Node<'_>, source: &[u8]| {
        if let Some(name) = type_name_of(node, source) {
            let line = node.start_position().row as u32 + 1;
            referenced.entry(name).or_insert(line);
        }
    };

    walk(root, &mut |node| match node.kind() {
        // `field Type` in a struct, `name Type` in a parameter list.
        "field_declaration" | "parameter_declaration" | "var_spec" | "const_spec" => {
            if let Some(ty) = node.child_by_field_name("type") {
                record(ty, source);
            }
        }
        // `Foo{...}` / `&Foo{...}`
        "composite_literal" => {
            if let Some(ty) = node.child_by_field_name("type") {
                record(ty, source);
            }
        }
        // Return types, including the parenthesised multi-value form.
        "function_declaration"
        | "method_declaration"
        | "method_elem"
        | "method_spec"
        | "func_literal" => {
            let Some(result) = node.child_by_field_name("result") else {
                return;
            };
            if result.kind() == "parameter_list" {
                let mut cursor = result.walk();
                for param in result.children(&mut cursor) {
                    if let Some(ty) = param.child_by_field_name("type") {
                        record(ty, source);
                    }
                }
            } else {
                record(result, source);
            }
        }
        // `var x Foo = ...` where the type sits on a type conversion.
        "type_conversion_expression" => {
            if let Some(ty) = node.child_by_field_name("type") {
                record(ty, source);
            }
        }
        _ => {}
    });
}

type Bindings = BTreeMap<String, String>;

fn bindings_for_function(
    func: Node<'_>,
    source: &[u8],
    referenced: &mut BTreeMap<String, u32>,
) -> Bindings {
    let mut bindings = Bindings::new();

    // `func (m *Mapper) Map(...)` — the receiver is a binding like any other.
    if let Some(receiver) = func.child_by_field_name("receiver") {
        collect_parameter_list(receiver, source, &mut bindings, referenced);
    }
    if let Some(params) = func.child_by_field_name("parameters") {
        collect_parameter_list(params, source, &mut bindings, referenced);
    }

    if let Some(body) = func.child_by_field_name("body") {
        walk(body, &mut |node| match node.kind() {
            // `x := Foo{}` / `x := pkg.Foo{}`
            "short_var_declaration" => {
                let (Some(left), Some(right)) = (
                    node.child_by_field_name("left"),
                    node.child_by_field_name("right"),
                ) else {
                    return;
                };
                bind_expression_list(left, right, source, &mut bindings);
            }
            // `var x Foo`
            "var_spec" | "const_spec" => {
                let Some(ty) = node
                    .child_by_field_name("type")
                    .and_then(|t| type_name_of(t, source))
                else {
                    return;
                };
                let Some(name) = node
                    .child_by_field_name("name")
                    .and_then(|n| node_text(n, source))
                else {
                    return;
                };
                referenced
                    .entry(ty.clone())
                    .or_insert(node.start_position().row as u32 + 1);
                bindings.insert(name.to_string(), ty);
            }
            _ => {}
        });
    }

    bindings
}

fn collect_parameter_list(
    list: Node<'_>,
    source: &[u8],
    bindings: &mut Bindings,
    referenced: &mut BTreeMap<String, u32>,
) {
    let mut cursor = list.walk();
    for param in list.children(&mut cursor) {
        if param.kind() != "parameter_declaration" {
            continue;
        }
        let Some(ty) = param
            .child_by_field_name("type")
            .and_then(|t| type_name_of(t, source))
        else {
            continue;
        };
        referenced
            .entry(ty.clone())
            .or_insert(param.start_position().row as u32 + 1);

        // One declaration can name several parameters: `a, b *Foo`.
        let mut names = param.walk();
        for child in param.children(&mut names) {
            if child.kind() == "identifier" {
                if let Some(name) = node_text(child, source) {
                    bindings.insert(name.to_string(), ty.clone());
                }
            }
        }
    }
}

/// Bind `a, b := expr, expr` positionally.
fn bind_expression_list(left: Node<'_>, right: Node<'_>, source: &[u8], bindings: &mut Bindings) {
    let names: Vec<Node<'_>> = named_children(left);
    let values: Vec<Node<'_>> = named_children(right);
    if names.len() != values.len() {
        return; // multi-return call: types are not recoverable syntactically
    }

    for (name_node, value) in names.into_iter().zip(values) {
        if name_node.kind() != "identifier" {
            continue;
        }
        let (Some(name), Some(ty)) = (
            node_text(name_node, source),
            inferred_type_name(value, source),
        ) else {
            continue;
        };
        bindings.insert(name.to_string(), ty);
    }
}

fn named_children<'t>(node: Node<'t>) -> Vec<Node<'t>> {
    let mut cursor = node.walk();
    let children: Vec<Node<'t>> = node.named_children(&mut cursor).collect();
    if children.is_empty() {
        vec![node]
    } else {
        children
    }
}

/// The type a composite literal or address-of expression produces.
fn inferred_type_name(value: Node<'_>, source: &[u8]) -> Option<String> {
    match value.kind() {
        "composite_literal" => value
            .child_by_field_name("type")
            .and_then(|t| type_name_of(t, source)),
        // `&Foo{}`
        "unary_expression" => value
            .child_by_field_name("operand")
            .and_then(|inner| inferred_type_name(inner, source)),
        _ => None,
    }
}

/// Reduce a type expression to the name a selector resolves against.
///
/// Qualified types keep their package prefix (`models.UserPayload`) because the
/// resolver needs it to pick the right package.
fn type_name_of(ty: Node<'_>, source: &[u8]) -> Option<String> {
    match ty.kind() {
        "type_identifier" => node_text(ty, source).map(str::to_string),
        "qualified_type" => {
            let package = node_text(ty.child_by_field_name("package")?, source)?;
            let name = node_text(ty.child_by_field_name("name")?, source)?;
            Some(format!("{package}.{name}"))
        }
        "pointer_type" => ty
            .named_child(0)
            .and_then(|inner| type_name_of(inner, source)),
        // Slices, maps, channels and funcs have no single member namespace.
        _ => None,
    }
}

/// Record members reached through bound values, ignoring package selectors.
fn collect_accesses(func: Node<'_>, source: &[u8], bindings: &Bindings, out: &mut TypeAccesses) {
    let Some(body) = func.child_by_field_name("body") else {
        return;
    };

    walk(body, &mut |node| {
        if node.kind() != "selector_expression" {
            return;
        }
        let (Some(operand), Some(field)) = (
            node.child_by_field_name("operand"),
            node.child_by_field_name("field"),
        ) else {
            return;
        };
        // Only `identifier.field`. A chained `a.b.c` would need the type of
        // `a.b`, which is not available without field-type resolution.
        if operand.kind() != "identifier" {
            return;
        }

        let Some(name) = node_text(operand, source) else {
            return;
        };
        // Not a value in scope — this is a package qualifier such as
        // `fmt.Println`, handled as a type reference instead of an access.
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
