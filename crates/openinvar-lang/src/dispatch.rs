//! Routing files to the language module that understands them.
//!
//! Each adapter resolves imports within its own language, so files are grouped
//! by language and each group analysed as a unit. Grouping is ordered rather
//! than hash-ordered: `RepoIR.files` determines the order symbols and edges are
//! inserted into the graph, and OpenInvar's first documented guarantee is that
//! the graph is deterministic. `HashMap` iteration order varies per process,
//! which made two runs over identical input produce differently-ordered output.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use openinvar_core::ir::{FileIR, Language, Relationship, RepoIR, Resolution};
use openinvar_core::scan::detect_language_from_extension;
use openinvar_core::symbol_id::is_placeholder;
use rayon::prelude::*;

#[derive(Debug)]
pub enum DispatchError {
    #[cfg(feature = "typescript")]
    Ts(String),
    #[cfg(feature = "python")]
    Python(String),
    #[cfg(feature = "rust")]
    Rust(String),
    #[cfg(feature = "go")]
    Go(String),
    #[cfg(feature = "c")]
    C(String),
    #[cfg(feature = "java")]
    Java(String),
    #[cfg(feature = "csharp")]
    CSharp(String),
    /// A Tier 2 language, analysed structurally.
    Structural(String),
}

impl std::fmt::Display for DispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(feature = "typescript")]
            Self::Ts(e) => write!(f, "TypeScript adapter error: {e}"),
            #[cfg(feature = "python")]
            Self::Python(e) => write!(f, "Python adapter error: {e}"),
            #[cfg(feature = "rust")]
            Self::Rust(e) => write!(f, "Rust adapter error: {e}"),
            #[cfg(feature = "go")]
            Self::Go(e) => write!(f, "Go adapter error: {e}"),
            #[cfg(feature = "c")]
            Self::C(e) => write!(f, "C/C++ adapter error: {e}"),
            #[cfg(feature = "java")]
            Self::Java(e) => write!(f, "Java adapter error: {e}"),
            #[cfg(feature = "csharp")]
            Self::CSharp(e) => write!(f, "C# adapter error: {e}"),
            Self::Structural(e) => write!(f, "structural analysis error: {e}"),
        }
    }
}

impl std::error::Error for DispatchError {}

/// A stable ordering key for a language group.
///
/// `Language` is not `Ord`, and deriving it would change a public enum's API
/// for an internal need, so the ordering lives here.
fn language_rank(language: &Language) -> u8 {
    match language {
        Language::TypeScript => 0,
        Language::JavaScript => 1,
        Language::Python => 2,
        Language::Rust => 3,
        Language::Go => 4,
        Language::C => 5,
        Language::Cpp => 6,
        Language::Java => 7,
        Language::Ruby => 8,
        Language::Php => 9,
        Language::CSharp => 10,
        Language::Kotlin => 11,
        Language::Swift => 12,
        Language::Sql => 13,
        Language::Lua => 14,
    }
}

/// The adapter that owns a language, or `None` if it is not supported yet.
///
/// TypeScript and JavaScript share an adapter, and C and C++ share one, so the
/// grouping key is the adapter rather than the language.
fn adapter_group(language: &Language) -> Option<Language> {
    match language {
        #[cfg(feature = "typescript")]
        Language::TypeScript | Language::JavaScript => Some(Language::TypeScript),
        #[cfg(feature = "python")]
        Language::Python => Some(Language::Python),
        #[cfg(feature = "rust")]
        Language::Rust => Some(Language::Rust),
        #[cfg(feature = "go")]
        Language::Go => Some(Language::Go),
        #[cfg(feature = "c")]
        Language::C | Language::Cpp => Some(Language::C),
        #[cfg(feature = "java")]
        Language::Java => Some(Language::Java),
        #[cfg(feature = "csharp")]
        Language::CSharp => Some(Language::CSharp),
        // A language whose feature is off is skipped, so a slim build ignores
        // files it cannot analyse rather than failing on them.
        //
        // Tier 2 has no arm: a structural language is its own group, and the
        // registry already knows which languages this build carries. Listing
        // them here as well is the "remember to edit it in two places" failure
        // that adding a language is supposed to have stopped costing — and it
        // failed exactly that way the first time two were added at once, with
        // the spec present and the file silently never routed.
        other => crate::spec::for_language(other)
            .filter(|spec| spec.tier() == crate::spec::Tier::Structural)
            .map(|spec| spec.language()),
    }
}

