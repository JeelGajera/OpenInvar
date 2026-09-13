use crate::ir::Language;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, Default)]
pub struct ScanConfig {
    pub include_patterns: Vec<String>,
    pub exclude_patterns: Vec<String>,
    pub respect_gitignore: bool,
}

impl ScanConfig {
    pub fn default_enabled() -> Self {
        Self {
            include_patterns: Vec::new(),
            exclude_patterns: Vec::new(),
            respect_gitignore: true,
        }
    }
}

pub fn parse_csv_patterns(raw: Option<&str>) -> Vec<String> {
    raw.unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| p.replace('\\', "/"))
        .collect()
}

pub fn path_matches_any(path: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|p| pattern_matches(path, p))
}

pub fn pattern_matches(path: &str, pattern: &str) -> bool {
    let path = normalize(path);
    let pattern = normalize(pattern);

    if pattern.is_empty() {
        return false;
    }

    if !pattern.contains('/') {
        if wildcard_match(&path, &pattern) {
            return true;
        }
        return path.split('/').any(|seg| wildcard_match(seg, &pattern));
    }

    if let Some(tail) = pattern.strip_prefix("**/") {
        return path.split('/').enumerate().any(|(idx, _)| {
            wildcard_match(
                &path.split('/').skip(idx).collect::<Vec<_>>().join("/"),
                tail,
            )
        });
    }

    if wildcard_match(&path, &pattern) {
        return true;
    }

    if path.len() >= pattern.len() {
        return path.ends_with(&pattern);
    }

    false
}

fn wildcard_match(text: &str, pattern: &str) -> bool {
    let t = text.as_bytes();
    let p = pattern.as_bytes();

    let (mut ti, mut pi) = (0usize, 0usize);
    let mut star = None::<usize>;
    let mut match_i = 0usize;

    while ti < t.len() {
        if pi < p.len() && (p[pi] == b'?' || p[pi] == t[ti]) {
            ti += 1;
            pi += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star = Some(pi);
            pi += 1;
            match_i = ti;
        } else if let Some(star_pos) = star {
            pi = star_pos + 1;
            match_i += 1;
            ti = match_i;
        } else {
            return false;
        }
    }

    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }

    pi == p.len()
}

fn normalize(input: &str) -> String {
    input
        .trim()
        .replace('\\', "/")
        .trim_start_matches("./")
        .trim_matches('/')
        .to_string()
}

#[derive(Debug, Clone)]
pub struct GitignoreRule {
    pub pattern: String,
    pub negated: bool,
    pub directory_only: bool,
}

pub fn load_root_gitignore_rules(root: &Path) -> Vec<GitignoreRule> {
    let path = root.join(".gitignore");
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };

    content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let negated = line.starts_with('!');
            let mut body = if negated { &line[1..] } else { line };
            body = body.trim();
            if body.is_empty() {
                return None;
            }

            let directory_only = body.ends_with('/');
            let pattern = body.trim_end_matches('/').replace('\\', "/");
            Some(GitignoreRule {
                pattern,
                negated,
                directory_only,
            })
        })
        .collect()
}

