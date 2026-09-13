//! Structural interface satisfaction.
//!
//! Go has no `implements` keyword: a type satisfies an interface by having its
//! methods. That makes interfaces invisible to a purely syntactic reading, and
//! it is exactly the relationship a blast-radius query needs — changing a
//! method signature can break an interface conformance stated nowhere in the
//! source.
//!
//! Method sets come from the symbol table, where the extractor records both
//! interface methods and concrete methods under ids qualified by their owning
//! type. The previous implementation recovered them by splitting signature text
//! on whitespace and punctuation, which treated parameter names and types as
//! method names and produced conformance matches that were essentially random.

use std::collections::{BTreeMap, BTreeSet};

use openinvar_core::ir::{Relationship, RelationshipKind, RepoIR, Resolution, SymbolKind};
use openinvar_core::symbol_id::parse_symbol_id;

pub fn detect_implementations(repo_ir: &mut RepoIR) {
    // Package scope is directory scope in Go: a type's methods may live in any
    // file of its package.
    let mut interfaces: BTreeMap<String, (String, BTreeSet<String>, String)> = BTreeMap::new();
    let mut concrete: BTreeMap<(String, String), (String, BTreeSet<String>, String)> =
        BTreeMap::new();

    for file in &repo_ir.files {
        let package = directory_of(&file.file);

        for symbol in &file.symbols {
            match symbol.kind {
                SymbolKind::Interface => {
                    interfaces
                        .entry(symbol.id.clone())
                        .or_insert_with(|| (symbol.name.clone(), BTreeSet::new(), package.clone()));
                }
                SymbolKind::Class => {
                    concrete
                        .entry((package.clone(), symbol.name.clone()))
                        .or_insert_with(|| (symbol.id.clone(), BTreeSet::new(), file.file.clone()));
                }
                _ => {}
            }
        }
    }

    // Attribute each method to the type that owns it.
    for file in &repo_ir.files {
        let package = directory_of(&file.file);

        for symbol in &file.symbols {
            if symbol.kind != SymbolKind::Method {
                continue;
            }
            let Some((owner, method)) = owner_and_method(&symbol.id) else {
                continue;
            };

            // Interface methods: the owner is an interface declared in this file.
            let interface_id = file
                .symbols
                .iter()
                .find(|s| s.kind == SymbolKind::Interface && s.name == owner)
                .map(|s| s.id.clone());
            if let Some(id) = interface_id {
                if let Some(entry) = interfaces.get_mut(&id) {
                    entry.1.insert(method.to_string());
                }
                continue;
            }

            if let Some(entry) = concrete.get_mut(&(package.clone(), owner.to_string())) {
                entry.1.insert(method.to_string());
            }
        }
    }

    // A type implements an interface when it has every method the interface
    // requires. Empty interfaces (`any`) are satisfied by everything and carry
    // no information, so they are skipped.
    let mut new_edges: Vec<(String, Relationship)> = Vec::new();

    for (struct_id, methods, struct_file) in concrete.values() {
        if methods.is_empty() {
            continue;
        }
        for (interface_id, (interface_name, required, interface_package)) in &interfaces {
            if required.is_empty() || !required.is_subset(methods) {
                continue;
            }
            // Only match within the same package, or against interfaces the
            // package can see. Cross-package structural matching produces a
            // combinatorial number of edges that are technically true and
            // practically noise.
            if interface_package != &directory_of(struct_file) {
                continue;
            }
            new_edges.push((
                struct_file.clone(),
                Relationship {
                    from: struct_id.clone(),
                    to: interface_id.clone(),
                    kind: RelationshipKind::Implements,
                    alias: None,
                    properties_accessed: required.iter().cloned().collect(),
                    context: format!("satisfies {interface_name} (method set match)"),
                    file: struct_file.clone(),
                    line: 0,
                    resolution: Resolution::default(),
                },
            ));
        }
    }

    for (file_path, edge) in new_edges {
        if let Some(file) = repo_ir.files.iter_mut().find(|f| f.file == file_path) {
            file.relationships.push(edge);
        }
    }
}

/// Split a qualified method id's name into `(owner, method)`.
fn owner_and_method(symbol_id: &str) -> Option<(&str, &str)> {
    let (_, name, _) = parse_symbol_id(symbol_id)?;
    let cut = name.rfind("::")?;
    Some((&name[..cut], &name[cut + 2..]))
}

fn directory_of(file: &str) -> String {
    match file.rfind('/') {
        Some(cut) => file[..cut].to_string(),
        None => ".".to_string(),
    }
}
