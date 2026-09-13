#[cfg(feature = "ast")]
pub mod ast;
pub mod audit;
pub mod context;
pub mod coverage;
pub mod delta;
pub mod error;
pub mod findings;
pub mod graph;
pub mod incremental;
pub mod index;
pub mod ir;
pub mod query;
pub mod resolver;
pub mod rule_eval;
pub mod rules;
pub mod scan;
pub mod symbol_id;
pub mod test_impact;

pub use error::InvarError;
pub use graph::{InvarGraph, RelationshipMeta};
pub use ir::*;
pub use symbol_id::{
    external_package_id, is_placeholder, make_symbol_id, module_symbol, module_symbol_id,
    unresolved_import_id, unresolved_local_type_id, IMPORT_ALL,
};
