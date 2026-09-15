//! Binding local variables to their declared types.
//!
//! Property tracking is only as good as this file. `payload.user_id` says
//! nothing on its own — the graph needs to know that `payload` is a
//! `ResponseModel`, which is an alias for `UserPayload`, before it can record
//! that callers depend on that field.
//!
//! The previous implementation scanned source text for `<alias>.` and, failing
//! that, compared the receiver against the literal string `"data"` — the
//! variable name used in the test fixture. Every other variable name silently
//! produced no properties at all. This module derives bindings from the AST
//! instead, so any name works and the answer is grounded in what was declared.

use std::collections::{BTreeMap, BTreeSet};

use openinvar_core::ast::{node_text, walk};
use tree_sitter::Node;

/// Type names that resolve to the standard library or the language itself.
///
/// Field access on these is real but points outside the repository, so it is
/// dropped rather than reported as an unresolved-type warning.
const RUST_BUILTIN_TYPES: &[&str] = &[
    "bool", "char", "str", "String", "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16",
    "i32", "i64", "i128", "isize", "f32", "f64", "Vec", "Option", "Result", "Box", "Rc", "Arc",
    "RefCell", "Cell", "Mutex", "RwLock", "HashMap", "HashSet", "BTreeMap", "BTreeSet", "VecDeque",
    "Cow", "Path", "PathBuf", "OsStr", "OsString", "Duration", "Instant", "Self",
];

/// True if `name` is a standard type rather than something defined in the repo.
pub fn is_builtin_type(name: &str) -> bool {
    RUST_BUILTIN_TYPES.contains(&name)
}

/// Callable names the language or its prelude provides.
///
/// The call resolver reaches a name it cannot place and has, until now, had no
/// way to say whether that is a prelude name or a local function it missed —
/// its own comment said as much. Both are unbound, but only one is a gap in
/// the graph, and a coverage figure that cannot separate them reports the
/// language's own vocabulary as missing work.
///
/// Deliberately short and literal. Every entry is in the prelude or is a macro
/// `std` exports, so a name here is outside any repository by definition; a
/// name merely *likely* to be external does not belong, because the whole point
/// of the split is that one side of it is evidence rather than guesswork.
const RUST_PRELUDE_CALLABLES: &[&str] = &[
    // Prelude constructors and functions.
    "Some", "None", "Ok", "Err", "drop", "default",
    // Macros `std` exports, spelled without the `!` by the extractor.
    "println", "eprintln", "print", "eprint", "format", "write", "writeln", "vec", "panic",
    "assert", "assert_eq", "assert_ne", "debug_assert", "debug_assert_eq", "debug_assert_ne",
    "matches", "todo", "unimplemented", "unreachable", "dbg", "include_str", "include_bytes",
    "concat", "stringify", "env", "line", "file", "column", "module_path", "cfg",
];

/// True if `name` is a prelude or `std` macro callable rather than repository code.
pub fn is_prelude_callable(name: &str) -> bool {
    RUST_PRELUDE_CALLABLES.contains(&name)
}

/// Fields and methods observed on each type within one file.
pub type TypeAccesses = BTreeMap<String, TypeAccess>;

/// Everything one file does with one type.
#[derive(Debug, Clone, Default)]
pub struct TypeAccess {
    /// Field and method names reached through a value of this type.
    pub properties: BTreeSet<String>,
    /// First line an access appeared on, so the edge can point somewhere useful.
    pub first_line: u32,
}

/// Variable name to declared type name, valid within one function body.
type Bindings = BTreeMap<String, String>;

/// Collect, for every type used in the file, the set of members reached through it.
///
/// Bindings are gathered per function so that a name reused with different types
/// in different functions does not cross-contaminate — the failure that made
/// `Alpha` appear to have `Beta`'s fields.
pub fn collect_type_accesses(root: Node<'_>, source: &[u8]) -> TypeAccesses {
    let mut out: TypeAccesses = BTreeMap::new();

    walk(root, &mut |node| {
        if !is_function_like(node) {
            return;
        }
        let bindings = bindings_for_function(node, source);
        if bindings.is_empty() {
            return;
        }
        collect_accesses_in_body(node, source, &bindings, &mut out);
    });

    out
}

fn is_function_like(node: Node<'_>) -> bool {
    matches!(node.kind(), "function_item" | "closure_expression")
}

