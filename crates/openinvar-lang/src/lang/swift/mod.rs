//! Swift — Tier 2, structural.
//!
//! The whole module, as with every Tier 2 language. Analysis runs through
//! [`crate::structural`], driven by the `tags.scm` the grammar already ships,
//! so there is no parser, extractor or resolver here to write or maintain.
//!
//! What this is not is a claim that OpenInvar understands Swift the way it
//! understands the Tier 1 languages. Nothing here resolves an `import`,
//! follows a `typealias`, or binds a declared type; `openinvar status` reports
//! the tier and the resolution coverage, and gates fail open on both.
//!
//! # Swift gives less than the other Tier 2 languages
//!
//! Tier 2 is usually "symbols, and references within one file". Swift is
//! symbols only. `tree-sitter-swift`'s `tags.scm` contains `@definition`
//! captures and no `@reference` ones, where Ruby, PHP and Lua all provide at
//! least `@reference.call` — so a Swift file yields declarations and no edges
//! at all.
//!
//! That is a property of the query the grammar ships, not of this adapter or of
//! the structural analyzer, and it would change the day upstream adds the
//! captures. Writing the missing query here instead was the alternative, and it
//! is the one thing a Tier 2 adapter is meant not to do: the tier's whole claim
//! is that adding a language is a dependency rather than a query to maintain
//! against a grammar that moves.
//!
//! Swift would be an awkward language to promote, and it is worth saying so
//! rather than implying the tier is only a matter of time. A module is the
//! unit of import and a target's files share a namespace without declaring it,
//! so resolving a name means knowing the package layout — and extensions let a
//! type gain members from any file in that target, which is the cross-file
//! member attribution Tier 1 rests on.

use openinvar_core::ir::Language;

use crate::spec::{LanguageSpec, Tier};

pub struct Spec;

impl LanguageSpec for Spec {
    /// XCTest and swift-testing files, plus SwiftPM's `Tests/` directory.
    ///
    /// SwiftPM puts every test target under `Tests/` and conventionally names
    /// each one `<Target>Tests`, so both halves point the same way. The suffix
    /// is plural — `UserServiceTests.swift` — which is the opposite of
    /// PHPUnit's singular `Test`, so the two are spelled out separately rather
    /// than shared.
    fn is_test_file(&self, path: &str) -> bool {
        let name = path.rsplit('/').next().unwrap_or(path);
        let stem = name.strip_suffix(".swift").unwrap_or(name);
        stem.ends_with("Tests")
            || stem.ends_with("Test")
            || path.starts_with("Tests/")
            || path.contains("/Tests/")
    }

    fn language(&self) -> Language {
        Language::Swift
    }

    fn tier(&self) -> Tier {
        Tier::Structural
    }

    fn name(&self) -> &'static str {
        "Swift"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["swift"]
    }

    fn grammar(&self) -> Option<tree_sitter::Language> {
        Some(tree_sitter_swift::LANGUAGE.into())
    }

    fn tags_query(&self) -> Option<&'static str> {
        Some(tree_sitter_swift::TAGS_QUERY)
    }
}
