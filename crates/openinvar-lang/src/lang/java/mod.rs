//! Java — the first Tier 2 language.
//!
//! Deliberately the whole module. There is no parser, no extractor and no
//! resolver here, because a Tier 2 language needs none: analysis runs through
//! [`crate::structural`], driven by the `tags.scm` the grammar already ships.
//!
//! This is the demonstration that adding a language is a dependency, a feature
//! flag and a spec. What it is *not* is a claim that OpenInvar understands Java
//! the way it understands the Tier 1 languages: nothing here resolves an
//! import, follows an alias, or binds a declared type, and `openinvar status`
//! says so.
//!
//! Promoting Java to Tier 1 means implementing import resolution, alias
//! resolution and declared-type binding for it. That is weeks of work and is
//! explicitly out of scope for 1.0.0.

use openinvar_core::ir::Language;

use crate::spec::{LanguageSpec, Tier};

pub struct Spec;

impl LanguageSpec for Spec {
    /// Maven and Gradle both put tests under `src/test/`, and JUnit classes
    /// conventionally end in `Test` or `Tests`.
    fn is_test_file(&self, path: &str) -> bool {
        let name = path.rsplit('/').next().unwrap_or(path);
        let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
        stem.ends_with("Test")
            || stem.ends_with("Tests")
            || stem.starts_with("Test")
            || path.contains("src/test/")
    }

    fn language(&self) -> Language {
        Language::Java
    }

    fn tier(&self) -> Tier {
        Tier::Structural
    }

    fn name(&self) -> &'static str {
        "Java"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["java"]
    }

    fn grammar(&self) -> Option<tree_sitter::Language> {
        Some(tree_sitter_java::language())
    }

    fn tags_query(&self) -> Option<&'static str> {
        // The grammar's own query, not one written here. It captures class,
        // interface and method definitions, plus calls, superclasses and
        // implemented types.
        Some(tree_sitter_java::TAGS_QUERY)
    }
}
