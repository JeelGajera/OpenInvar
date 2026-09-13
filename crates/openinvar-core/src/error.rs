#[derive(thiserror::Error, Debug)]
pub enum InvarError {
    #[error("Symbol not found: {0}")]
    SymbolNotFound(String),

    #[error("Parse error in {file}:{line} — {message}")]
    ParseError {
        file: String,
        line: u32,
        message: String,
    },

    #[error("Graph is corrupt: {0}")]
    GraphCorrupt(String),

    #[error("Storage error: {0}")]
    StorageError(String),

    #[error("Ambiguous symbol '{symbol}', provide file. Candidates: {candidates:?}")]
    AmbiguousSymbol {
        symbol: String,
        candidates: Vec<String>,
    },

    #[error("Invalid depth '{depth}', max allowed is {max}")]
    InvalidDepth { depth: usize, max: usize },
}
