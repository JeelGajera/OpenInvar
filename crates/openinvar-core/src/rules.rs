//! `openinvar.toml` — the constraints a repository chooses to enforce.
//!
//! Parsing and validation only. Evaluating a rule against a graph is a
//! separate concern and a separate change; this decides whether a rules file
//! is well formed and says precisely what is wrong when it is not.
//!
//! # Everything fails at parse time or not at all
//!
//! An unknown rule kind, a malformed glob, a threshold of zero — each is
//! rejected when the file is read rather than when the rule is reached. A
//! typo in a rule name would otherwise sit silently in a repository until the
//! day it was supposed to catch something, and a gate that quietly enforces
//! four of your five rules is worse than one that refuses to start.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Display, Formatter};
use std::path::Path;

use serde::Deserialize;

/// Why a rules file was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleError {
    Io(String),
    Syntax(String),
    /// A rule that parsed but cannot mean anything.
    Invalid { rule: String, reason: String },
}

impl Display for RuleError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "cannot read rules file: {err}"),
            Self::Syntax(err) => write!(f, "rules file is not valid TOML: {err}"),
            Self::Invalid { rule, reason } => write!(f, "rule '{rule}': {reason}"),
        }
    }
}

impl std::error::Error for RuleError {}

/// How loudly a violation is reported, and whether it stops anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Reported; exit status unaffected.
    Warn,
    /// Reported, and the command fails.
    #[default]
    Error,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

/// What a rule forbids or limits.
///
/// Each variant names the data it needs, so a rule missing a field it requires
/// fails to parse rather than evaluating against nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleKind {
    /// No dependency edge may run from `from` to `to`.
    ForbidDependency { from: String, to: String },
    /// No reference of any kind may run from `from` to `to`.
    ForbidReference { from: String, to: String },
    /// A named symbol may not lose fields.
    NoFieldRemoval { symbol: String },
    /// No symbol may exceed this many inbound edges.
    MaxFanIn { threshold: usize },
    /// An ordered stack. Each layer may depend downward, never upward.
    ///
    /// Listed top first, so `layers[0]` is the outermost and may reach
    /// everything below it.
    Layers { layers: Vec<String> },
    /// Siblings that may not reference each other in any direction.
    Independence { modules: Vec<String> },
}

impl RuleKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ForbidDependency { .. } => "forbid-dependency",
            Self::ForbidReference { .. } => "forbid-reference",
            Self::NoFieldRemoval { .. } => "no-field-removal",
            Self::MaxFanIn { .. } => "max-fan-in",
            Self::Layers { .. } => "layers",
            Self::Independence { .. } => "independence",
        }
    }

    /// Whether evaluating this needs two graphs rather than one.
    ///
    /// A field removal is only visible in a delta; the rest are properties of
    /// a single graph. Callers use this to skip rules they have no delta for
    /// rather than reporting them as passing.
    pub fn needs_delta(&self) -> bool {
        matches!(self, Self::NoFieldRemoval { .. })
    }
}

/// One rule, as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub name: String,
    pub kind: RuleKind,
    pub severity: Severity,
}

/// A parsed configuration file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rules {
    pub rules: Vec<Rule>,
    /// `[suppress]` — audit finding ids, each with the reason it is allowed.
    ///
    /// Parsed here because it shares the file, not because rules and audit
    /// share anything else. `audit` builds its own type from this map.
    pub suppress: BTreeMap<String, String>,
}

impl Rules {
    /// Rules that can be evaluated against a single graph.
    pub fn without_delta(&self) -> impl Iterator<Item = &Rule> {
        self.rules.iter().filter(|r| !r.kind.needs_delta())
    }

    /// Rules that need a delta.
    pub fn needing_delta(&self) -> impl Iterator<Item = &Rule> {
        self.rules.iter().filter(|r| r.kind.needs_delta())
    }
}

// ── the on-disk shape ────────────────────────────────────────
//
// Deserialized permissively into optional fields, then validated into the
// enum above. `serde`'s own errors for a tagged enum name the internal
// representation rather than the file the user wrote, and a rules file is
// something a person edits by hand.

