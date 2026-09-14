//! `.openinvar/rules.toml` parsing and validation.
//!
//! Most of these assert that a bad rules file is *rejected*, because the
//! alternative failure is the dangerous one: a gate that quietly enforces four
//! of your five rules reports a pass it has not earned.

use openinvar_core::rules::{self, RuleError, RuleKind, Severity};

#[test]
fn a_full_rules_file_parses() {
    let parsed = rules::parse(
        r#"
[[rule]]
name = "core-must-not-depend-on-cli"
kind = "forbid-dependency"
from = "crates/openinvar-core/**"
to   = "crates/openinvar-cli/**"
severity = "error"

[[rule]]
name = "internal-not-public"
kind = "forbid-reference"
from = "src/public/**"
to   = "src/internal/**"

[[rule]]
name = "frozen-contract"
kind = "no-field-removal"
symbol = "UserPayload"

[[rule]]
name = "god-node"
kind = "max-fan-in"
threshold = 60
severity = "warn"
"#,
    )
    .expect("this file is well formed");

    assert_eq!(parsed.rules.len(), 4);
    assert_eq!(parsed.rules[0].severity, Severity::Error);
    assert_eq!(parsed.rules[3].severity, Severity::Warn);
    assert!(matches!(
        parsed.rules[3].kind,
        RuleKind::MaxFanIn { threshold: 60 }
    ));
}

#[test]
fn severity_defaults_to_error_for_a_hard_invariant() {
    // A boundary written without a severity is one someone means to enforce.
    // Defaulting the whole vocabulary to warn would make every unannotated
    // rule advisory, which is the opposite of what writing it down expresses.
    // The fan rules are the deliberate exception; see below.
    let parsed = rules::parse(
        r#"
[[rule]]
name = "r"
kind = "no-field-removal"
symbol = "X"
"#,
    )
    .expect("parses");
    assert_eq!(parsed.rules[0].severity, Severity::Error);
}

#[test]
fn an_unknown_kind_is_rejected_and_lists_what_is_valid() {
    let err = rules::parse(
        r#"
[[rule]]
name = "typo"
kind = "forbid-dependancy"
from = "a/**"
to = "b/**"
"#,
    )
    .expect_err("an unknown kind must not parse");

    match err {
        RuleError::Invalid { rule, reason } => {
            assert_eq!(rule, "typo");
            assert!(reason.contains("forbid-dependency"), "{reason}");
        }
        other => panic!("wrong error: {other:?}"),
    }
}

#[test]
fn a_malformed_glob_is_rejected_at_parse_time() {
    // Not when the rule is reached. An unparseable pattern that silently
    // matched nothing would be a rule that passes forever.
    let err = rules::parse(
        r#"
[[rule]]
name = "bad-glob"
kind = "forbid-dependency"
from = "src/["
to = "b/**"
"#,
    )
    .expect_err("a malformed glob must not parse");

    assert!(
        matches!(err, RuleError::Invalid { ref reason, .. } if reason.contains("glob")),
        "{err:?}"
    );
}

#[test]
fn a_kind_missing_a_field_it_needs_is_rejected() {
    let err = rules::parse(
        r#"
[[rule]]
name = "half-written"
kind = "forbid-dependency"
from = "a/**"
"#,
    )
    .expect_err("a missing 'to' must not parse");

    assert!(
        matches!(err, RuleError::Invalid { ref reason, .. } if reason.contains("'to'")),
        "{err:?}"
    );
}

#[test]
fn a_zero_threshold_is_rejected() {
    // Zero forbids every inbound edge in the repository, which is far more
    // likely to be a mistake than an intention.
    let err = rules::parse(
        r#"
[[rule]]
name = "nonsense"
kind = "max-fan-in"
threshold = 0
"#,
    )
    .expect_err("a zero threshold must not parse");

    assert!(
        matches!(err, RuleError::Invalid { ref reason, .. } if reason.contains("at least 1")),
        "{err:?}"
    );
}

#[test]
fn a_negative_threshold_is_rejected() {
    let err = rules::parse(
        r#"
[[rule]]
name = "nonsense"
kind = "max-fan-in"
threshold = -5
"#,
    )
    .expect_err("a negative threshold must not parse");
    assert!(matches!(err, RuleError::Invalid { .. }), "{err:?}");
}

#[test]
fn two_rules_with_one_name_are_rejected() {
    // A violation under a duplicated name cannot be attributed, and a
    // suppression for it cannot be written.
    let err = rules::parse(
        r#"
[[rule]]
name = "same"
kind = "max-fan-in"
threshold = 10

[[rule]]
name = "same"
kind = "max-fan-in"
threshold = 20
"#,
    )
    .expect_err("duplicate names must not parse");

    assert!(
        matches!(err, RuleError::Invalid { ref reason, .. } if reason.contains("already defined")),
        "{err:?}"
    );
}

#[test]
fn malformed_toml_is_reported_as_syntax_rather_than_as_a_rule_problem() {
    let err = rules::parse("[[rule]\nname = \"x\"").expect_err("must not parse");
    assert!(matches!(err, RuleError::Syntax(_)), "{err:?}");
}

#[test]
fn an_empty_file_is_valid_and_has_no_rules() {
    // A repository that has started a rules file but written nothing in it is
    // not an error; `check` on it simply passes.
    let parsed = rules::parse("").expect("an empty file is valid");
    assert!(parsed.rules.is_empty());
}