pub fn is_ignored_by_rules(rel_path: &str, _is_dir: bool, rules: &[GitignoreRule]) -> bool {
    let path = normalize(rel_path);
    if path.is_empty() {
        return false;
    }

    let mut ignored = false;

    for rule in rules {
        let anchored = rule.pattern.starts_with('/');
        let rule_pattern = rule.pattern.trim_start_matches('/');
        let dir_prefix = format!("{rule_pattern}/");

        let matches_candidate = |candidate: &str| {
            if rule.directory_only {
                candidate == rule_pattern || candidate.starts_with(&dir_prefix)
            } else {
                pattern_matches(candidate, rule_pattern)
            }
        };

        let matched = if anchored {
            matches_candidate(&path)
        } else {
            matches_candidate(&path)
                || path.split('/').enumerate().any(|(idx, _)| {
                    matches_candidate(&path.split('/').skip(idx).collect::<Vec<_>>().join("/"))
                })
        };

        if matched {
            ignored = !rule.negated;
        }
    }

    ignored
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_parse_csv_patterns() {
        let patterns = parse_csv_patterns(Some("src/**,  tests/*.ts , ,node_modules/**"));
        assert_eq!(
            patterns,
            vec![
                "src/**".to_string(),
                "tests/*.ts".to_string(),
                "node_modules/**".to_string()
            ]
        );
    }

    #[test]
    fn test_pattern_matches_globs_and_suffix() {
        assert!(pattern_matches("src/a/b/file.ts", "src/**"));
        assert!(pattern_matches("src/a/b/file.ts", "**/*.ts"));
        assert!(pattern_matches("src/a/b/file.ts", "*.ts"));
        assert!(pattern_matches("src/a/b/file.ts", "a/b/file.ts"));
        assert!(!pattern_matches("src/a/b/file.ts", "*.tsx"));
    }

    #[test]
    fn test_gitignore_rule_evaluation_with_negation() {
        let rules = vec![
            GitignoreRule {
                pattern: "dist".to_string(),
                negated: false,
                directory_only: true,
            },
            GitignoreRule {
                pattern: "dist/keep.ts".to_string(),
                negated: true,
                directory_only: false,
            },
        ];

        assert!(is_ignored_by_rules("dist", true, &rules));
        assert!(is_ignored_by_rules("dist/a.ts", false, &rules));
        assert!(!is_ignored_by_rules("dist/keep.ts", false, &rules));
    }

    #[test]
    fn test_include_patterns_descend_correctly_into_subdirectories() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/scan/monorepo");
        let config = ScanConfig {
            include_patterns: vec!["projects/frontend/web-portal/**".to_string()],
            exclude_patterns: Vec::new(),
            respect_gitignore: false,
        };

        let files = walk_source_files_with_config(&root, &config, |_| true)
            .expect("walk should succeed for monorepo fixture");
        let rel_files: Vec<String> = files
            .iter()
            .filter_map(|p| {
                p.strip_prefix(&root)
                    .ok()
                    .map(|r| r.to_string_lossy().replace('\\', "/"))
            })
            .collect();

        assert!(
            rel_files
                .iter()
                .any(|f| f.ends_with("projects/frontend/web-portal/src/App.ts")),
            "should include App.ts under nested include path"
        );
        assert!(
            rel_files
                .iter()
                .any(|f| f.ends_with("projects/frontend/web-portal/src/Component.ts")),
            "should include Component.ts under nested include path"
        );
    }

    #[test]
    fn test_include_with_double_star_finds_nested_files() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/scan/monorepo");
        let config = ScanConfig {
            include_patterns: vec!["projects/**/*.ts".to_string()],
            exclude_patterns: Vec::new(),
            respect_gitignore: false,
        };

        let files = walk_source_files_with_config(&root, &config, |_| true)
            .expect("walk should succeed for recursive glob include");
        let rel_files: Vec<String> = files
            .iter()
            .filter_map(|p| {
                p.strip_prefix(&root)
                    .ok()
                    .map(|r| r.to_string_lossy().replace('\\', "/"))
            })
            .collect();

        assert_eq!(
            rel_files.len(),
            3,
            "recursive include should find all three fixture TypeScript files"
        );
        assert!(
            rel_files
                .iter()
                .any(|f| f.ends_with("projects/api/administration/src/AdminService.ts")),
            "recursive include should include AdminService.ts"
        );
    }
}

pub fn walk_source_files_with_config<F>(
    root: &Path,
    config: &ScanConfig,
    is_supported: F,
) -> Result<Vec<PathBuf>, std::io::Error>
where
    F: Fn(&Path) -> bool,
{
    let mut out = Vec::new();
    let rules = if config.respect_gitignore {
        load_root_gitignore_rules(root)
    } else {
        Vec::new()
    };

    for entry in WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| should_descend(root, e.path(), config, &rules))
    {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() || !is_supported(path) {
            continue;
        }

        if should_include_file(root, path, config, &rules) {
            out.push(path.to_path_buf());
        }
    }

    out.sort();
    Ok(out)
}

