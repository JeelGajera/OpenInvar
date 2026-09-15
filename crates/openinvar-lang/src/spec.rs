//! What OpenInvar knows about a language, and how much it knows.
//!
//! # Why tiers exist
//!
//! Consolidating the adapters into one crate removed the *packaging* cost of a
//! language. It did not remove the engineering cost, and pretending otherwise
//! would make the gates in later phases unsafe.
//!
//! Parsing is tree-sitter and is a solved, vendored problem. Resolution is not:
//! every bug 0.2.0 fixed was a resolution bug — property access compared
//! against a hardcoded receiver name, `#include` producing zero edges, Go
//! imports pointing at an arbitrary package member, Rust use-paths matching
//! leaf names repository-wide. Import resolution, alias resolution and
//! declared-type binding are irreducibly per-language and cost weeks, not
//! hours.
//!
//! So "supports thirty languages" has to mean something precise, or it is
//! dishonest:
//!
//! | Tier | What it gives you | Safe to gate on |
//! |---|---|---|
//! | 1 — Resolved | Full import, alias and declared-type resolution | Yes |
//! | 2 — Structural | Symbols and intra-file references only | No — advisory |
//!
//! A Tier 2 language is a low-confidence region *by construction*. That is what
//! makes broad coverage safe rather than reckless: a gate can fail open on it
//! automatically instead of reporting a pass it did not earn.
//!
//! # Why Tier 2 costs almost nothing
//!
//! Every tree-sitter grammar OpenInvar vendors ships a `queries/tags.scm` and
//! exposes it as `TAGS_QUERY`, using a small standard capture vocabulary:
//! `@definition.{class,function,method,type,interface,module,constant,macro}`,
//! `@reference.{call,type,class,implementation}`, and `@name`. That vocabulary
//! maps onto OpenInvar's own `SymbolKind` and `RelationshipKind` directly.
//!
//! So a Tier 2 language does not need query files written for it at all — see
//! [`crate::structural`], which is one generic implementation driven by
//! whatever `tags.scm` the grammar already ships. Adding one is a dependency,
//! a feature flag, and a spec.

use std::path::{Path, PathBuf};

use openinvar_core::ir::{FileIR, Language};

/// How much OpenInvar can resolve in a language.
///
/// Ordered so that comparisons read naturally: `Structural < Resolved`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    /// Symbols and intra-file references. No cross-file import resolution, no
    /// alias tracking, no declared-type binding. Advisory only.
    Structural,
    /// Full import, alias and declared-type resolution.
    Resolved,
}

impl Tier {
    pub fn as_str(&self) -> &'static str {
        match self {
            Tier::Structural => "structural",
            Tier::Resolved => "resolved",
        }
    }

    /// The tier number as the README and `status` report it.
    pub fn number(&self) -> u8 {
        match self {
            Tier::Resolved => 1,
            Tier::Structural => 2,
        }
    }

    /// Whether a gate may draw a conclusion from this region.
    ///
    /// Tier 2 cannot see across files, so "nothing references this" is a
    /// statement about one file rather than about the repository. A gate that
    /// treated it otherwise would report a pass it had not earned.
    pub fn is_gate_safe(&self) -> bool {
        matches!(self, Tier::Resolved)
    }
}

/// One language OpenInvar can analyse.
pub trait LanguageSpec: Send + Sync {
    fn language(&self) -> Language;
    fn tier(&self) -> Tier;

    /// The name reported to users.
    fn name(&self) -> &'static str;

    /// File extensions this spec claims, without the dot.
    fn extensions(&self) -> &'static [&'static str];

    /// The grammar, for the structural analyzer.
    ///
    /// Tier 1 languages parse through their own pipeline and never call this,
    /// so it is optional rather than required of every spec.
    fn grammar(&self) -> Option<tree_sitter::Language> {
        None
    }

    /// The grammar's own tags query, which drives [`crate::structural`].
    ///
    /// Upstream grammars ship one; there is no need to write it.
    fn tags_query(&self) -> Option<&'static str> {
        None
    }

    /// Whether `path` names a test file in this language.
    ///
    /// Convention only, and deliberately so. Every language in scope has a
    /// filename or directory convention its tooling already relies on to
    /// *find* tests — `_test.go`, `*.test.ts`, `test_*.py`, `tests/` — so the
    /// convention is not a heuristic layered on top, it is the same rule the
    /// test runner uses.
    ///
    /// What this cannot see is a test that lives outside the convention: a
    /// Rust `#[cfg(test)] mod tests` inside an ordinary source file is the
    /// common case in this very repository. Recognising those needs the
    /// attribute, not the path, and the symbols inside such a module are not
    /// indexed at all today. The blind spot is documented rather than guessed
    /// at: a `tests` edge OpenInvar cannot see is a test it will fail to
    /// suggest, which is a smaller harm than one it invents.
    ///
    /// `path` is repository-relative with forward slashes.
    fn is_test_file(&self, _path: &str) -> bool {
        false
    }

    /// A Tier 1 language's own pipeline.
    ///
    /// Returning `None` opts into the structural default. The five Tier 1
    /// languages keep the pipelines they already had rather than being
    /// rewritten into resolution hooks: they work, they are tested, and a
    /// rewrite would forfeit the one property this change has to preserve.
    fn analyze(&self, _root: &Path, _files: &[PathBuf]) -> Option<Result<Vec<FileIR>, String>> {
        None
    }
}

