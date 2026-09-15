//! Counting assertions, during the parse that is already happening.
//!
//! # Why this exists here rather than in a detector
//!
//! `assertion-removal` was specified early and deliberately held back, with
//! this reason recorded in `audit::detectors::held_back`:
//!
//! > assertion calls are not in the graph […] Counting them would mean parsing
//! > source text outside the graph, which is a second source of truth.
//!
//! That reasoning was right about the constraint and points straight at the
//! fix. The problem was never counting assertions; it was counting them
//! *somewhere else*. A second pass over source text would drift from the graph
//! the moment the two disagreed, and a gate cannot arbitrate between two
//! stories about the same file.
//!
//! So the count is taken from the same tree-sitter tree the symbols come from,
//! in the same pass, and travels with them. There is still exactly one source
//! of truth.
//!
//! # What counts, and why the list is short
//!
//! Only forms that are unambiguously assertions in their language. Not
//! `unwrap()`, which is ordinary code as often as it is a check; not `panic!`,
//! which is a failure path rather than a verification; not bare `if x != y`,
//! which is a conditional that happens to appear in a test.
//!
//! A narrow list is the point. This feeds a detector that accuses someone of
//! weakening a test, and a count that drifts because `unwrap()` was refactored
//! away would make that accusation on ordinary work.
//!
//! # Not counted is not zero
//!
//! A language this module does not recognise has *unknown* assertions, not
//! none. The two must never collapse: "this test has no assertions left" and
//! "this build cannot see this test's assertions" lead to opposite conclusions,
//! and only the first is a finding. [`counts_assertions`] is the predicate that
//! keeps them apart, and callers are expected to consult it before reading an
//! absent count as zero.

use std::collections::BTreeMap;

use openinvar_core::ir::{Language, Symbol};
use tree_sitter::{Node, Tree};

/// Whether this build can count assertions in `language`.
///
/// An absent count means zero **only** when this returns true. Where it
/// returns false, nothing is known about the assertions in the file, and a
/// detector must not draw a conclusion from their absence.
pub fn counts_assertions(language: &Language) -> bool {
    matches!(
        language,
        Language::Rust
            | Language::Python
            | Language::TypeScript
            | Language::JavaScript
            | Language::Go
            | Language::C
            | Language::Cpp
    )
}

/// Assertion counts per symbol id, for the symbols that have any.
///
/// Only non-zero counts are recorded. A symbol in a counted language with no
/// entry has no assertions; a symbol in an uncounted language has an unknown
/// number, which is why [`counts_assertions`] has to be consulted rather than
/// the map alone.
pub fn count_by_symbol(
    tree: &Tree,
    source: &str,
    language: &Language,
    symbols: &[Symbol],
) -> BTreeMap<String, u32> {
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    if !counts_assertions(language) {
        return counts;
    }

    for line in assertion_lines(tree, source, language) {
        if let Some(owner) = owning_symbol(symbols, line) {
            *counts.entry(owner.id.clone()).or_insert(0) += 1;
        }
    }

    counts
}

/// The symbol an assertion on `line` belongs to.
///
/// # Why this is not simply "the symbol whose span contains the line"
///
/// Because not every adapter records a span. The TypeScript extractor reports
/// `line_start == line_end` for a function that covers twenty lines, so
/// containment finds nothing and every assertion in a TypeScript test would be
/// silently dropped — the detector would then see a test go from zero
/// assertions to zero and conclude, wrongly but quietly, that nothing changed.
///
/// So containment is preferred where a real span exists, and the nearest
/// preceding declaration is the fallback where one does not. Both orderings are
/// total and derived from the symbols themselves, so the result does not depend
/// on the order the adapter happened to emit them in.
fn owning_symbol(symbols: &[Symbol], line: u32) -> Option<&Symbol> {
    // The innermost containing symbol. A test method inside a class is the
    // thing that was weakened, not the class: crediting the outer symbol would
    // make one changed method look like a change to every sibling.
    let contained = symbols
        .iter()
        .filter(|s| s.line_start <= line && line <= s.line_end && s.line_end > s.line_start)
        .min_by(|a, b| {
            (a.line_end - a.line_start)
                .cmp(&(b.line_end - b.line_start))
                .then_with(|| a.id.cmp(&b.id))
        });
    if contained.is_some() {
        return contained;
    }

    // No usable span: take the closest declaration at or above the line, and
    // prefer a real symbol over the synthetic per-file module, which stands for
    // the file rather than for anything a person wrote.
    symbols
        .iter()
        .filter(|s| s.line_start <= line)
        .max_by(|a, b| {
            a.line_start
                .cmp(&b.line_start)
                .then_with(|| {
                    let real = |s: &Symbol| !matches!(s.kind, openinvar_core::ir::SymbolKind::Module);
                    real(a).cmp(&real(b))
                })
                .then_with(|| b.id.cmp(&a.id))
        })
}

