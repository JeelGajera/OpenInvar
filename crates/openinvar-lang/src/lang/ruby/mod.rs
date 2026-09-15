//! Ruby — Tier 2, structural.
//!
//! The whole module, as with every Tier 2 language. Analysis runs through
//! [`crate::structural`], driven by the `tags.scm` the grammar already ships,
//! so there is no parser, extractor or resolver here to write or maintain.
//!
//! What this is not is a claim that OpenInvar understands Ruby the way it
//! understands the Tier 1 languages. Nothing here resolves an import, follows
//! an alias, or binds a declared type; `openinvar status` reports the tier and
//! the resolution coverage, and gates fail open on both.

use openinvar_core::ir::Language;

use crate::spec::{LanguageSpec, Tier};

pub struct Spec;

impl LanguageSpec for Spec {
    /// RSpec's `_spec.rb` and minitest's `_test.rb`, plus their directories.
    fn is_test_file(&self, path: &str) -> bool {
        let name = path.rsplit('/').next().unwrap_or(path);
        let stem = name.strip_suffix(".rb").unwrap_or(name);
        stem.ends_with("_spec")
            || stem.ends_with("_test")
            || path.starts_with("spec/")
            || path.starts_with("test/")
            || path.contains("/spec/")
            || path.contains("/test/")
    }

    fn language(&self) -> Language {
        Language::Ruby
    }

    fn tier(&self) -> Tier {
        Tier::Structural
    }

    fn name(&self) -> &'static str {
        "Ruby"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["rb"]
    }

    fn grammar(&self) -> Option<tree_sitter::Language> {
        Some(tree_sitter_ruby::LANGUAGE.into())
    }

    fn tags_query(&self) -> Option<&'static str> {
        Some(tree_sitter_ruby::TAGS_QUERY)
    }
}
