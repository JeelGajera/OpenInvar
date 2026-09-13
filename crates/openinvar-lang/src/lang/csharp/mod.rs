//! C# — Tier 2, structural.
//!
//! The whole module, as with every Tier 2 language. Analysis runs through
//! [`crate::structural`], driven by the `tags.scm` the grammar already ships,
//! so there is no parser, extractor or resolver here to write or maintain.
//!
//! What this is not is a claim that OpenInvar understands C# the way it
//! understands the Tier 1 languages. Nothing here resolves an import, follows
//! an alias, or binds a declared type; `openinvar status` reports the tier and
//! the resolution coverage, and gates fail open on both.

use openinvar_core::ir::Language;

use crate::spec::{LanguageSpec, Tier};

pub struct Spec;

impl LanguageSpec for Spec {
    /// The convention shared by xUnit, NUnit and MSTest projects.
    fn is_test_file(&self, path: &str) -> bool {
        let name = path.rsplit('/').next().unwrap_or(path);
        let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
        stem.ends_with("Test")
            || stem.ends_with("Tests")
            || stem.ends_with("Spec")
            || path.contains(".Tests/")
            || path.contains("/Tests/")
    }

    fn language(&self) -> Language {
        Language::CSharp
    }

    fn tier(&self) -> Tier {
        Tier::Structural
    }

    fn name(&self) -> &'static str {
        "C#"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["cs"]
    }

    fn grammar(&self) -> Option<tree_sitter::Language> {
        Some(tree_sitter_c_sharp::language())
    }

    fn tags_query(&self) -> Option<&'static str> {
        Some(tree_sitter_c_sharp::TAGS_QUERY)
    }
}
