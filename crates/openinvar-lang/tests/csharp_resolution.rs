//! C# name resolution, which is what the Tier 1 promotion is.
//!
//! Three of these cover things Java does not do at all: a `using` that names a
//! namespace rather than a type, an enclosing namespace that is in scope with
//! no `using` at all, and a type declared across two files.
//!
//! As with the Java suite, every test is a case where getting the rule wrong
//! still produces *an* answer — just the wrong one, silently.

#![cfg(feature = "csharp")]

use openinvar_core::ir::{FileIR, RelationshipKind};

fn analyze(name: &str, files: &[(&str, &str)]) -> Vec<FileIR> {
    let root = std::env::temp_dir().join(format!(
        "openinvar-cs-{name}-{}-{}",
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

    let ir = openinvar_lang::lang::csharp::analyze_files(&root, &paths).expect("analysis");
    let _ = std::fs::remove_dir_all(&root);
    ir.files
}

fn targets(files: &[FileIR], file: &str, kind: RelationshipKind) -> Vec<String> {
    let mut out: Vec<String> = files
        .iter()
        .filter(|f| f.file.ends_with(file))
        .flat_map(|f| f.relationships.iter())
        .filter(|r| r.kind == kind)
        .map(|r| {
            let (path, rest) = r.to.split_once("::").unwrap_or(("", &r.to));
            let stem = path.rsplit('/').next().unwrap_or(path);
            format!("{stem}::{rest}")
        })
        .collect();
    out.sort();
    out
}

#[test]
fn a_type_in_the_same_namespace_resolves_without_a_using() {
    let files = analyze(
        "same-namespace",
        &[
            (
                "a/Service.cs",
                "namespace A { public class Service { public void Run() {} } }",
            ),
            (
                "a/Caller.cs",
                "namespace A { public class Caller { public void Go() { Service s = new Service(); s.Run(); } } }",
            ),
        ],
    );

    assert_eq!(
        targets(&files, "Caller.cs", RelationshipKind::Calls),
        vec!["Service.cs::Service::Run::method"]
    );
}

#[test]
fn an_enclosing_namespace_is_in_scope() {
    // C# specific, and it has no Java equivalent: inside `A.B`, a type declared
    // in `A` is visible with no using at all. A resolver that only searched the
    // exact namespace would drop most references in a namespaced project.
    let files = analyze(
        "enclosing",
        &[
            (
                "a/Shared.cs",
                "namespace A { public class Shared { public void Ping() {} } }",
            ),
            (
                "a/b/Inner.cs",
                "namespace A.B { public class Inner { public void Go() { Shared s = new Shared(); s.Ping(); } } }",
            ),
        ],
    );

    assert_eq!(
        targets(&files, "Inner.cs", RelationshipKind::Calls),
        vec!["Shared.cs::Shared::Ping::method"],
        "a type in the enclosing namespace was not found"
    );
}

#[test]
fn a_using_brings_in_a_whole_namespace() {
    let files = analyze(
        "using",
        &[
            (
                "m/User.cs",
                "namespace M { public class User { public void Ping() {} } }",
            ),
            (
                "a/Main.cs",
                "using M;\nnamespace A { public class Main { public void Go(User u) { u.Ping(); } } }",
            ),
        ],
    );

    assert_eq!(
        targets(&files, "Main.cs", RelationshipKind::Calls),
        vec!["User.cs::User::Ping::method"]
    );
}

#[test]
fn two_usings_offering_one_name_resolve_to_nothing() {
    // The compiler rejects this as ambiguous. C# has no single-type import, so
    // every using is the on-demand form and this is the ordinary case rather
    // than a corner — which is why refusing to guess matters more here.
    let files = analyze(
        "ambiguous",
        &[
            ("x/Thing.cs", "namespace X { public class Thing { public void Go() {} } }"),
            ("y/Thing.cs", "namespace Y { public class Thing { public void Go() {} } }"),
            (
                "a/Main.cs",
                "using X;\nusing Y;\nnamespace A { public class Main { public void Run(Thing t) { t.Go(); } } }",
            ),
        ],
    );

    assert!(
        targets(&files, "Main.cs", RelationshipKind::Calls).is_empty(),
        "an ambiguous using was resolved to one of the candidates"
    );
    assert!(
        targets(&files, "Main.cs", RelationshipKind::UsesType).is_empty(),
        "an ambiguous type reference was resolved"
    );
}

#[test]
fn an_alias_binds_one_name_to_one_type() {
    let files = analyze(
        "alias",
        &[
            (
                "u/Formatter.cs",
                "namespace U { public class Formatter { public void Fmt() {} } }",
            ),
            (
                "a/Main.cs",
                "using Fmt = U.Formatter;\nnamespace A { public class Main { public void Go(Fmt f) { f.Fmt(); } } }",
            ),
        ],
    );

    assert_eq!(
        targets(&files, "Main.cs", RelationshipKind::Calls),
        vec!["Formatter.cs::Formatter::Fmt::method"]
    );
}

#[test]
fn a_static_using_binds_a_bare_call() {
    let files = analyze(
        "static-using",
        &[
            (
                "u/Helpers.cs",
                "namespace U { public class Helpers { public static void Log(string m) {} } }",
            ),
            (
                "a/Main.cs",
                "using static U.Helpers;\nnamespace A { public class Main { public void Go() { Log(\"x\"); } } }",
            ),
        ],
    );

    assert_eq!(
        targets(&files, "Main.cs", RelationshipKind::Calls),
        vec!["Helpers.cs::Helpers::Log::method"]
    );
}

#[test]
fn a_method_of_the_enclosing_type_wins_over_a_static_using() {
    // Both could answer a bare `Log()`. C# gives the enclosing type's own
    // member priority, and a resolver that checked imports first would return
    // the wrong one of two real symbols — the failure mode that shows up as a
    // blast radius pointing at the wrong file.
    let files = analyze(
        "shadowing",
        &[
            (
                "u/Helpers.cs",
                "namespace U { public class Helpers { public static void Log(string m) {} } }",
            ),
            (
                "a/Main.cs",
                "using static U.Helpers;\nnamespace A { public class Main { public void Log(string m) {} public void Go() { Log(\"x\"); } } }",
            ),
        ],
    );

    assert_eq!(
        targets(&files, "Main.cs", RelationshipKind::Calls),
        vec!["Main.cs::Main::Log::method"],
        "the using static won over the type's own method"
    );
}

#[test]
fn an_inherited_method_resolves_to_the_type_that_declares_it() {
    // The call names the subclass; the method lives on the base. Attributing it
    // to the subclass would be a symbol that does not exist.
    let files = analyze(
        "inheritance",
        &[
            (
                "a/Base.cs",
                "namespace A { public class Base { public void Shared() {} } }",
            ),
            (
                "a/Derived.cs",
                "namespace A { public class Derived : Base {} }",
            ),
            (
                "a/Main.cs",
                "namespace A { public class Main { public void Go(Derived d) { d.Shared(); } } }",
            ),
        ],
    );

    assert_eq!(
        targets(&files, "Main.cs", RelationshipKind::Calls),
        vec!["Base.cs::Base::Shared::method"]
    );
}

#[test]
fn a_partial_class_is_one_type_across_files() {
    // The C# case with no analogue anywhere else in this project: two files
    // declare one type, and each calls a method the other one declares.
    //
    // Both directions are checked deliberately. An index keyed by declaration
    // rather than by the owner's name still answers one of them — whichever
    // half it happened to pick as the type — so a test that only looked one way
    // would pass without the halves ever being merged.
    let files = analyze(
        "partial",
        &[
            (
                "a/Report.cs",
                "namespace A { public partial class Report { public string Render() { return Summarise(); } public string Title() { return \"t\"; } } }",
            ),
            (
                "a/Report.Extra.cs",
                "namespace A { public partial class Report { public string Summarise() { return Title(); } } }",
            ),
        ],
    );

    assert_eq!(
        targets(&files, "Report.cs", RelationshipKind::Calls),
        vec!["Report.Extra.cs::Report::Summarise::method"],
        "a call into the other half of the partial class did not resolve"
    );
    assert_eq!(
        targets(&files, "Report.Extra.cs", RelationshipKind::Calls),
        vec!["Report.cs::Report::Title::method"],
        "the other half could not call back"
    );
}

#[test]
fn var_with_a_constructor_binds_but_var_with_a_call_does_not() {
    // `var x = new Foo()` states its type; reading it is not inference.
    // `var x = Make()` does not, and working it out would mean inferring a
    // return type — a second type system beside the real one.
    let files = analyze(
        "var",
        &[
            (
                "a/Thing.cs",
                "namespace A { public class Thing { public void Go() {} public static Thing Make() { return null; } } }",
            ),
            (
                "a/Main.cs",
                "namespace A { public class Main { public void Direct() { var t = new Thing(); t.Go(); } } }",
            ),
            (
                "a/Indirect.cs",
                "namespace A { public class Indirect { public void Run() { var t = Thing.Make(); t.Go(); } } }",
            ),
        ],
    );

    assert!(
        targets(&files, "Main.cs", RelationshipKind::Calls)
            .contains(&"Thing.cs::Thing::Go::method".to_string()),
        "var with a constructor did not bind"
    );
    assert!(
        !targets(&files, "Indirect.cs", RelationshipKind::Calls)
            .contains(&"Thing.cs::Thing::Go::method".to_string()),
        "a return type was inferred, which this deliberately does not do"
    );
}

#[test]
fn property_access_is_attributed_to_the_declared_type() {
    let files = analyze(
        "properties",
        &[
            (
                "m/User.cs",
                "namespace M { public class User { public string Email { get; set; } public string Name { get; set; } } }",
            ),
            (
                "a/Main.cs",
                "using M;\nnamespace A { public class Main { public string Go(User u) { string e = u.Email; return u.Name; } } }",
            ),
        ],
    );

    let access: Vec<_> = files
        .iter()
        .filter(|f| f.file.ends_with("Main.cs"))
        .flat_map(|f| f.relationships.iter())
        .filter(|r| r.kind == RelationshipKind::AccessesProperty)
        .collect();

    assert_eq!(access.len(), 1, "expected one grouped edge: {access:?}");
    assert_eq!(access[0].properties_accessed, vec!["Email", "Name"]);
    assert!(access[0].to.ends_with("User::class"), "{}", access[0].to);
}

#[test]
fn a_using_of_a_namespace_outside_the_repository_is_external() {
    let files = analyze(
        "external",
        &[(
            "a/Main.cs",
            "using System.Collections.Generic;\nnamespace A { public class Main {} }",
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
        "{} is not an external package",
        imports[0]
    );
}

#[test]
fn a_using_of_the_repositorys_own_namespace_is_not_an_external_dependency() {
    // It would put the repository's own code in the third-party list. The
    // concrete edges to the types it brought in carry the real information.
    let files = analyze(
        "internal-using",
        &[
            ("m/User.cs", "namespace M { public class User {} }"),
            (
                "a/Main.cs",
                "using M;\nnamespace A { public class Main { private User u; } }",
            ),
        ],
    );

    let imports: Vec<&str> = files
        .iter()
        .flat_map(|f| f.relationships.iter())
        .filter(|r| r.kind == RelationshipKind::Imports)
        .map(|r| r.to.as_str())
        .collect();

    assert!(
        imports.is_empty(),
        "the repository's own namespace was recorded as external: {imports:?}"
    );
    assert_eq!(
        targets(&files, "Main.cs", RelationshipKind::UsesType),
        vec!["User.cs::User::class"],
        "the type the using brought in still has to resolve"
    );
}

#[test]
fn a_type_outside_the_repository_records_no_edge() {
    let files = analyze(
        "stdlib",
        &[(
            "a/Main.cs",
            "namespace A { public class Main { public string Go() { string s = \"x\"; return s; } } }",
        )],
    );

    for file in &files {
        for rel in &file.relationships {
            assert!(
                !rel.to.contains("string") && !rel.to.contains("String"),
                "an edge to a predefined type was recorded: {} -> {}",
                rel.from,
                rel.to
            );
        }
    }
}

#[test]
fn no_placeholder_survives_resolution() {
    let files = analyze(
        "placeholders",
        &[
            ("m/User.cs", "namespace M { public class User {} }"),
            (
                "a/Main.cs",
                "using M;\nusing System;\nnamespace A { public class Main { User u; public void Go(Unknown x) { x.Whatever(); } } }",
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
fn resolution_is_the_same_on_every_run() {
    let sources: &[(&str, &str)] = &[
        ("u/A.cs", "namespace U { public class A { public void Go() {} } }"),
        ("u/B.cs", "namespace U { public class B { public void Go() {} } }"),
        (
            "a/Main.cs",
            "using U;\nnamespace A { public class Main { public void Run(A a, B b) { a.Go(); b.Go(); } } }",
        ),
    ];

    let render = |files: &[FileIR]| {
        let mut lines: Vec<String> = files
            .iter()
            .flat_map(|f| f.relationships.iter())
            .map(|r| format!("{:?} {} -> {}", r.kind, r.from, r.to))
            .collect();
        lines.sort();
        lines
    };
    let expected = render(&analyze("determinism-0", sources));
    for run in 1..5 {
        assert_eq!(
            render(&analyze(&format!("determinism-{run}"), sources)),
            expected,
            "run {run} disagreed"
        );
    }
}