/// `deny_unknown_fields` because this file's whole contract is that a mistake
/// surfaces when the file is read. Without it a mistyped `[supress]` parses
/// happily and silences nothing, and a mistyped `[[rules]]` yields an empty
/// rule set that reports a clean run — both of which are the silent pass this
/// module exists to prevent.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    #[serde(default)]
    rule: Vec<RawRule>,
    #[serde(default)]
    suppress: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct RawRule {
    name: Option<String>,
    kind: Option<String>,
    from: Option<String>,
    to: Option<String>,
    symbol: Option<String>,
    threshold: Option<i64>,
    layers: Option<Vec<String>>,
    modules: Option<Vec<String>>,
    #[serde(default)]
    severity: Option<Severity>,
}

/// Every rule kind this build understands, for error messages.
const KNOWN_KINDS: &[&str] = &[
    "forbid-dependency",
    "forbid-reference",
    "no-field-removal",
    "max-fan-in",
    "layers",
    "independence",
];

/// Validate a list of path globs used as an ordered or unordered scope set.
///
/// Two entries is the minimum that can mean anything: one layer has nothing to
/// be above, and one module has nobody to stay independent from. A rule that
/// cannot be violated by construction is a rule someone believes is protecting
/// them.
fn check_glob_list(
    rule: &str,
    field: &str,
    values: Option<&Vec<String>>,
) -> Result<Vec<String>, RuleError> {
    let values = values.ok_or_else(|| RuleError::Invalid {
        rule: rule.to_string(),
        reason: format!("'{field}' is required for this kind"),
    })?;

    if values.len() < 2 {
        return Err(RuleError::Invalid {
            rule: rule.to_string(),
            reason: format!(
                "'{field}' needs at least 2 entries to mean anything, got {}",
                values.len()
            ),
        });
    }

    let mut checked = Vec::with_capacity(values.len());
    let mut seen = BTreeSet::new();
    for value in values {
        let pattern = check_glob(rule, field, value)?;
        // A repeated entry is either a copy-paste slip or a belief that the
        // same scope can sit at two depths. Neither should evaluate.
        if !seen.insert(pattern.clone()) {
            return Err(RuleError::Invalid {
                rule: rule.to_string(),
                reason: format!("'{field}' lists '{value}' twice"),
            });
        }
        checked.push(pattern);
    }
    Ok(checked)
}

/// Validate a glob without evaluating it.
///
/// `glob::Pattern` compiles the same syntax the evaluators will use, so a
/// pattern accepted here is one they can run. Rejecting at parse time means a
/// malformed pattern is a startup error rather than a rule that silently
/// matches nothing.
fn check_glob(rule: &str, field: &str, value: &str) -> Result<String, RuleError> {
    if value.trim().is_empty() {
        return Err(RuleError::Invalid {
            rule: rule.to_string(),
            reason: format!("'{field}' is empty"),
        });
    }
    glob::Pattern::new(value).map_err(|err| RuleError::Invalid {
        rule: rule.to_string(),
        reason: format!("'{field}' is not a valid glob: {err}"),
    })?;
    Ok(value.to_string())
}

fn require<'a>(rule: &str, field: &str, value: Option<&'a String>) -> Result<&'a String, RuleError> {
    value.ok_or_else(|| RuleError::Invalid {
        rule: rule.to_string(),
        reason: format!("'{field}' is required for this kind"),
    })
}