/// True if a file looks machine-generated and should be skipped.
///
/// Generated code parses fine, but its symbols are noise in an impact graph:
/// nobody edits it by hand, and it is regenerated wholesale rather than
/// refactored. Shared across adapters so every language applies the same rule.
///
/// Only the first few lines are inspected — the convention across generators
/// (protoc, bindgen, `go generate`, OpenAPI) is a banner comment at the top.
pub fn looks_generated(source: &str, path: &Path) -> bool {
    const MARKERS: &[&str] = &[
        "code generated by",
        "auto-generated",
        "autogenerated",
        "automatically generated",
        "do not edit",
        "@generated",
    ];

    let banner: String = source
        .lines()
        .take(8)
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
    if MARKERS.iter().any(|m| banner.contains(m)) {
        return true;
    }

    let path_str = path.to_string_lossy().to_ascii_lowercase();
    // `.pb.` and `_pb2` are the protobuf conventions; `.g.` is common for
    // codegen in Dart/Freezed-style toolchains.
    path_str.contains(".pb.") || path_str.contains("_pb2.") || path_str.contains(".g.")
}

/// What a scan covered, and what it chose to leave out.
///
/// Reported so that a smaller-than-expected file count is explainable. A
/// directory silently missing from the graph is indistinguishable from a
/// parser bug, and users reasonably assume the latter.
#[derive(Debug, Default, Clone)]
pub struct ScanReport {
    pub files: Vec<PathBuf>,
    /// Directories pruned by a built-in default rule, which `--include` can override.
    pub skipped_dirs: std::collections::BTreeSet<String>,
}

/// Walk for source files, recording which directories the defaults pruned.
pub fn walk_source_files_reporting<F>(
    root: &Path,
    config: &ScanConfig,
    is_supported: F,
) -> Result<ScanReport, std::io::Error>
where
    F: Fn(&Path) -> bool,
{
    let mut report = ScanReport::default();
    let rules = if config.respect_gitignore {
        load_root_gitignore_rules(root)
    } else {
        Vec::new()
    };

    let skipped = std::cell::RefCell::new(std::collections::BTreeSet::new());

    for entry in WalkDir::new(root).into_iter().filter_entry(|e| {
        let descend = should_descend(root, e.path(), config, &rules);
        if !descend && e.path().is_dir() {
            if let Ok(relative) = e.path().strip_prefix(root) {
                let rel = relative.to_string_lossy().replace('\\', "/");
                // Only report prunes a user could reverse; `.git` and explicit
                // `--exclude` patterns are not surprises worth reporting.
                if let Some(name) = rel.rsplit('/').next() {
                    if DEFAULT_EXCLUDE_DIRS.contains(&name) {
                        skipped.borrow_mut().insert(name.to_string());
                    }
                }
            }
        }
        descend
    }) {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() || !is_supported(path) {
            continue;
        }
        if should_include_file(root, path, config, &rules) {
            report.files.push(path.to_path_buf());
        }
    }

    report.files.sort();
    report.skipped_dirs = skipped.into_inner();
    Ok(report)
}

pub fn detect_language_from_extension(ext: &str) -> Option<Language> {
    match ext.to_ascii_lowercase().as_str() {
        "py" | "pyi" => Some(Language::Python),
        "rs" => Some(Language::Rust),
        "go" => Some(Language::Go),
        "c" | "h" => Some(Language::C),
        "cpp" | "cc" | "cxx" | "hpp" | "hxx" | "hh" => Some(Language::Cpp),
        "ts" | "tsx" | "mts" | "cts" | "vue" | "svelte" | "astro" => Some(Language::TypeScript),
        "js" | "jsx" | "mjs" | "cjs" => Some(Language::JavaScript),
        "java" => Some(Language::Java),
        "rb" => Some(Language::Ruby),
        "php" => Some(Language::Php),
        "cs" => Some(Language::CSharp),
        "kt" | "kts" => Some(Language::Kotlin),
        "swift" => Some(Language::Swift),
        "sql" => Some(Language::Sql),
        _ => None,
    }
}

pub fn is_any_supported_source_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .and_then(detect_language_from_extension)
        .is_some()
}

/// Directories that never contain source worth indexing, in any language.
///
/// Deliberately conservative. An earlier revision also listed `env`, `gen`,
/// `generated`, `proto` and `vendor`; those are ordinary source directory names
/// in plenty of projects (`src/env/` for configuration, `src/gen/` for
/// hand-written generator code), and because this check ran before
/// `--include` was consulted there was no way to opt back in. Files simply
/// vanished from the graph with no diagnostic, which reads as a parser bug.
///
/// Anything here can still be reached with an explicit `--include` pattern —
/// see [`should_include_relative_path`] — except `.git`, which is never source.
const DEFAULT_EXCLUDE_DIRS: &[&str] = &[
    // Package manager and build output
    "node_modules",
    "dist",
    "build",
    "out",
    "target",
    "coverage",
    // Framework caches
    ".next",
    ".nuxt",
    ".output",
    ".cache",
    ".turbo",
    ".parcel-cache",
    // Python environments and caches
    "__pycache__",
    ".venv",
    "venv",
    ".eggs",
    ".mypy_cache",
    ".pytest_cache",
    ".tox",
    "site-packages",
    // C/C++ build trees
    "CMakeFiles",
    "_deps",
    ".cmake",
    // Ours
    ".openinvar",
];

