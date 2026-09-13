use openinvar_core::graph::InvarGraph;
use openinvar_core::ir::{Language, Relationship, RelationshipKind, Resolution, Symbol, SymbolKind};
use openinvar_core::query::{blast_radius, symbol_usages, RelationshipKindMask};
use openinvar_core::resolver::{AliasResolver, AliasScope};

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

#[test]
fn test_aliased_import_is_tracked_with_property_accesses() {
    let mut graph = InvarGraph::new();

    let user_payload = symbol(
        "src/models/user_payload.ts::UserPayload::class",
        "UserPayload",
        "src/models/user_payload.ts",
        SymbolKind::Class,
    );
    let mapper = symbol(
        "src/mappers/response/deep/view_model_mapper.ts::ViewModelMapper::class",
        "ViewModelMapper",
        "src/mappers/response/deep/view_model_mapper.ts",
        SymbolKind::Class,
    );

    graph.add_symbol(user_payload.clone());
    graph.add_symbol(mapper.clone());

    let relationships = vec![Relationship {
        from: mapper.id.clone(),
        to: user_payload.id.clone(),
        kind: RelationshipKind::Imports,
        alias: Some("ResponseModel".to_string()),
        properties_accessed: vec![
            "userId".to_string(),
            "timestamp".to_string(),
            "status".to_string(),
        ],
        context: "import { UserPayload as ResponseModel } from '../../../models/user_payload';"
            .to_string(),
        file: "src/mappers/response/deep/view_model_mapper.ts".to_string(),
        line: 1,
        resolution: Resolution::Resolved,
    }];

    for relationship in &relationships {
        graph.add_relationship(relationship);
    }

    let resolver = AliasResolver::default();
    resolver.ingest_relationships(&graph, &relationships);

    let aliases = graph
        .alias_chains
        .get(&user_payload.id)
        .expect("alias chain exists");
    assert_eq!(aliases.len(), 1);
    assert_eq!(aliases[0].alias_name, "ResponseModel");
    assert_eq!(aliases[0].scope, AliasScope::ImportAlias);

    let blast = blast_radius(
        &graph,
        "UserPayload",
        None,
        Some(2),
        RelationshipKindMask::all(),
        Resolution::Structural,
    )
    .expect("blast radius succeeds");
    assert_eq!(blast.len(), 1);
    assert_eq!(
        blast[0].file,
        "src/mappers/response/deep/view_model_mapper.ts"
    );
    assert_eq!(blast[0].alias.as_deref(), Some("ResponseModel"));
    assert_eq!(
        blast[0].properties_accessed,
        vec!["userId", "timestamp", "status"]
    );

    let usages = symbol_usages(
        &graph,
        "UserPayload",
        None,
        true,
        RelationshipKindMask::all(),
        Resolution::Structural,
    )
    .expect("usages succeeds");
    assert_eq!(usages.len(), 1);
    assert_eq!(usages[0].alias.as_deref(), Some("ResponseModel"));
}

#[test]
fn test_alias_resolver_supports_reexport_barrel_and_default_scopes() {
    let mut graph = InvarGraph::new();

    let canonical = symbol(
        "src/models/user_payload.ts::UserPayload::class",
        "UserPayload",
        "src/models/user_payload.ts",
        SymbolKind::Class,
    );
    let barrel = symbol(
        "src/models/index.ts::Models::module",
        "Models",
        "src/models/index.ts",
        SymbolKind::Module,
    );

    graph.add_symbol(canonical.clone());
    graph.add_symbol(barrel.clone());

    let relationships = vec![
        Relationship {
            from: barrel.id.clone(),
            to: canonical.id.clone(),
            kind: RelationshipKind::ReExports,
            alias: Some("PublicUser".to_string()),
            properties_accessed: vec![],
            context: "export { UserPayload as PublicUser } from './user_payload'".to_string(),
            file: "src/models/index.ts".to_string(),
            line: 1,
            resolution: Resolution::Resolved,
        },
        Relationship {
            from: barrel.id.clone(),
            to: canonical.id.clone(),
            kind: RelationshipKind::ReExports,
            alias: Some("UserPayload".to_string()),
            properties_accessed: vec![],
            context: "export * from './user_payload'".to_string(),
            file: "src/models/index.ts".to_string(),
            line: 2,
            resolution: Resolution::Resolved,
        },
        Relationship {
            from: barrel.id.clone(),
            to: canonical.id.clone(),
            kind: RelationshipKind::Imports,
            alias: Some("User".to_string()),
            properties_accessed: vec![],
            context: "import User from './user_payload' // default".to_string(),
            file: "src/models/index.ts".to_string(),
            line: 3,
            resolution: Resolution::Resolved,
        },
    ];

    let resolver = AliasResolver::default();
    resolver.ingest_relationships(&graph, &relationships);

    let aliases = graph
        .alias_chains
        .get(&canonical.id)
        .expect("alias chain exists");
    assert_eq!(aliases.len(), 3);
    assert!(aliases
        .iter()
        .any(|a| a.alias_name == "PublicUser" && a.scope == AliasScope::ReExport));
    assert!(aliases
        .iter()
        .any(|a| a.alias_name == "UserPayload" && a.scope == AliasScope::BarrelReExport));
    assert!(aliases
        .iter()
        .any(|a| a.alias_name == "User" && a.scope == AliasScope::DefaultImport));

    assert_eq!(
        resolver
            .resolve_alias_in_file("PublicUser", "src/models/index.ts")
            .as_deref(),
        Some(canonical.id.as_str())
    );
}

