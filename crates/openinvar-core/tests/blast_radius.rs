use openinvar_core::error::InvarError;
use openinvar_core::graph::InvarGraph;
use openinvar_core::incremental::replace_file_ir;
use openinvar_core::ir::{FileIR, Language, Relationship, RelationshipKind, Resolution, Symbol, SymbolKind};
use openinvar_core::query::{blast_radius, dependencies, symbol_usages, RelationshipKindMask};

fn symbol(id: &str, name: &str, file: &str, kind: SymbolKind) -> Symbol {
    Symbol {
        id: id.to_string(),
        name: name.to_string(),
        kind,
        language: Language::TypeScript,
        file: file.to_string(),
        line_start: 1,
        line_end: 1,
        signature: None,
    }
}

fn rel(from: &str, to: &str, file: &str, line: u32) -> Relationship {
    Relationship {
        from: from.to_string(),
        to: to.to_string(),
        kind: RelationshipKind::Imports,
        alias: None,
        properties_accessed: vec![],
        context: "import".to_string(),
        file: file.to_string(),
        line,
        resolution: Resolution::Resolved,
    }
}

#[test]
fn test_blast_radius_depth_and_direction() {
    let mut graph = InvarGraph::new();

    let a = symbol("a.ts::A::class", "A", "a.ts", SymbolKind::Class);
    let b = symbol("b.ts::B::class", "B", "b.ts", SymbolKind::Class);
    let c = symbol("c.ts::C::class", "C", "c.ts", SymbolKind::Class);

    graph.add_symbol(a.clone());
    graph.add_symbol(b.clone());
    graph.add_symbol(c.clone());

    graph.add_relationship(&rel(&b.id, &a.id, "b.ts", 10));
    graph.add_relationship(&rel(&c.id, &b.id, "c.ts", 20));

    let depth_1 =
        blast_radius(&graph, "A", None, Some(1), RelationshipKindMask::all(), Resolution::Structural).expect("depth1 ok");
    assert_eq!(depth_1.len(), 1);
    assert_eq!(depth_1[0].from, b.id);

    let depth_2 =
        blast_radius(&graph, "A", None, Some(2), RelationshipKindMask::all(), Resolution::Structural).expect("depth2 ok");
    assert_eq!(depth_2.len(), 2);
    assert_eq!(depth_2[0].hop, 1);
    assert_eq!(depth_2[1].hop, 2);

    let deps =
        dependencies(&graph, "C", None, Some(2), RelationshipKindMask::all(), Resolution::Structural).expect("deps ok");
    assert_eq!(deps.len(), 2);
    assert_eq!(deps[0].to, "b.ts::B::class");
    assert_eq!(deps[1].to, "a.ts::A::class");
}

#[test]
fn test_symbol_lookup_ambiguity_requires_file_disambiguation() {
    let mut graph = InvarGraph::new();

    let x1 = symbol("a.ts::Thing::class", "Thing", "a.ts", SymbolKind::Class);
    let x2 = symbol("b.ts::Thing::class", "Thing", "b.ts", SymbolKind::Class);

    graph.add_symbol(x1.clone());
    graph.add_symbol(x2.clone());

    let err = blast_radius(&graph, "Thing", None, Some(1), RelationshipKindMask::all(), Resolution::Structural)
        .expect_err("must be ambiguous");
    match err {
        InvarError::AmbiguousSymbol { symbol, candidates } => {
            assert_eq!(symbol, "Thing");
            assert_eq!(candidates, vec!["a.ts".to_string(), "b.ts".to_string()]);
        }
        other => panic!("unexpected error: {other:?}"),
    }

    let ok = blast_radius(
        &graph,
        "Thing",
        Some("a.ts"),
        Some(1),
        RelationshipKindMask::all(),
        Resolution::Structural,
    );
    assert!(ok.is_ok());
}

#[test]
fn test_symbol_usages_dedupes_by_file_line() {
    let mut graph = InvarGraph::new();

    let target = symbol("t.ts::Target::class", "Target", "t.ts", SymbolKind::Class);
    let user = symbol("u.ts::User::class", "User", "u.ts", SymbolKind::Class);

    graph.add_symbol(target.clone());
    graph.add_symbol(user.clone());

    let mut r1 = rel(&user.id, &target.id, "u.ts", 30);
    r1.alias = Some("AliasT".to_string());
    let r2 = r1.clone();

    graph.add_relationship(&r1);
    graph.add_relationship(&r2);

    let usages = symbol_usages(&graph, "Target", None, true, RelationshipKindMask::all(), Resolution::Structural)
        .expect("usages ok");
    assert_eq!(usages.len(), 1);
    assert_eq!(usages[0].line, 30);
}

