//! PHP — Tier 2, structural.
//!
//! The whole module, as with every Tier 2 language. Analysis runs through
//! [`crate::structural`], driven by the `tags.scm` the grammar already ships,
//! so there is no parser, extractor or resolver here to write or maintain.
//!
//! What this is not is a claim that OpenInvar understands PHP the way it
//! understands the Tier 1 languages. Nothing here resolves a `use` statement,
//! follows an alias, or binds a declared type; `openinvar status` reports the
//! tier and the resolution coverage, and gates fail open on both.
//!
//! # Which dialect
//!
//! The grammar ships two. `LANGUAGE_PHP` parses a file as PHP embedded in
//! text, which is what a `.php` file on disk actually is — the `<?php` tag
//! opens a region and everything outside it is markup. `LANGUAGE_PHP_ONLY`
//! parses the code alone, and given a real file it would treat any leading
//! HTML as a syntax error.
//!
//! So the embedded dialect is the one that matches the extension this adapter
//! claims. A file that happens to contain no markup parses identically under
//! both, which is why the choice is easy to get wrong and never notice: it
//! only shows up on the files that use the feature PHP is named for.

use openinvar_core::ir::Language;

use crate::spec::{LanguageSpec, Tier};

pub struct Spec;

impl LanguageSpec for Spec {
    /// PHPUnit's `*Test.php`, plus the directories the ecosystem uses.
    ///
    /// PHPUnit finds tests by suffix and by the `testsuite` directories in
    /// `phpunit.xml`, and the suffix is the half that holds without reading a
    /// config file. `tests/` is the near-universal convention for the other
    /// half; `Test/` appears in PSR-4 layouts that mirror `src/`.
    fn is_test_file(&self, path: &str) -> bool {
        let name = path.rsplit('/').next().unwrap_or(path);
        let stem = name.strip_suffix(".php").unwrap_or(name);
        stem.ends_with("Test")
            || stem.ends_with("_test")
            || path.starts_with("tests/")
            || path.starts_with("test/")
            || path.starts_with("Test/")
            || path.contains("/tests/")
            || path.contains("/test/")
            || path.contains("/Test/")
    }

    fn language(&self) -> Language {
        Language::Php
    }

    fn tier(&self) -> Tier {
        Tier::Structural
    }

    fn name(&self) -> &'static str {
        "PHP"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["php"]
    }

    fn grammar(&self) -> Option<tree_sitter::Language> {
        Some(tree_sitter_php::LANGUAGE_PHP.into())
    }

    fn tags_query(&self) -> Option<&'static str> {
        Some(tree_sitter_php::TAGS_QUERY)
    }
}
