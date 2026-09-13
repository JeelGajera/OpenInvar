//! The working set, and what it honestly costs.
//!
//! The token figures here are byte-based estimates, not a tokenizer's count,
//! and the tests treat them as such: they assert direction and structure, not
//! exact figures a model would charge. What they do assert exactly is that
//! nothing is silently dropped — a truncated working set that looks complete
//! is worse than one that says what is missing, because an agent reasons from
//! the absence.

use std::collections::BTreeSet;

use openinvar_core::context::{self, Role};
use openinvar_core::graph::InvarGraph;
use openinvar_core::incremental::replace_file_ir;
use openinvar_core::ir::{
    FileIR, Language, Relationship, RelationshipKind, Resolution, Symbol, SymbolKind,
};

fn symbol(file: &str, name: &str, kind: SymbolKind, signature: Option<&str>) -> Symbol {
    Symbol {
        id: format!("{file}::{name}::{kind:?}").to_lowercase(),
        name: name.to_string(),
        kind,
        language: Language::TypeScript,
        file: file.to_string(),
        line_start: 1,
        line_end: 3,
        signature: signature.map(str::to_string),
    }
}

fn edge(from: &Symbol, to: &Symbol, kind: RelationshipKind, r: Resolution) -> Relationship {
    Relationship {
        from: from.id.clone(),
        to: to.id.clone(),
        kind,
        alias: None,
        properties_accessed: vec![],
        context: "c".to_string(),
        file: from.file.clone(),
        line: 1,
        resolution: r,
    }
}

fn graph_of(files: Vec<(&str, Vec<Symbol>, Vec<Relationship>)>) -> InvarGraph {
    let mut graph = InvarGraph::new();
    for (file, symbols, _) in &files {
        replace_file_ir(
            &mut graph,
            &FileIR {
                file: file.to_string(),
                language: Language::TypeScript,
                symbols: symbols.clone(),
                relationships: vec![],
                diagnostics: vec![],
                re_exports: vec![],
            },
        );
    }
    for (_, _, rels) in &files {
        for r in rels {
            graph.add_relationship(r);
        }
    }
    graph
}

/// subject ← dependent, subject → dependency.
fn neighbourhood() -> (InvarGraph, Symbol) {
    let subject = symbol("src/a.ts", "Subject", SymbolKind::Class, Some("class Subject"));
    let dependency = symbol("src/dep.ts", "helper", SymbolKind::Function, Some("fn helper()"));
    let dependent = symbol("src/use.ts", "caller", SymbolKind::Function, Some("fn caller()"));

    let graph = graph_of(vec![
        (
            "src/a.ts",
            vec![subject.clone()],
            vec![edge(&subject, &dependency, RelationshipKind::Calls, Resolution::Resolved)],
        ),
        ("src/dep.ts", vec![dependency], vec![]),
        (
            "src/use.ts",
            vec![dependent.clone()],
            vec![edge(&dependent, &subject, RelationshipKind::Calls, Resolution::Resolved)],
        ),
    ]);
    (graph, subject)
}

fn subjects(s: &Symbol) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    set.insert(s.id.clone());
    set
}

#[test]
fn a_working_set_carries_the_subject_its_dependencies_and_its_dependents() {
    let (graph, subject) = neighbourhood();
    let set = context::build(&graph, &subjects(&subject), 1, None, Resolution::Resolved);

    let roles: Vec<Role> = set.entries.iter().map(|e| e.role).collect();
    assert!(roles.contains(&Role::Subject));
    assert!(roles.contains(&Role::Dependency));
    assert!(roles.contains(&Role::Dependent));
    assert_eq!(set.entries.len(), 3);
}

#[test]
fn a_signature_is_emitted_rather_than_a_file_body() {
    // The whole trade: the shape of the neighbourhood, not its contents.
    let (graph, subject) = neighbourhood();
    let set = context::build(&graph, &subjects(&subject), 1, None, Resolution::Resolved);
    let rendered = set.render();

    assert!(rendered.contains("class Subject"), "{rendered}");
    assert!(rendered.contains("fn helper()"), "{rendered}");
    assert!(set.estimated_tokens > 0);
}

