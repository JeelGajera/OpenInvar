//! Turning a tool's `kinds` argument into a traversal mask.
//!
//! Shared by every query tool so an agent gets one vocabulary and one error
//! message rather than three that drifted apart.

use openinvar_core::ir::Resolution;
use openinvar_core::query::{self, RelationshipKindMask};

/// Documentation for the `kinds` argument, kept in one place because it is
/// repeated into three tool schemas and would otherwise diverge.
pub const KINDS_DOC: &str = "Optional: only follow these relationship kinds. \
One or more of: imports, calls, extends, implements, uses-type, \
accesses-property, re-exports, instantiates, tests. Omit to follow every kind. \
Which kinds a given repository contains depends on its languages; a filter \
matching no edge in the analyzed graph is reported as such rather than \
returning a bare empty result.";

/// Build a mask from tool arguments.
///
/// An unknown name is an error rather than something to ignore. Silently
/// dropping it would answer a narrower question than the caller asked and
/// present the result as if it were the answer to theirs — which, in a tool
/// used to decide whether a change is safe, is the worst available outcome.
pub fn mask_from_names(names: &Option<Vec<String>>) -> Result<RelationshipKindMask, String> {
    let Some(names) = names else {
        return Ok(RelationshipKindMask::all());
    };
    if names.is_empty() {
        return Ok(RelationshipKindMask::all());
    }

    let mut mask = RelationshipKindMask::none();
    for name in names {
        let kind = query::parse_kind(name).ok_or_else(|| {
            let known: Vec<&str> = query::ALL_KINDS.iter().map(query::kind_name).collect();
            format!(
                "unknown relationship kind '{name}'. Known kinds: {}",
                known.join(", ")
            )
        })?;
        mask = mask.with(kind);
    }
    Ok(mask)
}

/// A warning to prepend when a caller asked only for kinds this graph lacks.
///
/// Without it the reply is an empty result that reads as "nothing depends on
/// this" — the exact false reassurance OpenInvar exists to prevent.
///
/// Asks the analyzed graph rather than a hand-maintained list of unimplemented
/// kinds, so the note stays true as languages gain call edges at different
/// times and says something about this repository rather than about OpenInvar.
pub fn absent_kinds_warning(
    mask: &RelationshipKindMask,
    graph: &openinvar_core::graph::InvarGraph,
) -> Option<String> {
    if mask.is_all() {
        return None;
    }
    let requested = mask.kinds();
    if requested.is_empty() {
        return None;
    }

    let present = query::kinds_present(graph);
    let absent: Vec<&str> = requested
        .iter()
        .filter(|k| !present.contains(k))
        .map(query::kind_name)
        .collect();
    if absent.is_empty() {
        return None;
    }

    Some(if absent.len() == requested.len() {
        format!(
            "NOTE: no {} edge exists in this graph, so this result is empty for \
             that reason alone. This is not evidence that nothing depends on the \
             symbol.",
            absent.join(" or ")
        )
    } else {
        format!(
            "NOTE: {} matched nothing — no such edge exists in this graph.",
            absent.join(", ")
        )
    })
}

/// Doc string for the `min_resolution` parameter, shared by the query tools.
pub const MIN_RESOLUTION_DOC: &str = "Lowest resolution an edge may have to be \
followed and returned. 'resolved' restricts the answer to edges bound through \
imports, aliases and declared types — what a gate may act on. 'structural' \
(the default) also includes edges matched by name inside one file. Edges below \
the threshold are not traversed either, so nothing is reached by way of an \
edge you said you could not trust.";

/// Parse a `min_resolution` parameter, defaulting to the permissive floor.
pub fn resolution_from_name(name: &Option<String>) -> Result<Resolution, String> {
    match name.as_deref() {
        None | Some("structural") => Ok(Resolution::Structural),
        Some("resolved") => Ok(Resolution::Resolved),
        Some(other) => Err(format!(
            "unknown min_resolution '{other}'. Expected 'structural' or 'resolved'."
        )),
    }
}
