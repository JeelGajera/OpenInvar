//! Parsing and resolution for every language OpenInvar understands.
//!
//! Consolidates the five `openinvar-adapter-*` crates and `openinvar-adapter-
//! dispatch` into one crate with a module and a Cargo feature per language.
//! See [`lang`] for why.
//!
//! The entry point is [`analyze_files`], which routes files to the module that
//! owns their language and merges the results into one `RepoIR`.

pub mod dispatch;
pub mod lang;
pub mod spec;
pub mod structural;

pub use dispatch::{analyze_files, supported_languages, DispatchError};
pub use spec::{for_path, LanguageSpec, LanguageSupport, Tier};
