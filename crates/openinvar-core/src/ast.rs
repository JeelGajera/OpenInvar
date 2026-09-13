//! Shared tree-sitter traversal primitives.
//!
//! Behind the `ast` feature so that consumers who only read the graph — the
//! store, the query layer — do not pull a parser they never call.
//!
//! Every adapter previously carried its own copy of a recursive `walk_tree`.
//! Recursion is the wrong shape here: node depth is attacker- and
//! generator-controlled (a machine-generated initializer nests one level per
//! element), adapters parse files on `rayon` workers whose stacks are smaller
//! than the main thread's, and a stack overflow aborts the process rather than
//! unwinding, so one pathological file would take down an entire `analyze` run.
//! [`walk`] is iterative and depth-bounded instead, and reports what it skipped
//! so callers can turn truncation into a diagnostic rather than a silent gap.

use tree_sitter::Node;

/// Maximum node depth visited by [`walk`].
///
/// Hand-written code rarely exceeds ~100; the limit exists to bound generated
/// and adversarial input, not to constrain real source.
pub const MAX_TREE_DEPTH: usize = 512;

/// What a [`walk`] actually covered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WalkStats {
    /// Nodes handed to the visitor.
    pub visited: usize,
    /// Subtrees left unvisited because they sat below [`MAX_TREE_DEPTH`].
    pub skipped_subtrees: usize,
}

impl WalkStats {
    /// True if any subtree was skipped, meaning extraction for this file is
    /// incomplete and the caller should say so.
    pub fn truncated(&self) -> bool {
        self.skipped_subtrees > 0
    }
}

/// Visit `root` and all its descendants in pre-order, depth-first.
///
/// Iterative and allocation-free: it drives a single [`tree_sitter::TreeCursor`]
/// rather than recursing or buffering children. Sibling order is preserved, so
/// the visit sequence matches source order and results are reproducible.
///
/// The walk never escapes the subtree rooted at `root`, even when `root` has
/// siblings in the wider tree.
pub fn walk<'t, F>(root: Node<'t>, visit: &mut F) -> WalkStats
where
    F: FnMut(Node<'t>),
{
    let mut stats = WalkStats::default();
    let mut cursor = root.walk();
    let mut depth = 0usize;

    loop {
        visit(cursor.node());
        stats.visited += 1;

        if depth < MAX_TREE_DEPTH {
            if cursor.goto_first_child() {
                depth += 1;
                continue;
            }
        } else if cursor.node().child_count() > 0 {
            stats.skipped_subtrees += 1;
        }

        // Walk up until there is a sibling to move to. Stopping at depth 0
        // keeps us inside `root`'s subtree.
        loop {
            if depth == 0 {
                return stats;
            }
            if cursor.goto_next_sibling() {
                break;
            }
            cursor.goto_parent();
            depth -= 1;
        }
    }
}

/// The source text a node spans, or `None` if it is not valid UTF-8.
pub fn node_text<'a>(node: Node<'_>, source: &'a [u8]) -> Option<&'a str> {
    node.utf8_text(source).ok()
}

/// The text of a named field of `node`.
pub fn field_text<'a>(node: Node<'_>, field: &str, source: &'a [u8]) -> Option<&'a str> {
    node_text(node.child_by_field_name(field)?, source)
}

/// The 1-based line a node starts on, for `Symbol` and `Relationship` records.
pub fn start_line(node: Node<'_>) -> u32 {
    node.start_position().row as u32 + 1
}

/// The 1-based line a node ends on.
pub fn end_line(node: Node<'_>) -> u32 {
    node.end_position().row as u32 + 1
}

/// The first line of a node's text, for use as a symbol signature.
///
/// Trailing whitespace is trimmed; the body of a multi-line definition is
/// dropped, so the result stays a readable one-line summary.
pub fn first_line_of(node: Node<'_>, source: &[u8]) -> Option<String> {
    let text = node_text(node, source)?;
    Some(text.lines().next().unwrap_or("").trim_end().to_string())
}

/// True if the subtree contains a node tree-sitter could not parse.
///
/// tree-sitter always returns a tree; syntax it cannot handle becomes `ERROR`
/// or `MISSING` nodes. Adapters use this to emit a parse diagnostic instead of
/// silently extracting from a partial tree.
pub fn has_parse_error(root: Node<'_>) -> bool {
    // `has_error` covers the whole subtree and is maintained by the parser, so
    // this is O(1) rather than a traversal.
    root.has_error()
}

/// The first `ERROR` or `MISSING` node in the subtree, for locating a
/// parse diagnostic on a line.
pub fn first_error_line(root: Node<'_>) -> Option<u32> {
    if !root.has_error() {
        return None;
    }
    let mut line = None;
    walk(root, &mut |node| {
        if line.is_none() && (node.is_error() || node.is_missing()) {
            line = Some(start_line(node));
        }
    });
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_rust(source: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::language())
            .expect("rust grammar loads");
        parser.parse(source, None).expect("parser returns a tree")
    }

    #[test]
    fn walk_visits_in_source_order() {
        let tree = parse_rust("struct A; struct B; struct C;");
        let src = "struct A; struct B; struct C;".as_bytes();
        let mut names = Vec::new();
        walk(tree.root_node(), &mut |node| {
            if node.kind() == "type_identifier" {
                if let Some(t) = node_text(node, src) {
                    names.push(t.to_string());
                }
            }
        });
        assert_eq!(
            names,
            vec!["A", "B", "C"],
            "sibling order must be preserved"
        );
    }

    #[test]
    fn walk_covers_the_whole_subtree() {
        let tree = parse_rust("fn f() { let x = Foo { a: 1 }; }");
        let mut count = 0usize;
        let stats = walk(tree.root_node(), &mut |_| count += 1);
        assert_eq!(stats.visited, count);
        assert!(count > 10, "a non-trivial function has many nodes");
        assert!(!stats.truncated());
    }

    #[test]
    fn walk_bounds_pathological_nesting_instead_of_overflowing() {
        // Deeply nested parentheses: one AST level per pair. A recursive walker
        // overflows here on a worker thread; this must return normally.
        let depth = MAX_TREE_DEPTH * 4;
        let source = format!(
            "fn f() {{ let x = {}1{}; }}",
            "(".repeat(depth),
            ")".repeat(depth)
        );
        let tree = parse_rust(&source);
        let stats = walk(tree.root_node(), &mut |_| {});
        assert!(
            stats.truncated(),
            "input nests deeper than the limit, so truncation must be reported"
        );
        assert!(stats.visited >= MAX_TREE_DEPTH);
    }

    #[test]
    fn walk_stays_inside_the_requested_subtree() {
        let source = "struct A; struct B;";
        let tree = parse_rust(source);
        let first = tree.root_node().child(0).expect("first item exists");

        let mut seen = Vec::new();
        walk(first, &mut |node| {
            if node.kind() == "type_identifier" {
                if let Some(t) = node_text(node, source.as_bytes()) {
                    seen.push(t.to_string());
                }
            }
        });
        assert_eq!(
            seen,
            vec!["A"],
            "walking one item must not reach its sibling"
        );
    }

    #[test]
    fn parse_errors_are_detected_and_located() {
        let tree = parse_rust("fn broken( {");
        assert!(has_parse_error(tree.root_node()));
        assert!(first_error_line(tree.root_node()).is_some());

        let clean = parse_rust("fn ok() {}");
        assert!(!has_parse_error(clean.root_node()));
        assert_eq!(first_error_line(clean.root_node()), None);
    }
}
