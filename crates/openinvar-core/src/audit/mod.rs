//! Detecting reward-hacking, deterministically.
//!
//! # What makes this different from the rest of the engine
//!
//! Every other finding here describes code. An audit finding describes
//! *conduct*: it says a change looks like it was made to pass a check rather
//! than to work. That is an accusation, and it will sometimes be levelled at a
//! person who did nothing wrong.
//!
//! Three consequences run through this module.
//!
//! **No detector may run on evidence it cannot stand behind.** Accusing
//! someone of tampering on the strength of a name matched inside one file is
//! worse than missing the tampering. Detectors see only symbols in files a
//! Tier 1 adapter resolved, and [`AuditContext::in_scope`] is how that is
//! enforced rather than remembered.
//!
//! **A finding must be identifiable across runs.** CI has to be able to say
//! "this is the finding you already looked at" without the developer
//! re-triaging it every push, so [`FindingId`] is derived from what the
//! finding is *about* and never from where it currently sits. A line number in
//! the id would make an unrelated edit upstream look like a new accusation.
//!
//! **A suppressed finding is reported as suppressed, never omitted.** The
//! whole point of this feature is that it cannot be quietly satisfied, and a
//! gate that silently drops what someone told it to ignore is exactly the
//! thing it exists to catch.

pub mod detectors;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Display, Formatter};

use crate::delta::GraphDelta;
use crate::graph::InvarGraph;
use crate::rules::Severity;

/// How much to believe a finding, as distinct from how bad it would be.
///
/// Severity says what it costs if true; confidence says how sure the detector
/// is. Collapsing them into one number is how a gate ends up either shouting
/// about guesses or whispering about certainties.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Confidence {
    /// Reported, but expected to be wrong sometimes. Warn severity at most.
    Medium,
    /// Near-zero false positives on the fixture corpus.
    High,
}

impl Confidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// A stable identifier for one finding.
///
/// Derived from the detector and the symbols the finding is about — never from
/// a line, a file offset, or anything else that moves when unrelated code
/// changes. That is what lets a suppression written today still match the same
/// finding after a refactor, and what lets CI tell a recurring finding from a
/// new one.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FindingId(String);

impl FindingId {
    /// Build an id from a detector name and the subjects it is about.
    ///
    /// Subjects are sorted before hashing so that a detector which happens to
    /// collect them in a different order on a later run produces the same id.
    pub fn new(detector: &str, subjects: &[&str]) -> Self {
        let mut sorted: Vec<&str> = subjects.to_vec();
        sorted.sort_unstable();
        sorted.dedup();

        let mut hasher = Fnv1a::new();
        hasher.write(detector.as_bytes());
        for subject in sorted {
            // A separator, so ("ab","c") and ("a","bc") do not collide.
            hasher.write(b"\x1f");
            hasher.write(subject.as_bytes());
        }
        FindingId(format!("{detector}-{:08x}", hasher.finish()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for FindingId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// FNV-1a, written out rather than taken from `DefaultHasher`.
///
/// `DefaultHasher` is explicitly not stable across Rust releases, and these
/// ids end up in a checked-in suppression file. An id that changed with the
/// toolchain would silently invalidate every suppression in the repository.
struct Fnv1a(u32);

impl Fnv1a {
    fn new() -> Self {
        Fnv1a(0x811c_9dc5)
    }
    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= *byte as u32;
            self.0 = self.0.wrapping_mul(0x0100_0193);
        }
    }
    fn finish(&self) -> u32 {
        self.0
    }
}

/// One piece of evidence behind a finding.
///
/// Findings carry their evidence rather than only a message because the reader
/// is being asked to accept an accusation. "This test was modified in the same
/// change as the function it covers" is checkable; "suspicious change" is not.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Evidence {
    pub file: String,
    pub line: u32,
    pub detail: String,
}

/// One audit finding.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Finding {
    pub id: FindingId,
    /// The detector that raised it, in kebab-case.
    pub detector: &'static str,
    pub severity: Severity,
    pub confidence: Confidence,
    /// One sentence, in the terms the reader thinks in.
    pub summary: String,
    /// Where a reader should look first.
    pub file: String,
    pub line: u32,
    pub evidence: Vec<Evidence>,
}

