use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use openinvar_core::graph::InvarGraph;
use openinvar_core::ir::{Language, ReExportEntry, Relationship, RelationshipKind, Resolution, Symbol, SymbolKind};
use openinvar_core::resolver::AliasResolver;
use openinvar_store::{GraphSnapshot, RocksGraphStore, StoreError};

fn temp_db_path(name: &str) -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock must be after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("openinvar-store-{name}-{now}"))
}

fn make_symbol(id: &str, name: &str, file: &str, kind: SymbolKind) -> Symbol {
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

fn make_graph() -> InvarGraph {
    let mut graph = InvarGraph::new();

    let model = make_symbol(
        "models/user_payload.ts::UserPayload::class",
        "UserPayload",
        "models/user_payload.ts",
        SymbolKind::Class,
    );
    let mapper = make_symbol(
        "mappers/view_model_mapper.ts::ViewModelMapper::class",
        "ViewModelMapper",
        "mappers/view_model_mapper.ts",
        SymbolKind::Class,
    );

    graph.add_symbol(model.clone());
    graph.add_symbol(mapper.clone());

    let relationships = vec![Relationship {
        from: mapper.id.clone(),
        to: model.id.clone(),
        kind: RelationshipKind::Imports,
        alias: Some("ResponseModel".to_string()),
        properties_accessed: vec!["userId".to_string(), "timestamp".to_string()],
        context: "import { UserPayload as ResponseModel } from '../models/user_payload'"
            .to_string(),
        file: "mappers/view_model_mapper.ts".to_string(),
        line: 1,
        resolution: Resolution::Resolved,
    }];

    for rel in &relationships {
        graph.add_relationship(rel);
    }

    let resolver = AliasResolver::default();
    resolver.ingest_relationships(&graph, &relationships);
    graph.file_reexports.insert(
        "barrel/index.ts".to_string(),
        vec![ReExportEntry {
            exported_name: "UserPayload".to_string(),
            source_module: "./models/user_payload".to_string(),
        }],
    );

    graph
}

#[test]
fn test_graph_snapshot_round_trip_preserves_symbols_and_edges() {
    let graph = make_graph();

    let snapshot = GraphSnapshot::from_graph(&graph).expect("snapshot created");
    let restored = snapshot.into_graph().expect("graph restored");

    assert_eq!(restored.symbols.len(), 2);
    assert_eq!(restored.graph.edge_count(), 1);

    let aliases = restored
        .alias_chains
        .get("models/user_payload.ts::UserPayload::class")
        .expect("alias chain exists");
    assert_eq!(aliases.len(), 1);
    assert_eq!(aliases[0].alias_name, "ResponseModel");
}

#[test]
fn test_rocksdb_store_save_then_load_graph() {
    let path = temp_db_path("save-load");
    {
        let store = RocksGraphStore::open(&path).expect("db open");
        let graph = make_graph();
        store.save_graph(&graph).expect("graph saved");
    }

    {
        let store = RocksGraphStore::open(&path).expect("db reopen");
        let restored = store.load_graph().expect("graph loaded");

        assert_eq!(restored.symbols.len(), 2);
        assert_eq!(restored.graph.edge_count(), 1);
        assert!(restored
            .symbols
            .contains_key("models/user_payload.ts::UserPayload::class"));
    }

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn test_open_does_not_panic_on_windows_style_path() {
    let path = temp_db_path("windows-path-test");
    let store = RocksGraphStore::open(&path)
        .expect("store should open without mixed separator path errors on Windows");
    drop(store);
    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn test_load_graph_without_snapshot_returns_not_found() {
    let path = temp_db_path("missing");
    let store = RocksGraphStore::open(&path).expect("db open");

    let err = store.load_graph().expect_err("expected missing snapshot");
    assert!(matches!(err, StoreError::SnapshotNotFound));

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn test_snapshot_round_trip_preserves_literal_backslash_sequences() {
    let graph = make_graph();

    let model_id = "models/user_payload.ts::UserPayload::class";
    {
        let mut model = graph
            .symbols
            .get_mut(model_id)
            .expect("model symbol exists");
        model.signature = Some("class UserPayload { note: \"\\\\t\\\\n\" }".to_string());
    }

    let path = temp_db_path("backslash-roundtrip");
    {
        let store = RocksGraphStore::open(&path).expect("db open");
        store.save_graph(&graph).expect("graph saved");
    }

    let restored = {
        let store = RocksGraphStore::open(&path).expect("db open");
        store.load_graph().expect("graph loaded")
    };
    let restored_sig = restored
        .symbols
        .get(model_id)
        .and_then(|s| s.signature.clone())
        .expect("restored signature exists");
    assert_eq!(restored_sig, "class UserPayload { note: \"\\\\t\\\\n\" }");

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn test_snapshot_preserves_reexports() {
    let path = temp_db_path("reexports-roundtrip");
    {
        let store = RocksGraphStore::open(&path).expect("db open");
        let graph = make_graph();
        store.save_graph(&graph).expect("graph saved");
    }

    let restored = {
        let store = RocksGraphStore::open(&path).expect("db open");
        store.load_graph().expect("graph loaded")
    };

    let reexports = restored
        .file_reexports
        .get("barrel/index.ts")
        .expect("barrel re-exports should be restored");
    assert_eq!(
        reexports[0].source_module, "./models/user_payload",
        "restored barrel re-export source should match saved data"
    );

    let _ = std::fs::remove_dir_all(&path);
}
