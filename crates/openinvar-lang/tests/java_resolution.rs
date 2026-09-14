//! Java name resolution, which is what the Tier 1 promotion is.
//!
//! Java resolves a simple name in a defined order, and the order is the whole
//! point: a type declared in the file shadows an import, the file's own
//! package is searched before any on-demand import, and two wildcard imports
//! offering one name is an ambiguity `javac` rejects rather than a coin toss.
//!
//! Every test below is a case where getting the order wrong still produces *an*
//! answer — just the wrong one. That is the failure mode this adapter exists to
//! avoid, and it is invisible without tests like these.

#![cfg(feature = "java")]

use std::path::PathBuf;

use openinvar_core::ir::{FileIR, RelationshipKind};

/// Write `files` into a scratch directory and analyse them.
fn analyze(name: &str, files: &[(&str, &str)]) -> Vec<FileIR> {
    let root = std::env::temp_dir().join(format!(
        "openinvar-java-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let mut paths = Vec::new();
    for (path, source) in files {
        let full = root.join(path);
        std::fs::create_dir_all(full.parent().expect("parent")).expect("mkdir");
        std::fs::write(&full, source).expect("write");
        paths.push(full);
    }
    paths.sort();

    let ir = openinvar_lang::lang::java::analyze_files(&root, &paths).expect("analysis");
    let _ = std::fs::remove_dir_all(&root);
    ir.files
}

/// Targets of edges of `kind` leaving any symbol in `file`, as readable names.
fn targets(files: &[FileIR], file: &str, kind: RelationshipKind) -> Vec<String> {
    let mut out: Vec<String> = files
        .iter()
        .filter(|f| f.file.ends_with(file))
        .flat_map(|f| f.relationships.iter())
        .filter(|r| r.kind == kind)
        .map(|r| {
            // `dir/Thing.java::Owner::member::method` reads better as
            // `Thing.Owner::member`.
            let (path, rest) = r.to.split_once("::").unwrap_or(("", &r.to));
            let stem = path.rsplit('/').next().unwrap_or(path);
            format!("{stem}::{rest}")
        })
        .collect();
    out.sort();
    out
}

#[test]
fn a_type_in_the_same_package_resolves_without_an_import() {
    // Java's implicit same-package visibility. Miss it and every intra-package
    // reference in a real codebase disappears.
    let files = analyze(
        "same-package",
        &[
            ("a/Service.java", "package a;\npublic class Service {\n  public void run() {}\n}\n"),
            (
                "a/Caller.java",
                "package a;\npublic class Caller {\n  public void go() {\n    Service s = new Service();\n    s.run();\n  }\n}\n",
            ),
        ],
    );

    assert_eq!(
        targets(&files, "Caller.java", RelationshipKind::Calls),
        vec!["Service.java::Service::run::method"]
    );
}

#[test]
fn a_type_declared_in_the_file_shadows_an_import() {
    // The first rule of Java resolution. An adapter that checked imports first
    // would bind the local `User` to the imported one — a confidently wrong
    // answer, and one nothing downstream could detect.
    let files = analyze(
        "shadowing",
        &[
            ("m/User.java", "package m;\npublic class User {\n  public void ping() {}\n}\n"),
            (
                "a/Main.java",
                "package a;\nimport m.User;\nclass User {\n  public void ping() {}\n}\npublic class Main {\n  public void go() {\n    User u = new User();\n    u.ping();\n  }\n}\n",
            ),
        ],
    );

    let calls = targets(&files, "Main.java", RelationshipKind::Calls);
    assert_eq!(
        calls,
        vec!["Main.java::User::ping::method"],
        "the import won over the type declared in the file"
    );
}

#[test]
fn an_on_demand_import_resolves_when_only_one_package_offers_the_name() {
    let files = analyze(
        "wildcard",
        &[
            ("u/Formatter.java", "package u;\npublic class Formatter {\n  public void fmt() {}\n}\n"),
            (
                "a/Main.java",
                "package a;\nimport u.*;\npublic class Main {\n  public void go(Formatter f) {\n    f.fmt();\n  }\n}\n",
            ),
        ],
    );

    assert_eq!(
        targets(&files, "Main.java", RelationshipKind::Calls),
        vec!["Formatter.java::Formatter::fmt::method"]
    );
}

#[test]
fn two_on_demand_imports_offering_one_name_resolve_to_nothing() {
    // `javac` rejects this as ambiguous. Picking one would be a guess, and a
    // guess here attributes a change to the wrong class — so the edge is
    // dropped instead.
    let files = analyze(
        "ambiguous",
        &[
            ("x/Thing.java", "package x;\npublic class Thing {\n  public void go() {}\n}\n"),
            ("y/Thing.java", "package y;\npublic class Thing {\n  public void go() {}\n}\n"),
            (
                "a/Main.java",
                "package a;\nimport x.*;\nimport y.*;\npublic class Main {\n  public void run(Thing t) {\n    t.go();\n  }\n}\n",
            ),
        ],
    );

    assert!(
        targets(&files, "Main.java", RelationshipKind::Calls).is_empty(),
        "an ambiguous on-demand import was resolved to one of the candidates"
    );
    assert!(
        targets(&files, "Main.java", RelationshipKind::UsesType).is_empty(),
        "an ambiguous type reference was resolved"
    );
}

#[test]
fn a_static_import_binds_a_bare_call_to_another_type() {
    // `log(x)` names no receiver at all. Only the static import says where it
    // comes from.
    let files = analyze(
        "static-import",
        &[
            ("u/Helpers.java", "package u;\npublic class Helpers {\n  public static void log(String m) {}\n}\n"),
            (
                "a/Main.java",
                "package a;\nimport static u.Helpers.log;\npublic class Main {\n  public void go() {\n    log(\"x\");\n  }\n}\n",
            ),
        ],
    );

    assert_eq!(
        targets(&files, "Main.java", RelationshipKind::Calls),
        vec!["Helpers.java::Helpers::log::method"]
    );
}

#[test]
fn a_method_of_the_enclosing_type_wins_over_a_static_import() {
    // Java shadows the import. Resolving to the import would report a call
    // leaving the class when it never left.
    let files = analyze(
        "static-shadowed",
        &[
            ("u/Helpers.java", "package u;\npublic class Helpers {\n  public static void log(String m) {}\n}\n"),
            (
                "a/Main.java",
                "package a;\nimport static u.Helpers.log;\npublic class Main {\n  void log(String m) {}\n  public void go() {\n    log(\"x\");\n  }\n}\n",
            ),
        ],
    );

    assert_eq!(
        targets(&files, "Main.java", RelationshipKind::Calls),
        vec!["Main.java::Main::log::method"],
        "the statically imported method shadowed the one declared on the type"
    );
}

#[test]
fn an_inherited_method_resolves_to_the_type_that_declares_it() {
    // The supertype walk, across files. Without it the call is dropped and the
    // base class looks unused.
    let files = analyze(
        "inheritance",
        &[
            ("a/Base.java", "package a;\npublic class Base {\n  public void shared() {}\n}\n"),
            (
                "a/Child.java",
                "package a;\npublic class Child extends Base {\n  public void go() {\n    shared();\n  }\n}\n",
            ),
        ],
    );

    assert_eq!(
        targets(&files, "Child.java", RelationshipKind::Calls),
        vec!["Base.java::Base::shared::method"]
    );
}

#[test]
fn property_access_is_attributed_to_the_declared_type() {
    let files = analyze(
        "properties",
        &[
            ("m/User.java", "package m;\npublic class User {\n  public String email;\n  public String name;\n}\n"),
            (
                "a/Main.java",
                "package a;\nimport m.User;\npublic class Main {\n  public String go(User u) {\n    String e = u.email;\n    return u.name;\n  }\n}\n",
            ),
        ],
    );

    let access: Vec<_> = files
        .iter()
        .filter(|f| f.file.ends_with("Main.java"))
        .flat_map(|f| f.relationships.iter())
        .filter(|r| r.kind == RelationshipKind::AccessesProperty)
        .collect();

    assert_eq!(access.len(), 1, "expected one grouped edge: {access:?}");
    assert_eq!(access[0].properties_accessed, vec!["email", "name"]);
    assert!(
        access[0].to.ends_with("User::class"),
        "attributed to {} rather than the declared type",
        access[0].to
    );
}

#[test]
fn a_type_outside_the_repository_records_no_edge() {
    // `String` is not in the repository, so any edge to it would be invented.
    // Dispatch stamps every Tier 1 edge gate-safe, so an invented one would be
    // marked as evidence a gate may act on.
    let files = analyze(
        "stdlib",
        &[(
            "a/Main.java",
            "package a;\npublic class Main {\n  public String go() {\n    String s = \"x\";\n    return s;\n  }\n}\n",
        )],
    );

    for file in &files {
        for rel in &file.relationships {
            assert!(
                !rel.to.contains("String"),
                "an edge to the standard library was recorded: {} -> {}",
                rel.from,
                rel.to
            );
        }
    }
}

#[test]
fn no_placeholder_survives_resolution() {
    // A placeholder reaching the graph is a silently missing edge, and one
    // reaching a snapshot is a wrong id nothing will ever match.
    let files = analyze(
        "placeholders",
        &[
            ("m/User.java", "package m;\npublic class User {}\n"),
            (
                "a/Main.java",
                "package a;\nimport m.User;\nimport java.util.List;\npublic class Main {\n  User u;\n  List<String> items;\n  public void go(Unknown x) {\n    x.whatever();\n  }\n}\n",
            ),
        ],
    );

    for file in &files {
        for rel in &file.relationships {
            assert!(
                !openinvar_core::symbol_id::is_placeholder(&rel.to),
                "unresolved placeholder survived: {} -> {}",
                rel.from,
                rel.to
            );
        }
    }
}

#[test]
fn an_import_from_outside_the_repository_becomes_an_external_package() {
    let files = analyze(
        "external",
        &[(
            "a/Main.java",
            "package a;\nimport java.util.List;\npublic class Main {}\n",
        )],
    );

    let imports: Vec<&str> = files
        .iter()
        .flat_map(|f| f.relationships.iter())
        .filter(|r| r.kind == RelationshipKind::Imports)
        .map(|r| r.to.as_str())
        .collect();

    assert_eq!(imports.len(), 1, "{imports:?}");
    assert!(
        openinvar_core::symbol_id::is_external_package(imports[0]),
        "a jar or stdlib import became {} rather than an external package",
        imports[0]
    );
}

#[test]
fn resolution_is_the_same_on_every_run() {
    // Determinism is the property everything else rests on, and this adapter
    // resolves through several maps that would otherwise be free to vary.
    let sources: &[(&str, &str)] = &[
        ("u/A.java", "package u;\npublic class A {\n  public void go() {}\n}\n"),
        ("u/B.java", "package u;\npublic class B {\n  public void go() {}\n}\n"),
        (
            "a/Main.java",
            "package a;\nimport u.*;\npublic class Main {\n  public void run(A a, B b) {\n    a.go();\n    b.go();\n  }\n}\n",
        ),
    ];

    let first = analyze("determinism-0", sources);
    let render = |files: &[FileIR]| {
        let mut lines: Vec<String> = files
            .iter()
            .flat_map(|f| f.relationships.iter())
            .map(|r| format!("{:?} {} -> {}", r.kind, r.from, r.to))
            .collect();
        lines.sort();
        lines
    };
    let expected = render(&first);

    for run in 1..5 {
        let again = analyze(&format!("determinism-{run}"), sources);
        assert_eq!(render(&again), expected, "run {run} disagreed");
    }
}

/// Paths are only used to place the scratch files; nothing here depends on the
/// directory layout matching the package, which Java does not require either.
#[allow(dead_code)]
fn unused() -> PathBuf {
    PathBuf::new()
}
