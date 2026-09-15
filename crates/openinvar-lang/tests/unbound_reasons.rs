//! Why a reference did not bind, not just how many did not.
//!
//! `unbound_references` counts references an adapter attempted to place and
//! could not. On its own it is an upper bound on what is missing rather than a
//! count of it: `println!` and a local function the resolver missed land in the
//! same total, and every call resolver said so in its own comments — "this arm
//! cannot tell a prelude name from a local function the resolver missed".
//!
//! `unbound_outside_repository` is the part an adapter can vouch for. These
//! pin both directions, because the failure modes are opposite and a test for
//! one passes happily while the other is broken:
//!
//! - classify too little and the figure says work is missing that never was;
//! - classify too much and a real gap disappears into "not our problem", which
//!   is the direction that quietly makes the number look good.

#![cfg(any(feature = "rust", feature = "python", feature = "go"))]

use std::path::PathBuf;

use openinvar_core::ir::RepoIR;

fn analyze(name: &str, files: &[(&str, &str)]) -> RepoIR {
    let root = std::env::temp_dir().join(format!(
        "openinvar-unbound-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let mut paths: Vec<PathBuf> = Vec::new();
    for (path, source) in files {
        let full = root.join(path);
        std::fs::create_dir_all(full.parent().expect("parent")).expect("mkdir");
        std::fs::write(&full, source).expect("write");
        paths.push(full);
    }
    paths.sort();
    let ir = openinvar_lang::dispatch::analyze_files(&root, &paths).expect("analysis");
    let _ = std::fs::remove_dir_all(&root);
    ir
}

/// Total unbound, and the share the adapter placed outside the repository.
fn split(ir: &RepoIR) -> (u32, u32) {
    let total: u32 = ir.files.iter().map(|f| f.unbound_references).sum();
    let outside: u32 = ir.files.iter().map(|f| f.unbound_outside_repository).sum();
    (total, outside)
}

#[cfg(feature = "rust")]
#[test]
fn a_prelude_call_is_attributed_outside_the_repository() {
    // `println!` and `Some` are the language's own vocabulary. They are unbound
    // — there is no symbol for them — but no amount of work on the adapter
    // would ever bind them, so they are not a gap in the graph.
    let ir = analyze(
        "rust-prelude",
        &[(
            "src/lib.rs",
            "pub fn greet() {\n\
             \x20   println!(\"hello\");\n\
             \x20   let _ = Some(1);\n\
             }\n",
        )],
    );

    let (total, outside) = split(&ir);
    assert!(total > 0, "nothing was counted as unbound at all");
    assert_eq!(
        outside, total,
        "a file whose only unbound references are prelude names left {} unexplained",
        total - outside
    );
}

#[cfg(feature = "rust")]
#[test]
fn a_call_to_a_function_that_does_not_exist_stays_unexplained() {
    // The counterweight, and the direction that matters: this is a reference
    // the graph genuinely does not describe. If classification swallowed it,
    // the figure would improve while coverage got no better.
    let ir = analyze(
        "rust-missing",
        &[(
            "src/lib.rs",
            "pub fn greet() {\n\
             \x20   no_such_function_anywhere(1);\n\
             }\n",
        )],
    );

    let (total, outside) = split(&ir);
    assert!(total > 0, "a call to a name declared nowhere was not counted");
    assert_eq!(
        outside, 0,
        "a name declared nowhere was attributed outside the repository"
    );
}

#[cfg(feature = "rust")]
#[test]
fn the_classified_share_never_exceeds_the_total() {
    // The two counts are stored separately, so nothing in the type system stops
    // them disagreeing. A file mixing both kinds is where drift would show.
    let ir = analyze(
        "rust-mixed",
        &[(
            "src/lib.rs",
            "pub fn greet() {\n\
             \x20   println!(\"hello\");\n\
             \x20   let _ = Some(1);\n\
             \x20   missing_one(1);\n\
             \x20   missing_two(2);\n\
             }\n",
        )],
    );

    for file in &ir.files {
        assert!(
            file.unbound_outside_repository <= file.unbound_references,
            "{}: classified {} of {} unbound references",
            file.file,
            file.unbound_outside_repository,
            file.unbound_references
        );
    }
    let (total, outside) = split(&ir);
    assert!(outside > 0 && outside < total, "expected a mix, got {outside} of {total}");
}

#[cfg(feature = "python")]
#[test]
fn python_builtins_are_attributed_outside_the_repository() {
    // Same property, a second adapter: the mechanism is per-adapter, so one
    // language passing says nothing about another.
    let ir = analyze(
        "py-builtin",
        &[("mod.py", "def greet():\n    print(len([1, 2, 3]))\n")],
    );

    let (total, outside) = split(&ir);
    assert!(total > 0, "nothing was counted as unbound at all");
    assert_eq!(
        outside, total,
        "a file whose only unbound references are builtins left {} unexplained",
        total - outside
    );
}

#[cfg(feature = "python")]
#[test]
fn a_python_call_to_a_missing_name_stays_unexplained() {
    let ir = analyze(
        "py-missing",
        &[("mod.py", "def greet():\n    no_such_function_anywhere(1)\n")],
    );

    let (_, outside) = split(&ir);
    assert_eq!(
        outside, 0,
        "a name declared nowhere was attributed outside the repository"
    );
}

#[cfg(feature = "go")]
#[test]
fn go_builtins_are_attributed_outside_the_repository() {
    let ir = analyze(
        "go-builtin",
        &[(
            "main.go",
            "package main\n\nfunc greet() int {\n\treturn len([]int{1, 2})\n}\n",
        )],
    );

    let (total, outside) = split(&ir);
    assert!(total > 0, "nothing was counted as unbound at all");
    assert_eq!(outside, total, "a builtin call was left unexplained");
}
