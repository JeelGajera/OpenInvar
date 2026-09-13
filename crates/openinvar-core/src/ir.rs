use serde::{Deserialize, Serialize};

/// Unique identifier for a symbol across the entire codebase.
/// Format: "relative/file/path.ts::SymbolName::kind"
/// Example: "src/models/user.ts::UserPayload::class"
/// Example: "src/models/user.ts::UserPayload::userId::property"
pub type SymbolId = String;

/// A symbol — any named entity in source code.
///
/// Compares by value: `delta` pairs symbols across two graphs and needs to say
/// whether two of them are the same, and every field is already a plain value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Symbol {
    pub id: SymbolId,
    pub name: String,
    pub kind: SymbolKind,
    pub language: Language,
    pub file: String,
    pub line_start: u32,
    pub line_end: u32,
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    Class,
    Interface,
    TypeAlias,
    Function,
    Method,
    Property,
    Variable,
    Module,
    Enum,
    EnumVariant,
    ExternalPackage,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Language {
    TypeScript,
    JavaScript,
    Python,
    Rust,
    Go,
    Java,
    C,
    Cpp,
    Ruby,
    Php,
    CSharp,
    Kotlin,
    Swift,
    Sql,
}

/// How an edge's target was determined, and therefore what it is worth.
///
/// # Why this is on the edge rather than on the language
///
/// A polyglot graph mixes both in one answer. `blast-radius` over a repository
/// with TypeScript and Java returns rows from each, and before this existed a
/// reader could not tell which ones a gate may act on. The tier is a property
/// of the analysis that produced the edge, so the edge is where it belongs.
///
/// Ordered so `Structural < Resolved` reads naturally, and so the weakest
/// resolution in a set is its minimum.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Resolution {
    /// The target was matched by name inside one file, with no import table in
    /// play. A tags query reports that a call to `foo` happened; it does not
    /// say which `foo`. Advisory only.
    Structural,
    /// The target was bound through the file's imports, aliases and declared
    /// types. This is the resolution a gate may act on.
    Resolved,
}

impl Resolution {
    /// Whether this resolution is at least as strong as `floor`.
    ///
    /// The ladder has two rungs today. It is written as a comparison rather
    /// than an equality test so that adding a rung between them — a name-match
    /// heuristic, say — changes this one function and nothing that calls it.
    pub fn meets(self, floor: Resolution) -> bool {
        self.rank() >= floor.rank()
    }

    fn rank(self) -> u8 {
        match self {
            Resolution::Structural => 0,
            Resolution::Resolved => 1,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Resolution::Structural => "structural",
            Resolution::Resolved => "resolved",
        }
    }

    /// Whether a gate may draw a conclusion from an edge resolved this way.
    ///
    /// Structural resolution cannot see across files, so "nothing references
    /// this" from such an edge is a statement about one file rather than about
    /// the repository. A gate treating it otherwise would report a pass it had
    /// not earned.
    pub fn is_gate_safe(&self) -> bool {
        matches!(self, Resolution::Resolved)
    }
}

/// Edges predate the resolution field, and a stored graph written before it
/// existed carries none. Defaulting to the weaker value keeps an old graph
/// from being read as gate-safe on the strength of a missing field.
impl Default for Resolution {
    fn default() -> Self {
        Resolution::Structural
    }
}

/// A directed relationship between two symbols.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    pub from: SymbolId,
    pub to: SymbolId,
    pub kind: RelationshipKind,
    pub alias: Option<String>,
    pub properties_accessed: Vec<String>,
    pub context: String,
    pub file: String,
    pub line: u32,
    /// How `to` was determined. See [`Resolution`].
    #[serde(default)]
    pub resolution: Resolution,
}

/// `Ord` is derived because collections keyed by kind reach the user, and a
/// stable order there is the product's first guarantee. The order is the
/// declaration order below and carries no meaning beyond being fixed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RelationshipKind {
    Imports,
    Calls,
    Extends,
    Implements,
    UsesType,
    AccessesProperty,
    ReExports,
    Instantiates,
    /// A test exercises this symbol.
    ///
    /// Derived rather than parsed: a test's own call and type references say
    /// what it touches, and this restates the ones that leave the test file.
    /// Keeping it a distinct kind rather than a flag on `Calls` is what lets
    /// "which tests cover this change" be a filter over the graph instead of a
    /// second traversal with different rules.
    Tests,
}

// diagnostics

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DiagnosticLevel {
    /// Actual tree-sitter parse failures
    Error,
    /// Local relative import target not found, symbol not resolved
    Warning,
    /// Skipped minified file, .d.ts excluded, etc.
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DiagnosticCategory {
    /// Tree-sitter parse errors
    Parse,
    /// Unresolved imports, symbols, types
    Resolution,
    /// Skipped minified or bundled files
    Skip,
    /// Excluded by file policy rules
    Policy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub level: DiagnosticLevel,
    pub category: DiagnosticCategory,
    pub message: String,
    pub file: Option<String>,
    pub line: Option<u32>,
}

// re-export tracking (for barrel chain resolution)

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReExportEntry {
    /// Name being re-exported (e.g. "renderBlockquote")
    pub exported_name: String,
    /// Module specifier (e.g. "./blockquote")
    pub source_module: String,
}

/// The complete IR output from one file parse.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileIR {
    pub file: String,
    pub language: Language,
    pub symbols: Vec<Symbol>,
    pub relationships: Vec<Relationship>,
    pub diagnostics: Vec<Diagnostic>,
    pub re_exports: Vec<ReExportEntry>,
}

/// The complete IR output from a full repo or incremental update.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoIR {
    pub root: String,
    pub files: Vec<FileIR>,
    /// File counts per language, ordered by language name.
    ///
    /// Ordered rather than hashed because this is serialized: `HashMap`
    /// iteration order varies per process, so two runs over identical input
    /// emitted the same counts in a different order. Determinism is the
    /// property the graph is built to guarantee, and it has to hold all the
    /// way out to the bytes on stdout.
    pub language_stats: std::collections::BTreeMap<String, usize>,
}
