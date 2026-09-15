//! Lua — Tier 2, structural.
//!
//! The whole module, as with every Tier 2 language. Analysis runs through
//! [`crate::structural`], driven by the `tags.scm` the grammar already ships,
//! so there is no parser, extractor or resolver here to write or maintain.
//!
//! What this is not is a claim that OpenInvar understands Lua the way it
//! understands the Tier 1 languages. Nothing here resolves a `require`,
//! follows an alias, or binds a declared type; `openinvar status` reports the
//! tier and the resolution coverage, and gates fail open on both.
//!
//! Lua is a language where that caution is more than boilerplate. A module is
//! whatever table `require` returns, assembled at runtime; methods are fields
//! that happen to hold functions, and can be added to any table at any point;
//! and metatables let a lookup that finds nothing consult another table
//! instead. A resolver that reported certainty here would mostly be reporting
//! a guess about what a table held when it ran.

use openinvar_core::ir::Language;

use crate::spec::{LanguageSpec, Tier};

pub struct Spec;

impl LanguageSpec for Spec {
    /// busted's `_spec.lua` and luaunit's `test_*`, plus their directories.
    ///
    /// Lua has no single test runner, so this covers the two conventions in
    /// common use rather than pretending one is canonical. busted looks in
    /// `spec/` for `*_spec.lua`; luaunit files are conventionally named for the
    /// thing under test with a `test` prefix or suffix.
    fn is_test_file(&self, path: &str) -> bool {
        let name = path.rsplit('/').next().unwrap_or(path);
        let stem = name.strip_suffix(".lua").unwrap_or(name);
        stem.ends_with("_spec")
            || stem.ends_with("_test")
            || stem.starts_with("test_")
            || path.starts_with("spec/")
            || path.starts_with("test/")
            || path.starts_with("tests/")
            || path.contains("/spec/")
            || path.contains("/test/")
            || path.contains("/tests/")
    }

    fn language(&self) -> Language {
        Language::Lua
    }

    fn tier(&self) -> Tier {
        Tier::Structural
    }

    fn name(&self) -> &'static str {
        "Lua"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["lua"]
    }

    fn grammar(&self) -> Option<tree_sitter::Language> {
        Some(tree_sitter_lua::LANGUAGE.into())
    }

    fn tags_query(&self) -> Option<&'static str> {
        Some(tree_sitter_lua::TAGS_QUERY)
    }
}