pub fn analyze_files(root: &Path, files: &[PathBuf]) -> Result<RepoIR, DispatchError> {
    let mut by_adapter: BTreeMap<u8, (Language, Vec<PathBuf>)> = BTreeMap::new();

    for file in files {
        let Some(language) = file
            .extension()
            .and_then(|e| e.to_str())
            .and_then(detect_language_from_extension)
        else {
            continue;
        };
        let Some(group) = adapter_group(&language) else {
            continue;
        };
        by_adapter
            .entry(language_rank(&group))
            .or_insert_with(|| (group, Vec::new()))
            .1
            .push(file.clone());
    }

    // Each adapter resolves its own language independently, so groups can run
    // in parallel. Results are collected back in the deterministic key order.
    let groups: Vec<(Language, Vec<PathBuf>)> = by_adapter.into_values().collect();

    let analyzed: Vec<Result<Vec<FileIR>, DispatchError>> = groups
        .into_par_iter()
        .map(|(language, mut group_files)| {
            // Adapters index by path; a stable input order keeps their own
            // first-wins tie-breaks reproducible.
            group_files.sort();
            run_adapter(&language, root, &group_files)
        })
        .collect();

    let mut all_files: Vec<FileIR> = Vec::with_capacity(files.len());
    let mut language_stats: BTreeMap<String, usize> = BTreeMap::new();

    for result in analyzed {
        for file_ir in result? {
            *language_stats
                .entry(format!("{:?}", file_ir.language))
                .or_insert(0) += 1;
            all_files.push(file_ir);
        }
    }

    // After every adapter has run, because a test file's references are only
    // known to point outside the test tree once the whole tree is known.
    add_test_edges(&mut all_files);

    Ok(RepoIR {
        root: root.to_string_lossy().to_string(),
        files: all_files,
        language_stats,
    })
}

