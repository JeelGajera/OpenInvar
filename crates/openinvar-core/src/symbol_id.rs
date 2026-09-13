//! Symbol identity: the one place that knows how a `SymbolId` is spelled.
//!
//! Every adapter mints ids for the symbols it finds and, during resolution,
//! rewrites placeholder ids into real ones. Both halves of that contract used to
//! be duplicated per adapter, which let the spellings drift apart — the
//! TypeScript adapter separated placeholder fields with `|` while the newer
//! adapters used `::`, a separator that also occurs inside Rust and C++ paths
//! and therefore could not be parsed back out unambiguously.
//!
//! # Resolved ids
//!
//! `relative/file/path.rs::SymbolName::kind` — stable, persisted to the store,
//! and the only form that may appear in a graph node.
//!
//! # Placeholder ids
//!
//! Emitted by extractors, consumed by resolvers, and never persisted. They use
//! `|` as a field separator because `|` cannot occur in an identifier or a
//! module path in any language we parse:
//!
//! - `unresolved_import|<module>|<symbol>` — an import awaiting module resolution
//! - `unresolved_local_type|<type>` — a type reference awaiting local lookup
//!
//! [`external_package_id`] is the exception: it survives resolution as a real
//! graph node, because an edge to a third-party package is a fact worth keeping.

use crate::ir::{Language, Symbol, SymbolId, SymbolKind};

const UNRESOLVED_IMPORT_PREFIX: &str = "unresolved_import";
const UNRESOLVED_LOCAL_TYPE_PREFIX: &str = "unresolved_local_type";
const EXTERNAL_PREFIX: &str = "ext";

/// The trailing component of a symbol id, identifying what kind of thing it is.
pub fn kind_suffix(kind: &SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Class => "class",
        SymbolKind::Interface => "interface",
        SymbolKind::TypeAlias => "type_alias",
        SymbolKind::Function => "function",
        SymbolKind::Method => "method",
        SymbolKind::Property => "property",
        SymbolKind::Variable => "variable",
        SymbolKind::Module => "module",
        SymbolKind::Enum => "enum",
        SymbolKind::EnumVariant => "enum_variant",
        SymbolKind::ExternalPackage => "package",
    }
}

/// Mint the canonical id for a symbol defined at `file`.
pub fn make_symbol_id(file: &str, name: &str, kind: &SymbolKind) -> SymbolId {
    format!("{file}::{name}::{}", kind_suffix(kind))
}

/// The id of the synthetic per-file module symbol.
///
/// Every file gets one. It anchors file-level edges (imports, includes) that
/// belong to the file rather than to any symbol inside it.
pub fn module_symbol_id(file: &str) -> SymbolId {
    make_symbol_id(file, "module", &SymbolKind::Module)
}

/// Build the synthetic per-file module symbol.
pub fn module_symbol(file: &str, language: Language) -> Symbol {
    Symbol {
        id: module_symbol_id(file),
        name: "module".to_string(),
        kind: SymbolKind::Module,
        language,
        file: file.to_string(),
        line_start: 1,
        line_end: 1,
        signature: None,
    }
}

/// Split a resolved symbol id back into `(file, name, kind_suffix)`.
///
/// File paths may contain `::` on no platform we support, but symbol names can
/// (`Trait::method` in a Rust signature), so the name is taken as everything
/// between the first and last separator.
pub fn parse_symbol_id(id: &str) -> Option<(&str, &str, &str)> {
    let first = id.find("::")?;
    let last = id.rfind("::")?;
    if last <= first {
        return None;
    }
    Some((&id[..first], &id[first + 2..last], &id[last + 2..]))
}

/// The symbol name component of a resolved id, if it is one.
pub fn symbol_name_of(id: &str) -> Option<&str> {
    parse_symbol_id(id).map(|(_, name, _)| name)
}

// ── placeholders ─────────────────────────────────────────────

/// Placeholder for an import of `symbol` from `module`, pending resolution.
///
/// Pass [`IMPORT_ALL`] as `symbol` for a whole-module import (`import os`,
/// `use foo::*`, `import "fmt"`).
pub fn unresolved_import_id(module: &str, symbol: &str) -> SymbolId {
    format!("{UNRESOLVED_IMPORT_PREFIX}|{module}|{symbol}")
}

/// The `symbol` value denoting "the module itself", not a member of it.
pub const IMPORT_ALL: &str = "*";