/// The 1-based lines carrying a recognised assertion, in ascending order.
///
/// Lines rather than nodes: the caller matches them against symbol spans, and
/// a line is what a reader is pointed at. Duplicates are kept — two assertions
/// on one line are two assertions.
fn assertion_lines(tree: &Tree, source: &str, language: &Language) -> Vec<u32> {
    let mut lines = Vec::new();
    let mut cursor = tree.walk();
    let mut stack = vec![tree.root_node()];

    while let Some(node) = stack.pop() {
        if is_assertion(&node, source, language) {
            lines.push(node.start_position().row as u32 + 1);
        }
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    lines.sort_unstable();
    lines
}

fn is_assertion(node: &Node<'_>, source: &str, language: &Language) -> bool {
    match language {
        // `assert x`, and unittest's `self.assertEqual(...)` family.
        Language::Python => {
            node.kind() == "assert_statement"
                || (node.kind() == "call" && callee(node, source).is_some_and(python_assert))
        }

        // Only the assert macro family. Expanded macro bodies are token trees
        // this project does not walk into, but the invocation itself is a node.
        Language::Rust => {
            node.kind() == "macro_invocation"
                && macro_name(node, source).is_some_and(|name| {
                    name == "assert" || name.starts_with("assert_") || name.starts_with("debug_assert")
                })
        }

        Language::TypeScript | Language::JavaScript => {
            node.kind() == "call_expression" && callee(node, source).is_some_and(js_assert)
        }

        // testify's assert/require, plus the standard library's own mechanism:
        // in Go a failed check *is* `t.Error`/`t.Fatal`, so omitting them would
        // mean counting nothing in most real Go tests.
        Language::Go => {
            node.kind() == "call_expression" && callee(node, source).is_some_and(go_assert)
        }

        Language::C | Language::Cpp => {
            node.kind() == "call_expression"
                && callee(node, source).is_some_and(|c| c == "assert" || c == "static_assert")
        }

        _ => false,
    }
}

fn python_assert(callee: &str) -> bool {
    let last = callee.rsplit('.').next().unwrap_or(callee);
    // `assertEqual`, `assertRaises`, …; plus a bare `assertSomething()`.
    last.starts_with("assert") && last != "assert"
}

fn js_assert(callee: &str) -> bool {
    let root = callee.split('.').next().unwrap_or(callee);
    // `expect(x).toBe(y)` registers on the inner `expect(x)` call, so the outer
    // matcher call is not also counted — one assertion, counted once.
    root == "expect" || root == "assert" || root == "chai"
}

fn go_assert(callee: &str) -> bool {
    let mut parts = callee.split('.');
    let root = parts.next().unwrap_or_default();
    let method = parts.next().unwrap_or_default();

    if root == "assert" || root == "require" {
        return true;
    }
    // `t.Error`, `t.Fatalf`, and the rest of the family, on the conventional
    // receiver name. A differently named receiver is missed, which is a miss
    // rather than a false accusation.
    matches!(root, "t" | "b")
        && (method.starts_with("Error") || method.starts_with("Fatal"))
}

/// The text of the thing being called, for a call node.
///
/// A callee containing `(` is a chained matcher — `expect(x).toBe`,
/// `chai.expect(y).to.equal` — sitting on top of a call that was already
/// counted. Rejecting it here is what keeps one assertion worth one, and it has
/// to be here rather than in each language's predicate: `chai.expect(y).to.equal`
/// still begins with `chai`, so the root-segment test cannot tell the two apart.
fn callee<'a>(node: &Node<'_>, source: &'a str) -> Option<&'a str> {
    let function = node.child_by_field_name("function")?;
    let text = source.get(function.start_byte()..function.end_byte())?;
    if text.contains('(') {
        return None;
    }
    Some(text)
}

/// The macro's name, for a Rust `macro_invocation`.
fn macro_name<'a>(node: &Node<'_>, source: &'a str) -> Option<&'a str> {
    let name = node.child_by_field_name("macro")?;
    source.get(name.start_byte()..name.end_byte())
}

// Gated on the feature whose grammar the tests parse with. A slim build for
// another language compiles this crate on its own, and a test that reaches for
// an absent grammar breaks a build that has nothing to do with it — the exact
// failure `ci.yml` records for the per-language clippy job.
#[cfg(all(test, feature = "rust"))]
mod tests {
    use super::*;

    fn parse(source: &str, language: &Language) -> Tree {
        let mut parser = tree_sitter::Parser::new();
        let ts_language = match language {
            Language::Rust => tree_sitter_rust::LANGUAGE.into(),
            _ => panic!("test only parses Rust directly"),
        };
        parser.set_language(&ts_language).expect("set language");
        parser.parse(source, None).expect("parse")
    }

    #[test]
    fn rust_assert_macros_are_counted() {
        let source = "fn t() {\n    assert!(a);\n    assert_eq!(a, b);\n    debug_assert!(c);\n}\n";
        let tree = parse(source, &Language::Rust);
        let lines = assertion_lines(&tree, source, &Language::Rust);
        assert_eq!(lines, vec![2, 3, 4], "expected three assert macros");
    }

    #[test]
    fn ordinary_rust_code_counts_nothing() {
        // unwrap and panic are deliberately not assertions: unwrap is ordinary
        // code as often as it is a check, and refactoring one away would look
        // like a weakened test.
        let source = "fn t() {\n    let x = y.unwrap();\n    panic!(\"no\");\n    foo();\n}\n";
        let tree = parse(source, &Language::Rust);
        assert!(
            assertion_lines(&tree, source, &Language::Rust).is_empty(),
            "unwrap or panic was counted as an assertion"
        );
    }

    #[test]
    fn an_uncounted_language_reports_nothing_rather_than_zero() {
        // The predicate is the thing that keeps "no assertions" apart from
        // "cannot see assertions"; the empty map alone cannot say which.
        assert!(!counts_assertions(&Language::Java));
        assert!(counts_assertions(&Language::Rust));
    }

    #[test]
    fn the_innermost_symbol_owns_the_assertion() {
        let source = "fn outer() {\n    fn inner() {\n        assert!(x);\n    }\n}\n";
        let tree = parse(source, &Language::Rust);
        let symbols = vec![
            Symbol {
                id: "outer".into(),
                name: "outer".into(),
                kind: openinvar_core::ir::SymbolKind::Function,
                language: Language::Rust,
                file: "a.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: None,
            },
            Symbol {
                id: "inner".into(),
                name: "inner".into(),
                kind: openinvar_core::ir::SymbolKind::Function,
                language: Language::Rust,
                file: "a.rs".into(),
                line_start: 2,
                line_end: 4,
                signature: None,
            },
        ];

        let counts = count_by_symbol(&tree, source, &Language::Rust, &symbols);
        assert_eq!(counts.get("inner"), Some(&1));
        assert_eq!(
            counts.get("outer"),
            None,
            "the enclosing function was credited with its child's assertion"
        );
    }

    #[test]
    fn a_symbol_with_no_recorded_span_still_owns_its_assertions() {
        // The TypeScript extractor reports line_start == line_end for a
        // function covering twenty lines. Requiring containment dropped every
        // assertion in a TypeScript test on the floor, and the detector then
        // saw zero before and zero after and concluded, quietly, that nothing
        // had changed. Found by counting a real TypeScript file, not in review.
        let symbols = vec![Symbol {
            id: "t::testThing::function".into(),
            name: "testThing".into(),
            kind: openinvar_core::ir::SymbolKind::Function,
            language: Language::TypeScript,
            file: "a.test.ts".into(),
            line_start: 1,
            line_end: 1,
            signature: None,
        }];

        assert_eq!(
            owning_symbol(&symbols, 4).map(|s| s.id.as_str()),
            Some("t::testThing::function"),
            "an assertion below a symbol with no span found no owner"
        );
    }

    #[test]
    fn the_synthetic_module_symbol_loses_to_a_real_one() {
        let module = Symbol {
            id: "a::module".into(),
            name: "module".into(),
            kind: openinvar_core::ir::SymbolKind::Module,
            language: Language::TypeScript,
            file: "a.ts".into(),
            line_start: 1,
            line_end: 1,
            signature: None,
        };
        let function = Symbol {
            kind: openinvar_core::ir::SymbolKind::Function,
            id: "a::testThing::function".into(),
            name: "testThing".into(),
            ..module.clone()
        };

        // Both start on line 1, so the tiebreak decides. Crediting the file's
        // synthetic module symbol would attribute the assertion to something
        // nobody wrote.
        assert_eq!(
            owning_symbol(&[module.clone(), function.clone()], 3).map(|s| s.id.as_str()),
            Some("a::testThing::function")
        );
        // Order-independent: the adapter's emission order must not decide.
        assert_eq!(
            owning_symbol(&[function, module], 3).map(|s| s.id.as_str()),
            Some("a::testThing::function")
        );
    }

    #[test]
    fn a_chained_matcher_counts_once() {
        // `chai.expect(y).to.equal(2)` is two call nodes, and the outer one's
        // callee still begins with `chai`, so the root-segment test alone
        // counted the same assertion twice.
        assert!(
            callee_text_is_chained("chai.expect(y).to.equal"),
            "a chained matcher was not recognised as chained"
        );
        assert!(
            callee_text_is_chained("expect(compute(1)).toBe"),
            "a chained matcher was not recognised as chained"
        );
        assert!(!callee_text_is_chained("chai.expect"));
        assert!(!callee_text_is_chained("expect"));
        assert!(!callee_text_is_chained("assert.ok"));
    }

    /// Mirrors the rejection in [`callee`], which cannot be called without a
    /// node.
    fn callee_text_is_chained(text: &str) -> bool {
        text.contains('(')
    }

    #[test]
    fn counting_is_stable_across_runs() {
        // The map is a BTreeMap and the lines are sorted, so two runs over one
        // tree agree. Determinism is the property every claim here rests on.
        let source = "fn t() {\n    assert!(a);\n    assert_eq!(b, c);\n}\n";
        let tree = parse(source, &Language::Rust);
        let first = assertion_lines(&tree, source, &Language::Rust);
        for _ in 0..5 {
            assert_eq!(assertion_lines(&tree, source, &Language::Rust), first);
        }
    }
}