fn run_adapter(
    language: &Language,
    root: &Path,
    files: &[PathBuf],
) -> Result<Vec<FileIR>, DispatchError> {
    // A Tier 2 language needs no arm here: one generic implementation covers
    // every one of them, driven by the grammar's own tags query. That is the
    // whole point of the tier — adding a structural language is a dependency,
    // a feature flag and a spec, not a pipeline.
    if let Some(spec) = crate::spec::for_language(language) {
        if spec.tier() == crate::spec::Tier::Structural {
            return crate::structural::analyze(spec, root, files)
                .map_err(DispatchError::Structural);
        }
    }

    // Each remaining arm is gated by its language's feature. `adapter_group`
    // already refuses to route to a language this build does not carry, so an
    // arm that is compiled out is unreachable rather than silently skipped.
    // Annotated rather than inferred: in a build with no Tier 1 language
    // enabled — `--features java` alone, which the language docs present as a
    // supported slim build — every arm below is compiled out and the only one
    // left is `Vec::new()`, whose element type nothing then pins down.
    let mut files_ir: Vec<FileIR> = match language {
        #[cfg(feature = "typescript")]
        Language::TypeScript => {
            crate::lang::typescript::analyze_files(root, files)
                .map_err(|e| DispatchError::Ts(e.to_string()))?
                .files
        }
        #[cfg(feature = "python")]
        Language::Python => {
            crate::lang::python::analyze_files(root, files)
                .map_err(|e| DispatchError::Python(e.to_string()))?
                .files
        }
        #[cfg(feature = "rust")]
        Language::Rust => {
            crate::lang::rust::analyze_files(root, files)
                .map_err(|e| DispatchError::Rust(e.to_string()))?
                .files
        }
        #[cfg(feature = "go")]
        Language::Go => {
            crate::lang::go::analyze_files(root, files)
                .map_err(|e| DispatchError::Go(e.to_string()))?
                .files
        }
        #[cfg(feature = "c")]
        Language::C => {
            crate::lang::c::analyze_files(root, files)
                .map_err(|e| DispatchError::C(e.to_string()))?
                .files
        }
        #[cfg(feature = "java")]
        Language::Java => {
            crate::lang::java::analyze_files(root, files)
                .map_err(|e| DispatchError::Java(e.to_string()))?
                .files
        }
        #[cfg(feature = "csharp")]
        Language::CSharp => {
            crate::lang::csharp::analyze_files(root, files)
                .map_err(|e| DispatchError::CSharp(e.to_string()))?
                .files
        }
        // `adapter_group` never yields anything else.
        _ => Vec::new(),
    };

    // Everything above this point is a Tier 1 pipeline: it bound its targets
    // through the file's imports, aliases and declared types. Stamping the
    // resolution here rather than at each of the twenty-odd construction sites
    // means a new edge in any Tier 1 extractor is classified correctly without
    // the author having to remember — and `Resolution::default()` is the
    // *weaker* value, so the failure mode of forgetting is under-claiming
    // rather than marking an unresolved edge gate-safe.
    //
    // Which is exactly why an edge still carrying a placeholder target must go
    // first. Every adapter extracts with a placeholder id and replaces it once
    // it knows the answer, so a surviving placeholder means the adapter could
    // not bind that reference. Stamping one `Resolved` would put a name where a
    // symbol belongs and call it a fact a gate may act on.
    //
    // TypeScript did precisely that. Its resolver dropped an unbound *call*
    // but not an unbound type reference, so `x.field` on a parameter typed by
    // an untyped package recorded an `AccessesProperty` edge to
    // `__UNRESOLVED_LOCAL_TYPE__|Unknowable`, stamped gate-safe, and
    // `blast-radius` counted that name as a dependent symbol.
    //
    // Enforced here rather than in each adapter because seven adapters
    // remembering is seven chances to forget, and the one that forgot was the
    // oldest and most used.
    // The drop is also the measurement. A placeholder that survived the
    // adapter is a reference it could not place, and this is the only point
    // where every language's failures are visible in one shape — which is what
    // makes the number comparable across them.
    //
    // It is worth being precise about why the resolution figure cannot say
    // this. That figure is the share of *recorded edges* that resolved, and an
    // edge exists only once something bound it. A reference that failed leaves
    // no edge, so it never enters the denominator, so failing to bind more
    // references raises the percentage. Counting here supplies the denominator
    // that was missing.
    for file_ir in &mut files_ir {
        let before = file_ir.relationships.len();
        file_ir
            .relationships
            .retain(|rel| !is_placeholder(&rel.to));

        // Added to what the adapter already reported, not replacing it. An
        // adapter counts where it drops a reference it could not place, which
        // is the only place that knows the difference between "bound it" and
        // "could not"; this counts anything that reached here still unbound,
        // which is the adapter having missed a case. Both are references the
        // graph does not describe, and neither should go uncounted.
        file_ir.unbound_references += (before - file_ir.relationships.len()) as u32;
        file_ir.references_counted = true;

        for rel in &mut file_ir.relationships {
            rel.resolution = Resolution::Resolved;
        }
    }

    Ok(files_ir)
}