/// Never indexed, and not reachable even with an explicit `--include`.
const ALWAYS_EXCLUDE_DIRS: &[&str] = &[".git", ".hg", ".svn"];

const DEFAULT_EXCLUDE_SUFFIXES: &[&str] = &[
    ".d.ts", ".d.mts", ".d.cts", // TypeScript declarations
    ".min.js", ".min.mjs", // Minified JS
    ".min.css", // Minified CSS
    ".map",     // Source maps
    ".pyc", ".pyo", ".o", ".a", ".so", ".dll", ".obj", ".lib", ".exe", ".pb.c", ".pb.h", "_gen.c",
    "_gen.h", ".pb.rs",
];

pub fn should_include_relative_path(
    relative_path: &str,
    is_dir: bool,
    config: &ScanConfig,
    rules: &[GitignoreRule],
) -> bool {
    let rel = relative_path.replace('\\', "/");

    if rel.is_empty() || rel == "." {
        return true;
    }

    // Version-control metadata is never source, and no flag reaches it.
    for segment in rel.split('/') {
        if ALWAYS_EXCLUDE_DIRS.contains(&segment) {
            return false;
        }
    }

    // An explicit exclude always wins: the user asked for it by name.
    if !config.exclude_patterns.is_empty() && path_matches_any(&rel, &config.exclude_patterns) {
        return false;
    }

    // An explicit include also wins, over the built-in defaults below. Naming a
    // path on the command line is an unambiguous statement of intent, and
    // without this a directory matching a default exclude could not be indexed
    // at all — `--include 'src/env/**'` would silently match nothing.
    let explicitly_included =
        !config.include_patterns.is_empty() && path_matches_any(&rel, &config.include_patterns);

    if !explicitly_included {
        for segment in rel.split('/') {
            if DEFAULT_EXCLUDE_DIRS.contains(&segment) {
                return false;
            }
        }

        if config.respect_gitignore && is_ignored_by_rules(&rel, is_dir, rules) {
            return false;
        }
    }

    // Compiled output and declaration files carry no symbols a user would edit,
    // whatever the include patterns say.
    if !is_dir {
        for suffix in DEFAULT_EXCLUDE_SUFFIXES {
            if rel.ends_with(suffix) {
                return false;
            }
        }
    }

    if config.include_patterns.is_empty() {
        return true;
    }

    explicitly_included
}

fn should_descend(root: &Path, path: &Path, config: &ScanConfig, rules: &[GitignoreRule]) -> bool {
    if !path.is_dir() {
        return true;
    }

    let Ok(relative) = path.strip_prefix(root) else {
        return true;
    };
    let rel = relative.to_string_lossy().replace('\\', "/");
    if rel.is_empty() {
        return true;
    }

    for segment in rel.split('/') {
        if ALWAYS_EXCLUDE_DIRS.contains(&segment) {
            return false;
        }
    }

    if !config.exclude_patterns.is_empty() && path_matches_any(&rel, &config.exclude_patterns) {
        return false;
    }

    // Mirror `should_include_relative_path`: if an include pattern could match
    // inside this directory, descend even when a default exclude names it.
    let include_reaches_here = !config.include_patterns.is_empty()
        && config
            .include_patterns
            .iter()
            .any(|pattern| directory_could_contain_match(&rel, pattern));

    if !include_reaches_here {
        for segment in rel.split('/') {
            if DEFAULT_EXCLUDE_DIRS.contains(&segment) {
                return false;
            }
        }

        if config.respect_gitignore && is_ignored_by_rules(&rel, true, rules) {
            return false;
        }

        if !config.include_patterns.is_empty() {
            return false;
        }
    }

    true
}