/// Build the variable-to-type map visible inside one function.
fn bindings_for_function(func: Node<'_>, source: &[u8]) -> Bindings {
    let mut bindings = Bindings::new();

    // A generic parameter names no concrete type, so a value of that type
    // cannot be attributed to a symbol. Binding `subject: &T` would leave `T`
    // permanently unresolvable and produce a warning on every generic function.
    let type_parameters = generic_parameter_names(func, source);

    // `self` refers to the type of the enclosing `impl` block.
    if let Some(self_type) = enclosing_impl_type(func, source) {
        bindings.insert("self".to_string(), self_type);
    }

    if let Some(params) = func.child_by_field_name("parameters") {
        let mut cursor = params.walk();
        for param in params.children(&mut cursor) {
            match param.kind() {
                "parameter" => {
                    if let (Some(name), Some(ty)) = (
                        param
                            .child_by_field_name("pattern")
                            .and_then(|p| binding_name(p, source)),
                        param
                            .child_by_field_name("type")
                            .and_then(|t| base_type_name(t, source))
                            .filter(|ty| !type_parameters.contains(ty)),
                    ) {
                        bindings.insert(name, ty);
                    }
                }
                // `&self` / `self` — already handled via the impl type above.
                "self_parameter" => {}
                _ => {}
            }
        }
    }

    // `let` bindings anywhere in the body.
    if let Some(body) = func.child_by_field_name("body") {
        walk(body, &mut |node| {
            if node.kind() != "let_declaration" {
                return;
            }
            let Some(name) = node
                .child_by_field_name("pattern")
                .and_then(|p| binding_name(p, source))
            else {
                return;
            };

            // An explicit annotation wins; otherwise infer from the initializer.
            let ty = node
                .child_by_field_name("type")
                .and_then(|t| base_type_name(t, source))
                .or_else(|| {
                    node.child_by_field_name("value")
                        .and_then(|v| inferred_type_name(v, source))
                });

            if let Some(ty) = ty.filter(|t| !type_parameters.contains(t)) {
                bindings.insert(name, ty);
            }
        });
    }

    bindings
}

/// Names introduced by `<T, U: Bound>` on a function or its enclosing `impl`.
///
/// These are placeholders for types chosen by the caller, not types that exist
/// anywhere in the repository.
fn generic_parameter_names(func: Node<'_>, source: &[u8]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();

    let mut collect = |node: Node<'_>| {
        let Some(params) = node.child_by_field_name("type_parameters") else {
            return;
        };
        let mut cursor = params.walk();
        for param in params.named_children(&mut cursor) {
            let name = match param.kind() {
                // One node covers `T`, `T: Bound` and `T = Default` since
                // tree-sitter-rust 0.24, with the name in a required field.
                // Before that the grammar had three shapes — a bare
                // `type_identifier`, `constrained_type_parameter` and
                // `optional_type_parameter` — and matching only those meant
                // this collected nothing at all on the newer grammar. It fails
                // quietly: an empty set filters nothing, so `subject: &T` bound
                // to `T` and every generic function reported its own parameter
                // as a missing type.
                "type_parameter" => param
                    .child_by_field_name("name")
                    .and_then(|n| node_text(n, source)),
                // Still emitted for `const N: usize` and `'a`, neither of which
                // names a type, so neither belongs in this set.
                "const_parameter" | "lifetime_parameter" => None,
                _ => None,
            };
            if let Some(name) = name {
                out.insert(name.to_string());
            }
        }
    };

    collect(func);
    // Generics declared on `impl<T> Foo<T>` are in scope for its methods too.
    let mut current = func.parent();
    while let Some(node) = current {
        if node.kind() == "impl_item" {
            collect(node);
            break;
        }
        current = node.parent();
    }

    out
}

/// The type an `impl` block is for, as seen from a function inside it.
fn enclosing_impl_type(func: Node<'_>, source: &[u8]) -> Option<String> {
    let mut current = func.parent();
    while let Some(node) = current {
        if node.kind() == "impl_item" {
            return node
                .child_by_field_name("type")
                .and_then(|t| base_type_name(t, source));
        }
        current = node.parent();
    }
    None
}

/// The identifier a pattern binds, for the simple patterns worth tracking.
///
/// Destructuring patterns bind several names to field types rather than to the
/// annotated type, so they are skipped instead of guessed at.
fn binding_name(pattern: Node<'_>, source: &[u8]) -> Option<String> {
    match pattern.kind() {
        "identifier" => node_text(pattern, source).map(str::to_string),
        // `mut x`, `ref x`
        "mut_pattern" | "ref_pattern" => pattern
            .named_child(0)
            .and_then(|inner| binding_name(inner, source)),
        _ => None,
    }
}

