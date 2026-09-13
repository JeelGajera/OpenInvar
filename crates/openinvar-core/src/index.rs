use crate::error::InvarError;
use crate::graph::InvarGraph;
use crate::ir::SymbolId;

pub fn find_symbol_id(
    graph: &InvarGraph,
    symbol: &str,
    file: Option<&str>,
) -> Result<SymbolId, InvarError> {
    let Some(ids_ref) = graph.name_index.get(symbol) else {
        return Err(InvarError::SymbolNotFound(symbol.to_string()));
    };
    let ids = ids_ref.value().clone();
    drop(ids_ref);

    if let Some(file) = file {
        for id in ids {
            if let Some(sym) = graph.symbols.get(&id) {
                if sym.file == file {
                    return Ok(id);
                }
            }
        }
        return Err(InvarError::SymbolNotFound(format!("{symbol} in {file}")));
    }

    if ids.len() == 1 {
        return Ok(ids[0].clone());
    }

    let mut candidates = Vec::new();
    for id in ids {
        if let Some(sym) = graph.symbols.get(&id) {
            candidates.push(sym.file.clone());
        }
    }
    candidates.sort();
    candidates.dedup();

    Err(InvarError::AmbiguousSymbol {
        symbol: symbol.to_string(),
        candidates,
    })
}