/// Every language this build carries, in a fixed order.
///
/// Ordered rather than hash-ordered because it reaches the user through
/// `status` and `--help`, and OpenInvar's first guarantee is that identical
/// input produces identical output.
#[allow(clippy::vec_init_then_push)]
pub fn specs() -> Vec<&'static dyn LanguageSpec> {
    // Pushed rather than collected into a fixed-size array. The array needed a
    // length constant listing every feature a second time, and the two drifted
    // the first time a language was added after it was written — the same
    // "remember to edit it in two places" failure the publish-job check and the
    // per-language CI step exist to remove.
    //
    // `vec_init_then_push` is allowed rather than obeyed: its suggestion is to
    // write a literal, and a literal cannot carry a `cfg` attribute per element
    // the way these pushes do. Obeying it would mean going back to the
    // fixed-size array and the length constant this replaced.
    #[allow(unused_mut)]
    let mut out: Vec<&'static dyn LanguageSpec> = Vec::new();
    #[cfg(feature = "typescript")]
    out.push(&crate::lang::typescript::Spec as &'static dyn LanguageSpec);
    #[cfg(feature = "python")]
    out.push(&crate::lang::python::Spec as &'static dyn LanguageSpec);
    #[cfg(feature = "rust")]
    out.push(&crate::lang::rust::Spec as &'static dyn LanguageSpec);
    #[cfg(feature = "go")]
    out.push(&crate::lang::go::Spec as &'static dyn LanguageSpec);
    #[cfg(feature = "c")]
    out.push(&crate::lang::c::Spec as &'static dyn LanguageSpec);
    #[cfg(feature = "java")]
    out.push(&crate::lang::java::Spec as &'static dyn LanguageSpec);
    #[cfg(feature = "ruby")]
    out.push(&crate::lang::ruby::Spec as &'static dyn LanguageSpec);
    #[cfg(feature = "php")]
    out.push(&crate::lang::php::Spec as &'static dyn LanguageSpec);
    #[cfg(feature = "csharp")]
    out.push(&crate::lang::csharp::Spec as &'static dyn LanguageSpec);
    out
}


/// The spec that owns `language`, if this build carries it.
pub fn for_language(language: &Language) -> Option<&'static dyn LanguageSpec> {
    specs().into_iter().find(|s| s.language() == *language)
}

/// The spec that claims `path`'s extension, if this build carries one.
///
/// Routing by extension is already how dispatch decides which language reads a
/// file; this exposes the same answer to callers that have a path and need the
/// language, such as reporting coverage per language.
pub fn for_path(path: &str) -> Option<&'static dyn LanguageSpec> {
    let extension = path.rsplit_once('.').map(|(_, ext)| ext)?;
    specs()
        .into_iter()
        .find(|spec| spec.extensions().contains(&extension))
}

/// A language and how much of it OpenInvar can resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageSupport {
    pub name: &'static str,
    pub tier: Tier,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_one_is_the_only_gate_safe_tier() {
        // The whole point of the distinction. If this ever becomes true for
        // Structural, every gate in Phases 2 and 5 silently starts trusting
        // intra-file-only data.
        assert!(Tier::Resolved.is_gate_safe());
        assert!(!Tier::Structural.is_gate_safe());
        assert_eq!(Tier::Resolved.number(), 1);
        assert_eq!(Tier::Structural.number(), 2);
    }

    #[test]
    fn every_spec_claims_at_least_one_extension() {
        for spec in specs() {
            assert!(
                !spec.extensions().is_empty(),
                "{} claims no extensions, so no file would ever reach it",
                spec.name()
            );
        }
    }

    #[test]
    fn no_two_specs_claim_the_same_extension() {
        // Two specs claiming one extension would make routing depend on
        // registry order rather than on the language.
        let mut seen: Vec<(&str, &str)> = Vec::new();
        for spec in specs() {
            for ext in spec.extensions() {
                if let Some((other, _)) = seen.iter().find(|(_, e)| e == ext) {
                    panic!("both {other} and {} claim .{ext}", spec.name());
                }
                seen.push((spec.name(), ext));
            }
        }
    }

    #[test]
    fn a_structural_spec_can_actually_run() {
        // A Tier 2 spec with no grammar or no tags query would register as
        // supported and then analyse nothing.
        for spec in specs().into_iter().filter(|s| s.tier() == Tier::Structural) {
            assert!(
                spec.grammar().is_some(),
                "{} is Tier 2 but has no grammar",
                spec.name()
            );
            assert!(
                spec.tags_query().is_some(),
                "{} is Tier 2 but has no tags query",
                spec.name()
            );
        }
    }
}
