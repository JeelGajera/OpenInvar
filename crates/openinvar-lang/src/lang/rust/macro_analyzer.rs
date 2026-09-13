//! Derive macros as implementation edges.
//!
//! `#[derive(Serialize)]` generates a trait impl that never appears in the
//! source, so a purely syntactic pass misses it entirely. That matters for
//! impact analysis: changing a struct's fields changes its generated
//! `Serialize` output, and a caller reading that wire format depends on the
//! struct even though nothing in either file names the other.
//!
//! This module previously returned an empty vector while the README advertised
//! derive-macro support.

use openinvar_core::ast::node_text;
use tree_sitter::Node;

/// Traits provided by the language or its prelude.
///
/// These are derived on nearly every type and resolve to nothing in the
/// repository, so recording them would add an edge per struct to a node that
/// tells the user nothing.
const STD_DERIVES: &[&str] = &[
    "Debug",
    "Clone",
    "Copy",
    "PartialEq",
    "Eq",
    "PartialOrd",
    "Ord",
    "Hash",
    "Default",
];

/// A trait named in a `#[derive(..)]` on an item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedTrait {
    /// The trait name as written, with any path qualification stripped.
    pub name: String,
    /// Line of the `#[derive(..)]` attribute.
    pub line: u32,
    /// True for traits from the standard prelude, which callers usually skip.
    pub is_std: bool,
}

/// Every trait derived on `item`, read from the attributes preceding it.
///
/// tree-sitter-rust models attributes as siblings that precede the item rather
/// than as children of it, so this walks backwards from the item until it runs
/// out of attributes.
pub fn derived_traits(item: Node<'_>, source: &[u8]) -> Vec<DerivedTrait> {
    let mut out = Vec::new();
    let mut sibling = item.prev_sibling();

    while let Some(node) = sibling {
        match node.kind() {
            "attribute_item" => {
                collect_from_attribute(node, source, &mut out);
                sibling = node.prev_sibling();
            }
            // Doc comments and ordinary comments may sit between attributes.
            "line_comment" | "block_comment" => sibling = node.prev_sibling(),
            _ => break,
        }
    }

    // Walking backwards reverses source order; restore it so output is stable.
    out.reverse();
    out
}

fn collect_from_attribute(attr_item: Node<'_>, source: &[u8], out: &mut Vec<DerivedTrait>) {
    // `attribute_item` wraps the attribute in `#[` … `]`; the grammar does not
    // expose it as a named field, so it is located by kind.
    let mut cursor = attr_item.walk();
    let attribute = attr_item
        .children(&mut cursor)
        .find(|child| child.kind() == "attribute");
    let Some(attribute) = attribute else {
        return;
    };

    // The grammar exposes the attribute's arguments as a field but not its
    // name, which is the first child identifier: `derive(Debug, Serialize)`.
    let is_derive = attribute
        .named_child(0)
        .and_then(|n| node_text(n, source))
        .map(|name| name == "derive")
        .unwrap_or(false);
    if !is_derive {
        return;
    }

    let Some(arguments) = attribute.child_by_field_name("arguments") else {
        return;
    };
    let line = attr_item.start_position().row as u32 + 1;

    // The argument list is an unparsed token tree; the trait names are its
    // identifier tokens. Paths like `serde::Serialize` arrive as separate
    // tokens, so only the final identifier of each run is kept.
    let mut cursor = arguments.walk();
    let mut pending: Option<String> = None;
    for token in arguments.children(&mut cursor) {
        match token.kind() {
            "identifier" => {
                if let Some(text) = node_text(token, source) {
                    pending = Some(text.to_string());
                }
            }
            "::" => {} // keep the following identifier, discard what came before
            "," | "(" | ")" => {
                if let Some(name) = pending.take() {
                    out.push(DerivedTrait {
                        is_std: STD_DERIVES.contains(&name.as_str()),
                        name,
                        line,
                    });
                }
            }
            _ => {}
        }
    }
    if let Some(name) = pending {
        out.push(DerivedTrait {
            is_std: STD_DERIVES.contains(&name.as_str()),
            name,
            line,
        });
    }
}
