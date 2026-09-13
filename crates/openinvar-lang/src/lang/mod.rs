//! One module per language.
//!
//! Each was its own crate until 1.0.0. Three arguments usually justify a
//! crate-per-language split, and none held here: every adapter depended on
//! `openinvar-core` and was consumed only by dispatch, so they versioned in
//! lockstep and had exactly one consumer; and the expensive compilation is the
//! generated C in the upstream `tree-sitter-*` crates, which cargo caches
//! independently of how this code is laid out.
//!
//! What the split did cost was real. Ten crates had to be published in
//! dependency order, and the 0.2.0 release job would have failed partway
//! through with `openinvar-core` already on crates.io and its version
//! unrepublishable. Every new language added another entry to that job, another
//! manifest, another README, another version bump. Adding a language is now a
//! module and a feature flag.
//!
//! Features gate each language, which is what makes
//! [`crate::supported_languages`] mean something: a build reports what it can
//! actually analyse rather than what the source tree happens to contain.

#[cfg(feature = "c")]
pub mod c;
#[cfg(feature = "go")]
pub mod go;
#[cfg(feature = "java")]
pub mod java;
#[cfg(feature = "python")]
pub mod python;
#[cfg(feature = "rust")]
pub mod rust;
#[cfg(feature = "typescript")]
pub mod typescript;

#[cfg(feature = "ruby")]
pub mod ruby;


#[cfg(feature = "csharp")]
pub mod csharp;