#[test]
fn test_accesses_property_edge_can_be_canonicalized_from_alias() {
    let mut graph = InvarGraph::new();
    let canonical = symbol(
        "src/models/user_payload.ts::UserPayload::class",
        "UserPayload",
        "src/models/user_payload.ts",
        SymbolKind::Class,
    );
    let mapper = symbol(
        "src/mappers/response/deep/view_model_mapper.ts::ViewModelMapper::class",
        "ViewModelMapper",
        "src/mappers/response/deep/view_model_mapper.ts",
        SymbolKind::Class,
    );
    graph.add_symbol(canonical.clone());
    graph.add_symbol(mapper.clone());

    let import_rel = Relationship {
        from: mapper.id.clone(),
        to: canonical.id.clone(),
        kind: RelationshipKind::Imports,
        alias: Some("ResponseModel".to_string()),
        properties_accessed: vec![],
        context: "import { UserPayload as ResponseModel } from '../../../models/user_payload';"
            .to_string(),
        file: "src/mappers/response/deep/view_model_mapper.ts".to_string(),
        line: 1,
        resolution: Resolution::Resolved,
    };
    let resolver = AliasResolver::default();
    resolver.ingest_relationships(&graph, &[import_rel]);

    let accesses_property = Relationship {
        from: mapper.id.clone(),
        to: "src/mappers/response/deep/view_model_mapper.ts::ResponseModel::alias".to_string(),
        kind: RelationshipKind::AccessesProperty,
        alias: Some("ResponseModel".to_string()),
        properties_accessed: vec!["userId".to_string()],
        context: "id: data.userId".to_string(),
        file: "src/mappers/response/deep/view_model_mapper.ts".to_string(),
        line: 6,
        resolution: Resolution::Resolved,
    };

    let normalized = resolver.canonicalize_relationship(&accesses_property);
    assert_eq!(normalized.to, canonical.id);
    assert_eq!(normalized.properties_accessed, vec!["userId"]);
}

// ── alias risk classification ────────────────────────────────

use openinvar_core::query::{is_renamed, QueryEdge};

fn edge(to: &str, alias: Option<&str>) -> QueryEdge {
    QueryEdge {
        from: "src/consumer.ts::module::module".to_string(),
        to: to.to_string(),
        kind: RelationshipKind::Imports,
        file: "src/consumer.ts".to_string(),
        line: 1,
        alias: alias.map(str::to_string),
        properties_accessed: Vec::new(),
        context: String::new(),
        hop: 1,
        resolution: Resolution::Resolved,
    }
}

fn graph_with_user_payload() -> InvarGraph {
    let mut graph = InvarGraph::new();
    graph.add_symbol(Symbol {
        id: "src/models/user.ts::UserPayload::class".to_string(),
        name: "UserPayload".to_string(),
        kind: SymbolKind::Class,
        language: Language::TypeScript,
        file: "src/models/user.ts".to_string(),
        line_start: 1,
        line_end: 5,
        signature: None,
    });
    graph
}

#[test]
fn an_alias_matching_the_symbol_name_is_not_a_rename() {
    // Adapters record the local name on type-reference edges whether or not it
    // differs from the symbol's own name. Treating every populated `alias` as a
    // rename flagged ordinary references as HIGH RISK and buried the genuine
    // renames among them.
    let graph = graph_with_user_payload();
    let same = edge(
        "src/models/user.ts::UserPayload::class",
        Some("UserPayload"),
    );
    assert!(!is_renamed(&graph, &same));
}

#[test]
fn a_differing_alias_is_a_rename() {
    let graph = graph_with_user_payload();
    let renamed = edge(
        "src/models/user.ts::UserPayload::class",
        Some("ResponseModel"),
    );
    assert!(
        is_renamed(&graph, &renamed),
        "this is the case the tool exists to surface"
    );
}

#[test]
fn a_qualified_reference_is_compared_on_its_final_segment() {
    // `models.UserPayload` in Go and `geometry::Circle` in C++ name the symbol
    // directly; the qualifier is not a rename.
    let graph = graph_with_user_payload();
    for alias in ["models.UserPayload", "geometry::UserPayload"] {
        let qualified = edge("src/models/user.ts::UserPayload::class", Some(alias));
        assert!(
            !is_renamed(&graph, &qualified),
            "'{alias}' names the symbol directly"
        );
    }
}

#[test]
fn an_edge_without_an_alias_is_never_a_rename() {
    let graph = graph_with_user_payload();
    assert!(!is_renamed(
        &graph,
        &edge("src/models/user.ts::UserPayload::class", None)
    ));
}
