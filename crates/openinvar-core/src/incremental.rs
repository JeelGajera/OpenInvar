use crate::graph::InvarGraph;
use crate::ir::FileIR;

pub struct IncrementalUpdateResult {
    pub removed_symbol_ids: Vec<String>,
    pub added_symbol_ids: Vec<String>,
    pub removed_relationships: usize,
    pub added_relationships: usize,
}

pub fn replace_file_ir(graph: &mut InvarGraph, file_ir: &FileIR) -> IncrementalUpdateResult {
    let removed_relationships = graph.remove_relationships_in_file(&file_ir.file);
    let removed_symbol_ids = graph.remove_file(&file_ir.file);
    graph
        .file_reexports
        .insert(file_ir.file.clone(), file_ir.re_exports.clone());

    let mut added_symbol_ids = Vec::new();
    for symbol in &file_ir.symbols {
        graph.add_symbol(symbol.clone());
        added_symbol_ids.push(symbol.id.clone());
    }

    let mut added_relationships = 0usize;
    for relationship in &file_ir.relationships {
        graph.add_relationship(relationship);
        added_relationships += 1;
    }

    added_symbol_ids.sort();

    IncrementalUpdateResult {
        removed_symbol_ids,
        added_symbol_ids,
        removed_relationships,
        added_relationships,
    }
}
