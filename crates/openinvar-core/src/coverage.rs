//! How much of the graph a gate may act on.
//!
//! Every edge records how its target was determined — see [`Resolution`]. An
//! edge bound through the file's imports, aliases and declared types is a fact;
//! an edge matched by name inside one file is a guess that happens to be
//! useful. Both are in the graph, and until now nothing reported the ratio.
//!
//! That ratio is what makes an enforcing tool trustworthy. "No edges broken"
//! means something entirely different in a repository where 98% of references
//! resolved than in one where 40% did, and a gate that cannot tell the two
//! apart will eventually pass a change it should have stopped.

use std::collections::BTreeMap;

use crate::graph::InvarGraph;
use crate::ir::Resolution;

/// Resolved and structural edge counts for some slice of the graph.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Coverage {
    pub resolved: usize,
    pub structural: usize,
}

impl Coverage {
    pub fn total(&self) -> usize {
        self.resolved + self.structural
    }

    /// Share of edges a gate may act on, or `None` when there are no edges.
    ///
    /// `None` rather than 0.0 or 100.0 deliberately: a file with no
    /// relationships has no coverage to report, and printing either number
    /// would state something about it that is not true.
    pub fn percent(&self) -> Option<f64> {
        match self.total() {
            0 => None,
            total => Some(self.resolved as f64 * 100.0 / total as f64),
        }
    }

    fn record(&mut self, resolution: &Resolution) {
        match resolution {
            Resolution::Resolved => self.resolved += 1,
            Resolution::Structural => self.structural += 1,
        }
    }
}

/// Coverage across every edge in the graph.
pub fn overall(graph: &InvarGraph) -> Coverage {
    let mut coverage = Coverage::default();
    for edge in graph.graph.edge_weights() {
        coverage.record(&edge.resolution);
    }
    coverage
}

/// Coverage per source file, keyed by the file the reference was written in.
///
/// A `BTreeMap` because this reaches a user: `HashMap` iteration order is
/// seeded per process, and two runs over one graph would print the same files
/// in a different order.
pub fn by_file(graph: &InvarGraph) -> BTreeMap<String, Coverage> {
    let mut out: BTreeMap<String, Coverage> = BTreeMap::new();
    for edge in graph.graph.edge_weights() {
        out.entry(edge.file.clone())
            .or_default()
            .record(&edge.resolution);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_is_none_rather_than_a_number_when_there_are_no_edges() {
        // A file with no relationships has no coverage. Reporting 0% would say
        // it resolved nothing, and 100% that it resolved everything; both are
        // claims about a file the graph has nothing to say about.
        assert_eq!(Coverage::default().percent(), None);
    }

    #[test]
    fn percent_counts_resolved_against_the_total() {
        let coverage = Coverage {
            resolved: 3,
            structural: 1,
        };
        assert_eq!(coverage.total(), 4);
        assert_eq!(coverage.percent(), Some(75.0));
    }

    #[test]
    fn a_fully_structural_slice_reports_zero_not_none() {
        // The distinction that matters to a gate: "nothing resolved here" is a
        // real answer, and must not look like "nothing to say".
        let coverage = Coverage {
            resolved: 0,
            structural: 5,
        };
        assert_eq!(coverage.percent(), Some(0.0));
    }
}
