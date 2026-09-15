//! What a default build carries.
//!
//! `default` is `full`, because there is one released binary per platform and
//! it analyses every language. That was not always so, and the way it went
//! wrong is the reason this file exists: `release.yml` built with default
//! features while `default` was the Tier 1 set, so every published binary
//! carried five languages — and the README, for a year, documented the eight
//! of `full` and quoted its coverage figure. Nothing failed. The binary simply
//! did not do what the docs said, and no test could tell.
//!
//! These assertions fix the released feature set in place. A change that
//! narrows `default` again has to delete a test that says why not.
//!
//! Both set assertions are gated on `full`, which is what `default` resolves
//! to. A deliberately narrowed build — `--no-default-features --features
//! python`, which the per-language CI job exercises — carries one language on
//! purpose, and asserting eight there would be asserting the opposite of what
//! that build is for. The tier assertion is not gated: whatever a build
//! carries must report its tier correctly, since a gate reads that.

#[cfg(feature = "full")]
use openinvar_lang::dispatch::supported_language_names;

/// Every language the project ships, by the name `status` prints.
#[cfg(feature = "full")]
const EVERY_LANGUAGE: [&str; 8] = [
    "TypeScript",
    "Python",
    "Rust",
    "Go",
    "C/C++",
    "Java",
    "Ruby",
    "C#",
];

#[test]
#[cfg(feature = "full")]
fn a_default_build_carries_every_language() {
    let carried = supported_language_names();

    for language in EVERY_LANGUAGE {
        assert!(
            carried.contains(&language),
            "a default build must carry {language}, because that is the build \
             `release.yml` publishes and the one the README's coverage figure \
             describes.\ncarried: {carried:?}"
        );
    }
}

#[test]
#[cfg(feature = "full")]
fn a_default_build_carries_nothing_beyond_the_documented_set() {
    // The other direction, so adding a language without documenting its tier
    // fails here rather than surfacing as an undocumented row in `status`.
    let carried = supported_language_names();

    for language in &carried {
        assert!(
            EVERY_LANGUAGE.contains(language),
            "'{language}' is in a default build but not in the documented set. \
             Add it to the README's tier table and to EVERY_LANGUAGE here."
        );
    }
    assert_eq!(
        carried.len(),
        EVERY_LANGUAGE.len(),
        "duplicate language in a default build: {carried:?}"
    );
}

#[test]
fn the_tier_one_languages_are_reported_as_resolved() {
    // A gate acts on Tier 1 and must never act on Tier 2, so the tier a build
    // reports is load-bearing rather than cosmetic. Shipping Tier 2 in the
    // default binary is only safe while this holds.
    use openinvar_lang::dispatch::supported_languages;
    use openinvar_lang::spec::Tier;

    for support in supported_languages() {
        let expected = match support.name {
            // Java was promoted: it has an import resolver, a scope analyzer
            // binding receivers to declared types, and a supertype walk, so a
            // gate may act on it. Ruby and C# have none of that yet.
            "Ruby" => Tier::Structural,
            _ => Tier::Resolved,
        };
        assert_eq!(
            support.tier.as_str(),
            expected.as_str(),
            "{} is reported as {} but should be {}",
            support.name,
            support.tier.as_str(),
            expected.as_str()
        );
    }
}