/// Languages this build can analyse, for help text and diagnostics.
///
/// Built from the enabled features rather than hardcoded, so a slim build
/// reports what it can actually do. The list was always accurate before only
/// because there was exactly one possible build.
pub fn supported_languages() -> Vec<crate::spec::LanguageSupport> {
    crate::spec::specs()
        .into_iter()
        .map(|s| crate::spec::LanguageSupport {
            name: s.name(),
            tier: s.tier(),
        })
        .collect()
}

/// Just the names, for callers that only need a list.
pub fn supported_language_names() -> Vec<&'static str> {
    crate::spec::specs().into_iter().map(|s| s.name()).collect()
}

// ── test edges ───────────────────────────────────────────────

/// Add a `tests` edge for every reference a test makes into non-test code.
///
/// Derived rather than parsed. A test function already records what it calls,
/// instantiates and takes as a type; those edges have been resolved by the
/// language's own pipeline, and the ones that leave the test file are exactly
/// the code the test exercises. Restating them under a distinct kind is what
/// lets "which tests cover this change" be a filter over the graph rather than
/// a second traversal with its own rules.
///
/// Three things this deliberately does not do.
///
/// It adds rather than replaces: the underlying `calls` edge stays, because a
/// test calling a function is still a call and a query for callers that
/// silently dropped tests would under-report the blast radius.
///
/// It ignores references between two test files. A test helper calling another
/// helper is not coverage of the code under test, and counting it would make
/// every test appear to cover the whole suite.
///
/// It carries the underlying edge's resolution rather than asserting its own.
/// A test edge derived from a structural reference is exactly as trustworthy
/// as that reference — which is to say, advisory — and a Tier 2 language must
/// not become gate-safe by passing through this function.
fn add_test_edges(files: &mut [FileIR]) {
    use openinvar_core::ir::RelationshipKind;
    use openinvar_core::symbol_id::{is_external_package, is_placeholder, parse_symbol_id};

    // Which files are tests, decided once. `for_path` walks the spec registry,
    // and a repository has far more edges than files.
    let test_files: BTreeSet<String> = files
        .iter()
        .filter(|f| {
            crate::spec::for_path(&f.file).is_some_and(|spec| spec.is_test_file(&f.file))
        })
        .map(|f| f.file.clone())
        .collect();

    if test_files.is_empty() {
        return;
    }

    for file_ir in files.iter_mut() {
        if !test_files.contains(&file_ir.file) {
            continue;
        }

        let mut derived: Vec<Relationship> = Vec::new();
        // Deduplicated: a test calling the same function four times exercises
        // it once, and four identical edges would inflate every count a reader
        // uses to judge how well covered a symbol is.
        let mut seen: BTreeSet<(String, String)> = BTreeSet::new();

        for relationship in &file_ir.relationships {
            if relationship.kind == RelationshipKind::Tests {
                continue;
            }
            // A test importing `std` does not test `std`, and an unresolved
            // placeholder names nothing at all. Both parse as an id, so both
            // have to be refused explicitly.
            if is_external_package(&relationship.to) || is_placeholder(&relationship.to) {
                continue;
            }
            let Some((target_file, _, _)) = parse_symbol_id(&relationship.to) else {
                continue;
            };
            if target_file == file_ir.file || test_files.contains(target_file) {
                continue;
            }
            if !seen.insert((relationship.from.clone(), relationship.to.clone())) {
                continue;
            }
            derived.push(Relationship {
                from: relationship.from.clone(),
                to: relationship.to.clone(),
                kind: RelationshipKind::Tests,
                alias: relationship.alias.clone(),
                properties_accessed: relationship.properties_accessed.clone(),
                context: "test".to_string(),
                file: relationship.file.clone(),
                line: relationship.line,
                resolution: relationship.resolution,
            });
        }

        // Sorted before appending: the source order of relationships is the
        // adapter's, and a derived set that varied with it would make two runs
        // over one repository disagree.
        derived.sort_by(|a, b| (&a.from, &a.to, a.line).cmp(&(&b.from, &b.to, b.line)));
        file_ir.relationships.extend(derived);
    }
}
