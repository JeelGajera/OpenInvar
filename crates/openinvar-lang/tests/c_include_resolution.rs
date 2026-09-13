// Exercises the c module, so it compiles only when that
// language is enabled. A slim build otherwise tries to compile a test
// for a module it does not carry.
#![cfg(feature = "c")]

#[path = "common/c.rs"]
mod common;

use common::*;
use openinvar_core::ir::RelationshipKind;

#[test]
fn a_local_include_resolves_to_the_header_it_names() {
    // Regression: includes resolved to `local_header::<name>`, which is neither
    // a symbol id nor an `ext::` package, so `add_relationship` dropped every
    // one of them and a C repository produced a graph with zero edges.
    let repo = analyze("language_features");
    let mapper = file(&repo, "src/mapper.c");

    assert!(
        has_edge(
            mapper,
            RelationshipKind::Imports,
            "include/user_payload.h::module::module"
        ),
        "have: {:?}",
        targets(mapper, RelationshipKind::Imports)
    );
}

#[test]
fn a_relative_include_is_normalised_against_the_including_file() {
    let repo = analyze("language_features");
    let render = file(&repo, "src/render.cpp");

    // `#include "../include/shapes.hpp"` from `src/` must reach `include/`.
    assert!(
        has_edge(
            render,
            RelationshipKind::Imports,
            "include/shapes.hpp::module::module"
        ),
        "have: {:?}",
        targets(render, RelationshipKind::Imports)
    );
}

#[test]
fn a_system_include_resolves_to_an_external_package() {
    let repo = analyze("language_features");
    let mapper = file(&repo, "src/mapper.c");

    // `#include <stdio.h>` is outside the repository, but the dependency is
    // still worth recording rather than dropping.
    assert!(
        has_edge(mapper, RelationshipKind::Imports, "ext::stdio.h::package"),
        "have: {:?}",
        targets(mapper, RelationshipKind::Imports)
    );
}

#[test]
fn every_include_edge_points_at_something_the_graph_can_address() {
    let repo = analyze("language_features");
    for f in &repo.files {
        for rel in &f.relationships {
            assert!(
                !openinvar_core::symbol_id::is_placeholder(&rel.to),
                "{} left an unresolved placeholder: {}",
                f.file,
                rel.to
            );
        }
    }
}