/// Reduce a type expression to the name a member access would resolve against.
///
/// `&mut Vec<Foo>` yields `Vec`, not `Foo`: `v.len()` is a `Vec` method. The
/// inner types are still recorded separately as type references.
fn base_type_name(ty: Node<'_>, source: &[u8]) -> Option<String> {
    match ty.kind() {
        "type_identifier" | "primitive_type" => node_text(ty, source).map(str::to_string),
        // `&T`, `&mut T`, `*const T`
        "reference_type" | "pointer_type" => ty
            .child_by_field_name("type")
            .and_then(|inner| base_type_name(inner, source)),
        // `Vec<T>` — the receiver is the `Vec`.
        "generic_type" => ty
            .child_by_field_name("type")
            .and_then(|inner| base_type_name(inner, source)),
        // `models::UserPayload` — the receiver is the final segment.
        "scoped_type_identifier" => ty
            .child_by_field_name("name")
            .and_then(|n| node_text(n, source))
            .map(str::to_string),
        // `(T, U)`, `[T; N]`, `dyn Trait`, `impl Trait` — no single receiver type.
        _ => None,
    }
}

/// Guess a binding's type from its initializer.
fn inferred_type_name(value: Node<'_>, source: &[u8]) -> Option<String> {
    match value.kind() {
        // `Foo { .. }`
        "struct_expression" => value
            .child_by_field_name("name")
            .and_then(|n| base_type_name(n, source)),
        // `Foo::new(..)` — the constructor's owning type.
        "call_expression" => {
            let func = value.child_by_field_name("function")?;
            if func.kind() == "scoped_identifier" {
                let path = func.child_by_field_name("path")?;
                return node_text(path, source)
                    .and_then(|t| t.rsplit("::").next())
                    .map(str::to_string);
            }
            None
        }
        // `&expr`, `expr?`, `expr.await`
        "reference_expression" | "try_expression" | "await_expression" => value
            .named_child(0)
            .and_then(|inner| inferred_type_name(inner, source)),
        _ => None,
    }
}

/// Record every member reached through a bound variable inside this function.
fn collect_accesses_in_body(
    func: Node<'_>,
    source: &[u8],
    bindings: &Bindings,
    out: &mut TypeAccesses,
) {
    let Some(body) = func.child_by_field_name("body") else {
        return;
    };

    let mut record = |type_name: &str, field_name: &str, line: u32| {
        let entry = out
            .entry(type_name.to_string())
            .or_insert_with(|| TypeAccess {
                properties: BTreeSet::new(),
                first_line: line,
            });
        entry.properties.insert(field_name.to_string());
        entry.first_line = entry.first_line.min(line);
    };

    walk(body, &mut |node| {
        // Macro arguments are not parsed as expressions — `format!("{}", a.b)`
        // arrives as an opaque `token_tree`. Skipping them would lose field
        // accesses inside `format!`, `println!`, `assert!` and `vec!`, which is
        // where a great deal of real field usage lives.
        if node.kind() == "token_tree" {
            scan_token_tree(node, source, bindings, &mut record);
            return;
        }

        if node.kind() != "field_expression" {
            return;
        }
        let (Some(receiver), Some(field)) = (
            node.child_by_field_name("value"),
            node.child_by_field_name("field"),
        ) else {
            return;
        };

        // Only direct `variable.field` accesses are attributed. Chained access
        // (`a.b.c`) would need the type of `a.b`, which requires field-type
        // resolution the graph does not have yet; attributing `c` to `a`'s type
        // would be wrong, so the outer access is left alone.
        if receiver.kind() != "identifier" && receiver.kind() != "self" {
            return;
        }

        let Some(receiver_name) = node_text(receiver, source) else {
            return;
        };
        let Some(type_name) = bindings.get(receiver_name) else {
            return;
        };
        if is_builtin_type(type_name) {
            return;
        }
        let Some(field_name) = node_text(field, source) else {
            return;
        };

        record(type_name, field_name, node.start_position().row as u32 + 1);
    });
}

/// Find `name.field` token sequences inside an unparsed macro argument list.
///
/// Only a direct `identifier . identifier` run is taken, matching the rule used
/// for parsed expressions: attributing `c` in `a.b.c` would require knowing the
/// type of `a.b`.
fn scan_token_tree<F>(tree: Node<'_>, source: &[u8], bindings: &Bindings, record: &mut F)
where
    F: FnMut(&str, &str, u32),
{
    let mut cursor = tree.walk();
    let tokens: Vec<Node<'_>> = tree.children(&mut cursor).collect();

    let mut index = 0usize;
    while index + 2 < tokens.len() {
        let (receiver, dot, field) = (tokens[index], tokens[index + 1], tokens[index + 2]);
        if receiver.kind() != "identifier" || dot.kind() != "." || field.kind() != "identifier" {
            index += 1;
            continue;
        }

        let matched = (|| {
            let name = node_text(receiver, source)?;
            let type_name = bindings.get(name)?;
            if is_builtin_type(type_name) {
                return None;
            }
            let field_name = node_text(field, source)?;
            record(
                type_name,
                field_name,
                receiver.start_position().row as u32 + 1,
            );
            Some(())
        })();

        // Step past the whole chain either way, so `a.b.c` does not also
        // register `b.c`.
        index += if matched.is_some() { 3 } else { 1 };
    }
}