#[test]
fn test_symbol_usages_respects_include_aliases_flag() {
    let mut graph = InvarGraph::new();

    let target = symbol("t.ts::Target::class", "Target", "t.ts", SymbolKind::Class);
    let direct = symbol(
        "d.ts::DirectUser::class",
        "DirectUser",
        "d.ts",
        SymbolKind::Class,
    );
    let alias_user = symbol(
        "a.ts::AliasUser::class",
        "AliasUser",
        "a.ts",
        SymbolKind::Class,
    );

    graph.add_symbol(target.clone());
    graph.add_symbol(direct.clone());
    graph.add_symbol(alias_user.clone());

    let mut direct_rel = rel(&direct.id, &target.id, "d.ts", 10);
    direct_rel.alias = None;
    let mut alias_rel = rel(&alias_user.id, &target.id, "a.ts", 20);
    alias_rel.alias = Some("AliasT".to_string());

    graph.add_relationship(&direct_rel);
    graph.add_relationship(&alias_rel);

    let with_aliases = symbol_usages(&graph, "Target", None, true, RelationshipKindMask::all(), Resolution::Structural)
        .expect("with aliases");
    assert_eq!(with_aliases.len(), 2);

    let without_aliases = symbol_usages(&graph, "Target", None, false, RelationshipKindMask::all(), Resolution::Structural)
        .expect("without aliases");
    assert_eq!(without_aliases.len(), 1);
    assert_eq!(without_aliases[0].from, direct.id);
    assert!(without_aliases[0].alias.is_none());
}

#[test]
fn test_incremental_replace_file_preserves_indexes() {
    let mut graph = InvarGraph::new();

    let old_symbol = symbol("x.ts::X::class", "X", "x.ts", SymbolKind::Class);
    graph.add_symbol(old_symbol);

    let file_ir = FileIR {
        file: "x.ts".to_string(),
        language: Language::TypeScript,
        symbols: vec![symbol("x.ts::X2::class", "X2", "x.ts", SymbolKind::Class)],
        relationships: vec![],
        diagnostics: vec![],
        re_exports: vec![],
        assertions: Default::default(),
        assertions_counted: false,
    };

    let result = replace_file_ir(&mut graph, &file_ir);
    assert_eq!(
        result.removed_symbol_ids,
        vec!["x.ts::X::class".to_string()]
    );
    assert_eq!(result.added_symbol_ids, vec!["x.ts::X2::class".to_string()]);
    assert_eq!(result.removed_relationships, 0);
    assert_eq!(result.added_relationships, 0);

    assert!(graph.symbols.get("x.ts::X::class").is_none());
    assert!(graph.symbols.get("x.ts::X2::class").is_some());
}

#[test]
fn test_invalid_depth_is_rejected() {
    let graph = InvarGraph::new();
    let err = blast_radius(&graph, "Any", None, Some(11), RelationshipKindMask::all(), Resolution::Structural)
        .expect_err("invalid depth");
    match err {
        InvarError::InvalidDepth { depth, max } => {
            assert_eq!(depth, 11);
            assert_eq!(max, 10);
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn test_remove_file_keeps_remaining_node_indexes_valid() {
    let mut graph = InvarGraph::new();

    let a = symbol("a.ts::A::class", "A", "a.ts", SymbolKind::Class);
    let b = symbol("b.ts::B::class", "B", "b.ts", SymbolKind::Class);
    let c = symbol("c.ts::C::class", "C", "c.ts", SymbolKind::Class);
    graph.add_symbol(a.clone());
    graph.add_symbol(b.clone());
    graph.add_symbol(c.clone());

    graph.add_relationship(&rel(&b.id, &a.id, "b.ts", 1));
    graph.add_relationship(&rel(&c.id, &a.id, "c.ts", 2));
    graph.remove_file("b.ts");

    let blast =
        blast_radius(&graph, "A", None, Some(1), RelationshipKindMask::all(), Resolution::Structural).expect("blast ok");
    assert_eq!(blast.len(), 1);
    assert_eq!(blast[0].from, c.id);
}