#[test]
fn rules_are_partitioned_by_whether_they_need_a_delta() {
    // A caller with only one graph must skip the delta rules rather than
    // report them as passing.
    let parsed = rules::parse(
        r#"
[[rule]]
name = "state"
kind = "max-fan-in"
threshold = 10

[[rule]]
name = "change"
kind = "no-field-removal"
symbol = "X"
"#,
    )
    .expect("parses");

    let stateless: Vec<&str> = parsed.without_delta().map(|r| r.name.as_str()).collect();
    let delta: Vec<&str> = parsed.needing_delta().map(|r| r.name.as_str()).collect();

    assert_eq!(stateless, vec!["state"]);
    assert_eq!(delta, vec!["change"]);
}

#[test]
fn an_unnamed_rule_still_gets_a_usable_identity() {
    // So an error about it can be attributed to a place in the file.
    let err = rules::parse(
        r#"
[[rule]]
kind = "max-fan-in"
"#,
    )
    .expect_err("missing threshold");

    assert!(
        matches!(err, RuleError::Invalid { ref rule, .. } if rule.contains('1')),
        "{err:?}"
    );
}

// ── layers and independence ──────────────────────────────────

#[test]
fn a_layer_stack_needs_at_least_two_layers() {
    // One layer has nothing to be above. A rule that cannot be violated by
    // construction is one somebody believes is protecting them.
    let err = rules::parse(
        r#"
[[rule]]
name = "pointless"
kind = "layers"
layers = ["src/**"]
"#,
    )
    .expect_err("a single layer cannot mean anything");
    assert!(err.to_string().contains("at least 2"), "{err}");
}

#[test]
fn a_layer_listed_twice_is_rejected() {
    // Either a copy-paste slip or a belief that one scope sits at two depths.
    // Neither should evaluate.
    let err = rules::parse(
        r#"
[[rule]]
name = "duplicated"
kind = "layers"
layers = ["src/a/**", "src/b/**", "src/a/**"]
"#,
    )
    .expect_err("a repeated layer cannot be ordered against itself");
    assert!(err.to_string().contains("twice"), "{err}");
}

#[test]
fn an_independence_set_needs_at_least_two_modules() {
    let err = rules::parse(
        r#"
[[rule]]
name = "alone"
kind = "independence"
modules = ["src/billing/**"]
"#,
    )
    .expect_err("one module has nobody to stay independent from");
    assert!(err.to_string().contains("at least 2"), "{err}");
}

#[test]
fn a_layer_glob_that_does_not_compile_is_rejected_at_parse_time() {
    let err = rules::parse(
        r#"
[[rule]]
name = "broken-glob"
kind = "layers"
layers = ["src/**", "src/[unclosed"]
"#,
    )
    .expect_err("a malformed glob must fail when the file is read");
    assert!(err.to_string().contains("glob"), "{err}");
}

#[test]
fn layers_and_independence_require_their_own_field() {
    for (kind, field) in [("layers", "layers"), ("independence", "modules")] {
        let err = rules::parse(&format!(
            r#"
[[rule]]
name = "missing"
kind = "{kind}"
"#
        ))
        .expect_err("the list is required");
        assert!(err.to_string().contains(field), "{err}");
    }
}

// ── kind-aware severity defaults ─────────────────────────────

#[test]
fn the_fan_rules_default_to_warn_rather_than_error() {
    // High fan-in and fan-out are frequently intentional: a utility module, an
    // IR type, an entry point. Defaulting these to error would block a
    // developer who added a module before wiring up its callers, and the rule
    // would be switched off rather than tuned.
    for kind in ["max-fan-in", "max-fan-out"] {
        let parsed = rules::parse(&format!(
            r#"
[[rule]]
name = "hygiene"
kind = "{kind}"
threshold = 40
"#
        ))
        .expect("parses");
        assert_eq!(
            parsed.rules[0].severity,
            Severity::Warn,
            "{kind} is hygiene, not an invariant"
        );
    }
}

#[test]
fn every_boundary_kind_defaults_to_error() {
    let cases = [
        r#"kind = "forbid-dependency"
from = "a/**"
to = "b/**""#,
        r#"kind = "forbid-reference"
from = "a/**"
to = "b/**""#,
        r#"kind = "no-field-removal"
symbol = "X""#,
        r#"kind = "layers"
layers = ["a/**", "b/**"]"#,
        r#"kind = "independence"
modules = ["a/**", "b/**"]"#,
        r#"kind = "naming-convention"
symbols = "a/**"
matches = "*Handler""#,
    ];
    for body in cases {
        let parsed = rules::parse(&format!("[[rule]]\nname = \"r\"\n{body}\n")).expect("parses");
        assert_eq!(
            parsed.rules[0].severity,
            Severity::Error,
            "a stated boundary must block by default: {body}"
        );
    }
}

#[test]
fn an_explicit_severity_still_wins_over_the_kind_default() {
    let parsed = rules::parse(
        r#"
[[rule]]
name = "zero-tolerance"
kind = "max-fan-in"
threshold = 40
severity = "error"
"#,
    )
    .expect("parses");
    assert_eq!(parsed.rules[0].severity, Severity::Error);
}

#[test]
fn an_unknown_symbol_kind_in_only_is_rejected() {
    let err = rules::parse(
        r#"
[[rule]]
name = "naming"
kind = "naming-convention"
symbols = "src/**"
matches = "*Handler"
only = "struct"
"#,
    )
    .expect_err("'struct' is not one of this graph's symbol kinds");
    let text = err.to_string();
    assert!(text.contains("struct"), "{text}");
    assert!(text.contains("class"), "the error must list what is valid: {text}");
}

#[test]
fn a_zero_threshold_is_rejected_for_both_fan_rules() {
    for kind in ["max-fan-in", "max-fan-out"] {
        let err = rules::parse(&format!(
            r#"
[[rule]]
name = "r"
kind = "{kind}"
threshold = 0
"#
        ))
        .expect_err("zero forbids every edge, which is never the intent");
        assert!(err.to_string().contains("at least 1"), "{err}");
    }
}