/// A finding that a suppression matched.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Suppressed {
    pub finding: Finding,
    /// Why, as written in the ignore file. Empty when none was given.
    pub reason: String,
}

/// What a detector is given.
///
/// Both graphs and the delta between them, plus the set of files a Tier 1
/// adapter resolved. A detector must consult [`Self::in_scope`] before
/// reporting anything about a file.
pub struct AuditContext<'a> {
    pub before: &'a InvarGraph,
    pub after: &'a InvarGraph,
    pub delta: &'a GraphDelta,
    /// Files a Tier 1 adapter resolved.
    ///
    /// Supplied by the caller rather than computed here: tier is a property of
    /// the language adapter, which this crate deliberately does not depend on.
    pub tier_one_files: &'a BTreeSet<String>,
}

impl AuditContext<'_> {
    /// Whether a finding about `file` may be reported at all.
    ///
    /// The gate that keeps an accusation off structural evidence. A file
    /// analysed within-file only has no cross-file references recorded, so
    /// every conclusion drawn from their absence is an artefact of how much
    /// was resolved.
    pub fn in_scope(&self, file: &str) -> bool {
        self.tier_one_files.contains(file)
    }

    /// Whether assertion counts may be compared between the two revisions.
    ///
    /// The second gate, and it exists for the same reason as [`Self::in_scope`].
    /// An assertion count of zero is two completely different statements
    /// depending on whether the language was counted: *this test checks
    /// nothing*, or *this build cannot see what this test checks*. Only the
    /// first supports an accusation.
    ///
    /// Both revisions must have counted the file. A base snapshot written
    /// before the format carried counts reports none counted, so without this
    /// an upgrade would make every test in the repository look emptied at once
    /// — the loudest possible false accusation, on the first run after an
    /// upgrade, which is precisely when trust is thinnest.
    pub fn assertions_comparable(&self, before_file: &str, after_file: &str) -> bool {
        let counted = |graph: &InvarGraph, file: &str| {
            graph.assertions_counted.get(file).map(|v| *v).unwrap_or(false)
        };
        counted(self.before, before_file) && counted(self.after, after_file)
    }
}

/// One deterministic check.
pub trait Detector: Send + Sync {
    /// Kebab-case, and stable: it is half of every finding id this detector
    /// produces, so renaming it invalidates suppressions.
    fn name(&self) -> &'static str;

    /// How bad a true finding is.
    fn severity(&self) -> Severity;

    /// How sure this detector is, measured on the fixture corpus.
    fn confidence(&self) -> Confidence;

    /// One line, shown when listing what ran.
    fn describes(&self) -> &'static str;

    fn detect(&self, context: &AuditContext<'_>) -> Vec<Finding>;
}

/// The result of an audit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuditReport {
    /// Findings that were not suppressed, most severe first.
    pub findings: Vec<Finding>,
    /// Findings a suppression matched. Reported, never omitted.
    pub suppressed: Vec<Suppressed>,
    /// Suppressions that matched nothing this run.
    ///
    /// Dead configuration that hides nothing and gives false comfort — someone
    /// reading the ignore file believes a finding is being held back when it
    /// no longer exists.
    pub stale_suppressions: Vec<String>,
    /// Detectors that ran, in order.
    pub detectors_run: Vec<&'static str>,
    /// Files excluded because no Tier 1 adapter resolved them.
    pub out_of_scope_files: usize,
}

impl AuditReport {
    /// Findings that should fail a gate: error severity, not suppressed.
    pub fn blocking(&self) -> impl Iterator<Item = &Finding> {
        self.findings
            .iter()
            .filter(|f| f.severity == Severity::Error)
    }

    pub fn should_fail(&self) -> bool {
        self.blocking().next().is_some()
    }

    pub fn count_of(&self, severity: Severity) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == severity)
            .count()
    }
}