/// Recover `(module, symbol)` from an unresolved-import placeholder.
pub fn parse_unresolved_import_id(raw: &str) -> Option<(&str, &str)> {
    let rest = raw
        .strip_prefix(UNRESOLVED_IMPORT_PREFIX)?
        .strip_prefix('|')?;
    // The module may itself be empty (a bare relative import), so split from the
    // right: the symbol never contains a separator.
    let cut = rest.rfind('|')?;
    Some((&rest[..cut], &rest[cut + 1..]))
}

/// Placeholder for a reference to a type that must be looked up in file scope.
pub fn unresolved_local_type_id(type_name: &str) -> SymbolId {
    format!("{UNRESOLVED_LOCAL_TYPE_PREFIX}|{type_name}")
}

/// Recover the type name from a local-type placeholder.
pub fn parse_unresolved_local_type_id(raw: &str) -> Option<&str> {
    raw.strip_prefix(UNRESOLVED_LOCAL_TYPE_PREFIX)?
        .strip_prefix('|')
}

/// True if `id` is any placeholder, i.e. resolution did not finish for it.
///
/// Placeholders must never reach the graph: [`crate::graph::InvarGraph::add_relationship`]
/// drops edges pointing at unknown ids, so an unresolved placeholder is a
/// silently missing edge. Resolvers use this to decide what to report.
pub fn is_placeholder(id: &str) -> bool {
    id.starts_with(UNRESOLVED_IMPORT_PREFIX) || id.starts_with(UNRESOLVED_LOCAL_TYPE_PREFIX)
}

// ── external packages ────────────────────────────────────────

/// The id of the shared node representing a third-party package.
///
/// Unlike placeholders this survives into the graph;
/// [`crate::graph::InvarGraph::add_relationship`] creates the node on first
/// reference.
pub fn external_package_id(package: &str) -> SymbolId {
    format!("{EXTERNAL_PREFIX}::{package}::package")
}

/// True if `id` names an external package node.
pub fn is_external_package(id: &str) -> bool {
    id.starts_with("ext::") && id.ends_with("::package")
}

/// Recover the package name from an external package id.
pub fn parse_external_package_id(id: &str) -> Option<&str> {
    id.strip_prefix("ext::")?.strip_suffix("::package")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_ids_round_trip() {
        let id = make_symbol_id("src/models/user.rs", "UserPayload", &SymbolKind::Class);
        assert_eq!(id, "src/models/user.rs::UserPayload::class");
        assert_eq!(
            parse_symbol_id(&id),
            Some(("src/models/user.rs", "UserPayload", "class"))
        );
    }

    #[test]
    fn symbol_names_containing_the_separator_survive_a_round_trip() {
        // Rust and C++ both produce names like this for qualified methods.
        let id = make_symbol_id("src/lib.rs", "Display::fmt", &SymbolKind::Method);
        assert_eq!(
            parse_symbol_id(&id),
            Some(("src/lib.rs", "Display::fmt", "method"))
        );
    }

    #[test]
    fn import_placeholders_round_trip_paths_containing_colons() {
        // The old `::`-separated placeholder format could not represent this:
        // splitting on `::` yielded module="crate", symbol="models::UserPayload".
        let id = unresolved_import_id("crate::models::user_payload", "UserPayload");
        assert_eq!(
            parse_unresolved_import_id(&id),
            Some(("crate::models::user_payload", "UserPayload"))
        );
    }

    #[test]
    fn import_placeholders_round_trip_go_style_module_paths() {
        let id = unresolved_import_id("github.com/test/app/models", IMPORT_ALL);
        assert_eq!(
            parse_unresolved_import_id(&id),
            Some(("github.com/test/app/models", "*"))
        );
    }

    #[test]
    fn local_type_placeholders_round_trip() {
        let id = unresolved_local_type_id("ResponseModel");
        assert_eq!(parse_unresolved_local_type_id(&id), Some("ResponseModel"));
        assert!(is_placeholder(&id));
    }

    #[test]
    fn resolved_ids_are_not_placeholders() {
        let id = make_symbol_id("src/a.rs", "Alpha", &SymbolKind::Class);
        assert!(!is_placeholder(&id));
        assert!(!is_placeholder(&external_package_id("serde")));
    }

    #[test]
    fn external_package_ids_round_trip() {
        let id = external_package_id("serde");
        assert!(is_external_package(&id));
        assert_eq!(parse_external_package_id(&id), Some("serde"));
    }
}