fn directory_could_contain_match(dir_rel: &str, pattern: &str) -> bool {
    let dir = normalize(dir_rel);
    let pat = normalize(pattern);

    if dir.is_empty() || pat.is_empty() {
        return true;
    }

    if pat.starts_with("**") {
        return true;
    }

    let pat_no_globstar = pat.strip_prefix("**/").unwrap_or(&pat);
    let fixed_prefix = pat_no_globstar
        .split('*')
        .next()
        .unwrap_or("")
        .trim_matches('/');

    if fixed_prefix.is_empty() {
        return true;
    }

    if fixed_prefix.starts_with(&dir) || dir.starts_with(fixed_prefix) {
        return true;
    }

    pattern_matches(&format!("{dir}/dummy"), &pat)
}

fn should_include_file(
    root: &Path,
    path: &Path,
    config: &ScanConfig,
    rules: &[GitignoreRule],
) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };

    let rel = relative.to_string_lossy().replace('\\', "/");
    should_include_relative_path(&rel, false, config, rules)
}

#[cfg(test)]
mod default_exclude_tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/scan/default_excludes")
    }

    fn scan(config: ScanConfig) -> Vec<String> {
        let root = fixture();
        let report = walk_source_files_reporting(&root, &config, |p| {
            p.extension().and_then(|e| e.to_str()) == Some("ts")
        })
        .expect("scan succeeds");
        report
            .files
            .iter()
            .filter_map(|p| {
                p.strip_prefix(&root)
                    .ok()
                    .map(|r| r.to_string_lossy().replace('\\', "/"))
            })
            .collect()
    }

    #[test]
    fn ordinary_source_directory_names_are_no_longer_excluded_by_default() {
        // `env`, `gen` and `proto` were added to the default exclude list, which
        // silently removed `src/env/` — a common configuration directory — from
        // every existing TypeScript project's graph.
        let files = scan(ScanConfig {
            include_patterns: Vec::new(),
            exclude_patterns: Vec::new(),
            respect_gitignore: false,
        });

        assert!(
            files.iter().any(|f| f.contains("src/env/")),
            "src/env is ordinary source; got {files:?}"
        );
        assert!(
            files.iter().any(|f| f.contains("src/gen/")),
            "src/gen is ordinary source; got {files:?}"
        );
    }

    #[test]
    fn genuine_build_output_is_still_excluded_by_default() {
        let files = scan(ScanConfig {
            include_patterns: Vec::new(),
            exclude_patterns: Vec::new(),
            respect_gitignore: false,
        });
        assert!(
            !files.iter().any(|f| f.contains("node_modules")),
            "got {files:?}"
        );
    }

    #[test]
    fn an_explicit_include_overrides_a_default_exclusion() {
        // Default excludes used to be checked before include patterns, so a
        // directory the defaults named could not be indexed at all —
        // `--include 'src/node_modules/**'` matched nothing and reported no
        // reason why.
        let files = scan(ScanConfig {
            include_patterns: vec!["src/node_modules/**".to_string()],
            exclude_patterns: Vec::new(),
            respect_gitignore: false,
        });

        assert!(
            files.iter().any(|f| f.contains("src/node_modules/")),
            "naming a path explicitly is an unambiguous request for it; got {files:?}"
        );
    }

    #[test]
    fn an_explicit_exclude_still_wins_over_an_include() {
        let files = scan(ScanConfig {
            include_patterns: vec!["src/**".to_string()],
            exclude_patterns: vec!["src/ok/**".to_string()],
            respect_gitignore: false,
        });
        assert!(
            !files.iter().any(|f| f.contains("src/ok/")),
            "got {files:?}"
        );
        assert!(
            files.iter().any(|f| f.contains("src/env/")),
            "got {files:?}"
        );
    }

    #[test]
    fn skipped_directories_are_reported_so_a_short_file_count_is_explainable() {
        let root = fixture();
        let report = walk_source_files_reporting(
            &root,
            &ScanConfig {
                include_patterns: Vec::new(),
                exclude_patterns: Vec::new(),
                respect_gitignore: false,
            },
            |p| p.extension().and_then(|e| e.to_str()) == Some("ts"),
        )
        .expect("scan succeeds");

        assert!(
            report.skipped_dirs.contains("node_modules"),
            "a pruned directory must be reported, got {:?}",
            report.skipped_dirs
        );
    }
}