/// Run `detectors` and apply `suppressions`.
///
/// Detectors run in the order given and their findings are sorted afterwards,
/// so the report does not depend on which detector happened to finish first.
pub fn run(
    detectors: &[&dyn Detector],
    context: &AuditContext<'_>,
    suppressions: &Suppressions,
) -> AuditReport {
    let mut raw: Vec<Finding> = Vec::new();
    let mut detectors_run = Vec::new();

    for detector in detectors {
        detectors_run.push(detector.name());
        for finding in detector.detect(context) {
            // Enforced here as well as trusted to the detector. A detector
            // that forgot to check is a detector that accuses someone on
            // structural evidence, and that must not be one mistake away.
            if !context.in_scope(&finding.file) {
                continue;
            }
            raw.push(finding);
        }
    }

    raw.sort();
    raw.dedup_by(|a, b| a.id == b.id);

    let mut findings = Vec::new();
    let mut suppressed = Vec::new();
    let mut matched: BTreeSet<String> = BTreeSet::new();

    for finding in raw {
        match suppressions.reason_for(&finding.id) {
            Some(reason) => {
                matched.insert(finding.id.as_str().to_string());
                suppressed.push(Suppressed {
                    finding,
                    reason: reason.to_string(),
                });
            }
            None => findings.push(finding),
        }
    }

    // Most severe first: a reader who stops after the first entry should have
    // seen the worst of it.
    findings.sort_by(|a, b| {
        b.severity
            .cmp(&a.severity)
            .then(b.confidence.cmp(&a.confidence))
            .then(a.id.cmp(&b.id))
    });

    let stale_suppressions: Vec<String> = suppressions
        .ids()
        .filter(|id| !matched.contains(*id))
        .map(|id| id.to_string())
        .collect();

    AuditReport {
        findings,
        suppressed,
        stale_suppressions,
        detectors_run,
        out_of_scope_files: 0,
    }
}

// ── suppression ──────────────────────────────────────────────

/// Deliberate exceptions: finding ids, each with the reason it is allowed.
///
/// Written in the `[suppress]` table of `openinvar.toml`:
///
/// ```toml
/// [suppress]
/// "test-tampering-1a2b3c4d" = "the test was rewritten when the API changed"
/// ```
///
/// In a committed file on purpose. Suppressing a finding is an act someone
/// should be able to review, and the previous home — `.openinvar/audit-ignore`
/// — sat inside the gitignored data directory, so the record of what a
/// repository had chosen to ignore could never be shared with anyone.
///
/// Ids rather than patterns, deliberately. A glob would let one line silence a
/// whole detector, and a suppression that broad is indistinguishable from
/// turning the check off — which is a decision that should be visible in the
/// command line, not buried in a file nobody re-reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Suppressions {
    entries: BTreeMap<String, String>,
}

impl Suppressions {
    pub fn parse(text: &str) -> Self {
        let mut entries = BTreeMap::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (id, reason) = match line.split_once('#') {
                Some((id, reason)) => (id.trim(), reason.trim()),
                None => (line, ""),
            };
            if id.is_empty() {
                continue;
            }
            entries.insert(id.to_string(), reason.to_string());
        }
        Suppressions { entries }
    }

    /// Build from the `[suppress]` table of a parsed config file.
    pub fn from_map(entries: BTreeMap<String, String>) -> Self {
        Suppressions { entries }
    }

    /// Read a pre-0.3.0 `audit-ignore` file, treating absence as "nothing
    /// suppressed".
    ///
    /// A missing ignore file is the normal case and not worth an error; an
    /// unreadable one is, because silently suppressing nothing when the author
    /// believes otherwise is the same class of lie as suppressing silently.
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::default());
        }
        std::fs::read_to_string(path)
            .map(|text| Self::parse(&text))
            .map_err(|e| format!("cannot read {}: {e}", path.display()))
    }

    pub fn reason_for(&self, id: &FindingId) -> Option<&str> {
        self.reason_for_id(id.as_str())
    }

    /// Look up by the raw text an ignore file holds.
    ///
    /// The file is written by hand and read back as strings, so a caller
    /// reporting what a line suppresses has no `FindingId` to look up with.
    pub fn reason_for_id(&self, id: &str) -> Option<&str> {
        self.entries.get(id).map(|s| s.as_str())
    }

    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(|s| s.as_str())
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// The pre-0.3.0 location, relative to a repository root.
///
/// Suppressions now live in the `[suppress]` table of `openinvar.toml`. This
/// path is still read for one release so an upgrade does not silently
/// un-suppress a finding somebody deliberately accepted.
pub fn legacy_ignore_path(root: &std::path::Path) -> std::path::PathBuf {
    root.join(".openinvar").join("audit-ignore")
}