#[test]
fn a_budget_drops_the_outermost_hops_and_says_how_many() {
    // Silent truncation is the failure that matters: an agent reasons from
    // the absence of a symbol as readily as from its presence.
    let (graph, subject) = neighbourhood();
    let full = context::build(&graph, &subjects(&subject), 1, None, Resolution::Resolved);
    let squeezed = context::build(&graph, &subjects(&subject), 1, Some(8), Resolution::Resolved);

    assert!(squeezed.entries.len() < full.entries.len());
    assert_eq!(
        squeezed.entries.len() + squeezed.omitted,
        full.entries.len(),
        "every dropped entry must be counted"
    );
    assert!(squeezed.estimated_tokens <= 8);
}

#[test]
fn an_external_package_is_not_counted_as_a_file_to_read() {
    // It has no file behind it. Counting one would try to read a path that
    // does not exist and take the whole comparison down with it — which is
    // exactly what happened the first time this ran on a real repository.
    let subject = symbol("src/a.ts", "Subject", SymbolKind::Class, Some("class Subject"));
    let external = symbol("", "serde", SymbolKind::ExternalPackage, None);
    let graph = graph_of(vec![
        (
            "src/a.ts",
            vec![subject.clone()],
            vec![edge(&subject, &external, RelationshipKind::Imports, Resolution::Resolved)],
        ),
        ("", vec![external], vec![]),
    ]);

    let set = context::build(&graph, &subjects(&subject), 1, None, Resolution::Resolved);
    assert!(
        !set.files.contains(""),
        "an external package has no file to read: {:?}",
        set.files
    );
    assert!(set.render().contains("(external package)"), "{}", set.render());
}

#[test]
fn structural_evidence_marks_the_neighbourhood_incomplete() {
    let subject = symbol("src/a.ts", "Subject", SymbolKind::Class, Some("class Subject"));
    let other = symbol("src/b.ts", "other", SymbolKind::Function, Some("fn other()"));
    let graph = graph_of(vec![
        (
            "src/a.ts",
            vec![subject.clone()],
            vec![edge(&subject, &other, RelationshipKind::Calls, Resolution::Structural)],
        ),
        ("src/b.ts", vec![other], vec![]),
    ]);

    let set = context::build(&graph, &subjects(&subject), 1, None, Resolution::Structural);
    assert!(!set.gate_safe, "a structural edge cannot see across files");
}

#[test]
fn a_tests_edge_is_not_followed_twice() {
    // The `tests` edge is derived from a call that is already in the walk, so
    // following it would list the same neighbour twice and spend budget on a
    // duplicate.
    let subject = symbol("src/a.ts", "Subject", SymbolKind::Class, Some("class Subject"));
    let test = symbol("src/a.test.ts", "covers", SymbolKind::Function, Some("fn covers()"));
    let graph = graph_of(vec![
        ("src/a.ts", vec![subject.clone()], vec![]),
        (
            "src/a.test.ts",
            vec![test.clone()],
            vec![
                edge(&test, &subject, RelationshipKind::Calls, Resolution::Resolved),
                edge(&test, &subject, RelationshipKind::Tests, Resolution::Resolved),
            ],
        ),
    ]);

    let set = context::build(&graph, &subjects(&subject), 1, None, Resolution::Resolved);
    let occurrences = set.entries.iter().filter(|e| e.name == "covers").count();
    assert_eq!(occurrences, 1, "the test appeared twice: {:?}", set.entries);
}

#[test]
fn an_unreadable_file_is_excluded_rather_than_counted_as_zero() {
    // A file silently treated as zero makes the saving look larger than it
    // is, and the saving is this feature's entire claim.
    let mut files = BTreeSet::new();
    files.insert("definitely/not/here.ts".to_string());
    let cost = context::whole_file_tokens(std::path::Path::new("/nonexistent"), &files);

    assert_eq!(cost.tokens, 0);
    assert_eq!(cost.files_read, 0);
    assert_eq!(cost.files_unreadable, 1);
}

#[test]
fn a_working_set_is_reproducible() {
    let (graph, subject) = neighbourhood();
    let ids = subjects(&subject);
    let first = context::build(&graph, &ids, 2, None, Resolution::Resolved);
    for _ in 0..5 {
        assert_eq!(
            context::build(&graph, &ids, 2, None, Resolution::Resolved),
            first,
            "two identical questions must cost the same"
        );
    }
}