fn build(raw: RawRule, index: usize) -> Result<Rule, RuleError> {
    let name = raw
        .name
        .clone()
        .unwrap_or_else(|| format!("rule #{}", index + 1));

    let Some(kind_name) = raw.kind.as_deref() else {
        return Err(RuleError::Invalid {
            rule: name,
            reason: format!("'kind' is required. Expected one of: {}", KNOWN_KINDS.join(", ")),
        });
    };

    let kind = match kind_name {
        "forbid-dependency" => RuleKind::ForbidDependency {
            from: check_glob(&name, "from", require(&name, "from", raw.from.as_ref())?)?,
            to: check_glob(&name, "to", require(&name, "to", raw.to.as_ref())?)?,
        },
        "forbid-reference" => RuleKind::ForbidReference {
            from: check_glob(&name, "from", require(&name, "from", raw.from.as_ref())?)?,
            to: check_glob(&name, "to", require(&name, "to", raw.to.as_ref())?)?,
        },
        "no-field-removal" => {
            let symbol = require(&name, "symbol", raw.symbol.as_ref())?;
            if symbol.trim().is_empty() {
                return Err(RuleError::Invalid {
                    rule: name,
                    reason: "'symbol' is empty".to_string(),
                });
            }
            RuleKind::NoFieldRemoval {
                symbol: symbol.clone(),
            }
        }
        "max-fan-in" => {
            let threshold = raw.threshold.ok_or_else(|| RuleError::Invalid {
                rule: name.clone(),
                reason: "'threshold' is required for this kind".to_string(),
            })?;
            // Zero would forbid every inbound edge in the repository, which is
            // far more likely to be a mistake than an intention.
            if threshold < 1 {
                return Err(RuleError::Invalid {
                    rule: name,
                    reason: format!("'threshold' must be at least 1, got {threshold}"),
                });
            }
            RuleKind::MaxFanIn {
                threshold: threshold as usize,
            }
        }
        "layers" => RuleKind::Layers {
            layers: check_glob_list(&name, "layers", raw.layers.as_ref())?,
        },
        "independence" => RuleKind::Independence {
            modules: check_glob_list(&name, "modules", raw.modules.as_ref())?,
        },
        other => {
            return Err(RuleError::Invalid {
                rule: name,
                reason: format!(
                    "unknown kind '{other}'. Expected one of: {}",
                    KNOWN_KINDS.join(", ")
                ),
            })
        }
    };

    Ok(Rule {
        name,
        kind,
        severity: raw.severity.unwrap_or_default(),
    })
}

/// Parse rules from TOML text.
pub fn parse(text: &str) -> Result<Rules, RuleError> {
    let raw: RawFile =
        toml::from_str(text).map_err(|err| RuleError::Syntax(err.message().to_string()))?;

    let mut rules = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();

    for (index, raw_rule) in raw.rule.into_iter().enumerate() {
        let rule = build(raw_rule, index)?;
        // Two rules under one name make a violation impossible to attribute
        // and a suppression impossible to write.
        if !seen.insert(rule.name.clone()) {
            return Err(RuleError::Invalid {
                rule: rule.name,
                reason: "a rule with this name is already defined".to_string(),
            });
        }
        rules.push(rule);
    }

    Ok(Rules {
        rules,
        suppress: raw.suppress,
    })
}

/// Read and parse a rules file.
pub fn load(path: &Path) -> Result<Rules, RuleError> {
    let text = std::fs::read_to_string(path).map_err(|err| RuleError::Io(err.to_string()))?;
    parse(&text)
}

/// Where a repository's configuration was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Located {
    /// `openinvar.toml` at the repository root. The supported location.
    Root(std::path::PathBuf),
    /// A pre-0.3.0 `.openinvar/rules.toml`.
    ///
    /// Still read, for one release, so an upgrade does not silently stop
    /// enforcing rules — the failure mode this whole module is built against.
    /// Callers are expected to say it is deprecated.
    Legacy(std::path::PathBuf),
}

impl Located {
    pub fn path(&self) -> &Path {
        match self {
            Self::Root(p) | Self::Legacy(p) => p,
        }
    }

    pub fn is_legacy(&self) -> bool {
        matches!(self, Self::Legacy(_))
    }
}

/// The supported location, relative to a repository root.
///
/// At the root, and committed. The previous location was inside `.openinvar/`,
/// which every repository gitignores because the graph database lives there —
/// so the file specified as a repository's versioned constraints could not be
/// versioned, and `check` had nothing to enforce anywhere.
pub fn config_path(root: &Path) -> std::path::PathBuf {
    root.join("openinvar.toml")
}

/// The pre-0.3.0 location.
pub fn legacy_path(root: &Path) -> std::path::PathBuf {
    root.join(".openinvar").join("rules.toml")
}

/// Find a repository's configuration, preferring the supported location.
///
/// `None` means neither exists, which is not an error: a repository that has
/// written no rules is the ordinary case.
pub fn locate(root: &Path) -> Option<Located> {
    let root_file = config_path(root);
    if root_file.is_file() {
        return Some(Located::Root(root_file));
    }
    let legacy = legacy_path(root);
    if legacy.is_file() {
        return Some(Located::Legacy(legacy));
    }
    None
}
