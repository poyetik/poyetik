//! Tests of the gates over reviewed exceptions: mutation reports, Stryker, proof bypasses, scopes
//! and unviable mutants.

use crate::tests::{
    CHANGES_MODE, DISABLED_SOURCE, MEMBER, STRYKER_CONFIG, TS_KILLED_OR_DISABLED, failed, found,
    one_mutant, passing,
};
use crate::{
    Action, EQUIVALENT_REVIEWED, Extent, Failure, Step, UNVIABLE_REVIEWED, check_steps,
    equivalent_gate, membership_gate, package_jsons, run_steps, ts_mutation_gate, unviable_gate,
};

#[test]
fn a_partial_run_finds_no_stale_review_but_every_unreviewed_mutant() {
    let report = "crates/a/src/b.rs:3:5: replace f -> T with y\n";
    let reviewed = "crates/a/src/z.rs: replace z -> Z with w -- elsewhere\n";
    assert_eq!(
        unviable_gate("", Extent::Partial, reviewed),
        Ok(()),
        "a review of a mutant the run did not generate is not stale"
    );
    assert_eq!(
        unviable_gate("", Extent::Complete, reviewed),
        found(
            &format!("unviable mutants must be reviewed in {UNVIABLE_REVIEWED}"),
            &["stale review: crates/a/src/z.rs: replace z -> Z with w".to_owned()]
        ),
        "after a complete run it is"
    );
    assert_eq!(
        unviable_gate(report, Extent::Partial, &format!("{reviewed}no reason\n")),
        found(
            &format!("unviable mutants must be reviewed in {UNVIABLE_REVIEWED}"),
            &[
                "review without a reason: no reason".to_owned(),
                "not reviewed: crates/a/src/b.rs: replace f -> T with y".to_owned()
            ]
        ),
        "an unreviewed mutant and a malformed review are still refused"
    );
    let twice = "crates/a/src/b.rs:3:5: replace f -> T with y\ncrates/a/src/b.rs:9:5: replace f -> T with y\n";
    assert_eq!(
        unviable_gate(
            twice,
            Extent::Partial,
            "crates/a/src/b.rs: replace f -> T with y -- once\n"
        ),
        found(
            &format!("unviable mutants must be reviewed in {UNVIABLE_REVIEWED}"),
            &[
                "not reviewed: crates/a/src/b.rs: replace f -> T with y (2 found, 1 reviewed)"
                    .to_owned()
            ]
        ),
        "each occurrence still needs its review"
    );
}

#[test]
fn refuses_reviews_of_mutants_that_no_longer_exist() {
    let gate = crate::unviable_reviews_gate;
    let context = "every review of mutants-unviable.txt must name a mutant that exists";
    let listed = "crates/a/src/b.rs:3:5: replace f -> T with Default::default()\n\
                  crates/a/src/b.rs:9:5: replace += with *= in g\n\
                  crates/a/src/b.rs:12:5: replace += with *= in g\n\n";
    let reviewed = "# a comment\n\ncrates/a/src/b.rs: replace += with *= in g -- one\n\
                    crates/a/src/b.rs: replace += with *= in g -- two\n";
    assert_eq!(
        gate(listed, reviewed),
        Ok(()),
        "every review names a mutant, as often as it exists; unreviewed mutants are another gate's"
    );
    assert_eq!(gate(listed, ""), Ok(()), "no review at all");
    assert_eq!(
        gate(
            listed,
            &format!("{reviewed}crates/a/src/b.rs: replace += with *= in g -- three\n")
        ),
        found(
            context,
            &[
                "stale review: crates/a/src/b.rs: replace += with *= in g (2 found, 3 reviewed)"
                    .to_owned()
            ]
        ),
        "reviewed more often than it exists"
    );
    assert_eq!(
        gate(
            listed,
            "crates/a/src/b.rs: replace += with -= in g -- gone\nno reason here\n"
        ),
        found(
            context,
            &[
                "review without a reason: no reason here".to_owned(),
                "stale review: crates/a/src/b.rs: replace += with -= in g".to_owned(),
            ]
        ),
        "a mutant that no longer exists, and a malformed review"
    );
    assert_eq!(
        gate("not a mutant\n", ""),
        found(context, &["unrecognized mutant: not a mutant".to_owned()]),
        "a listed line that is not a mutant is refused, never skipped"
    );
}

#[test]
fn the_default_check_refuses_stale_reviews_of_unviable_mutants() {
    let steps = check_steps(CHANGES_MODE);
    let mut fake = passing();
    let _stale = fake.files.insert(
        UNVIABLE_REVIEWED.to_owned(),
        "crates/a/src/b.rs: replace f -> T with Default::default() -- T has no Default\n\
         crates/a/src/c.rs: replace h with () -- gone\n"
            .to_owned(),
    );
    assert_eq!(
        run_steps(&steps, &mut fake, &mut Vec::new()),
        failed(
            "Rust unviable mutant reviews all current: every review of mutants-unviable.txt \
             must name a mutant that exists: stale review: crates/a/src/c.rs: replace h with ()"
        ),
        "a review of a mutant cargo-mutants no longer lists"
    );
    let mut unlisted = passing();
    let _gone = unlisted
        .files
        .remove(&format!("capture {}", crate::mutants_list_command().line()));
    assert_eq!(
        run_steps(&steps, &mut unlisted, &mut Vec::new()),
        failed(
            "Rust unviable mutant reviews all current: cannot list the Rust mutants: \
             `cargo mutants --no-config --output=. --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --cargo-arg=--config=target.'cfg(target_os = \"linux\")'.runner=[\"prlimit\", \"--as=4294967296\"] --test-tool=nextest --cargo-test-arg=--max-fail=1:immediate --cargo-test-arg=--no-tests=warn --list \
             --workspace` exited with code 1: failed"
        ),
        "the list cannot be made"
    );
    // The GPU workspace's mutants are listed apart, each path from its root: their reviews,
    // which name them from the repository's, are current.
    let mut gpu = passing();
    let _reviewed = gpu.files.insert(
        UNVIABLE_REVIEWED.to_owned(),
        "crates/a/src/b.rs: replace f -> T with Default::default() -- T has no Default\n\
         crates/visual-gpu/src/lib.rs: replace h with () -- reviewed\n"
            .to_owned(),
    );
    assert_eq!(
        run_steps(&steps, &mut gpu, &mut Vec::new()),
        Ok(()),
        "a review of a GPU workspace's mutant"
    );
    let _unlisted_gpu = gpu.files.remove(&format!(
        "capture {}",
        crate::gpu_mutants_list_command().line()
    ));
    assert!(
        run_steps(&steps, &mut gpu, &mut Vec::new()).is_err_and(|error| error
            .summary
            .contains("cannot list the Rust mutants of the GPU workspace: `cargo mutants")),
        "the GPU workspace's list cannot be made"
    );
    assert_eq!(
        crate::gpu_mutants_list_command().line(),
        "cargo mutants --no-config --output=. --cap-lints=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --cargo-arg=--config=target.'cfg(target_os = \"linux\")'.runner=[\"prlimit\", \"--as=4294967296\"] --test-tool=nextest --cargo-test-arg=--max-fail=1:immediate --cargo-test-arg=--no-tests=warn --manifest-path crates/visual-gpu/Cargo.toml --list",
        "the GPU run's arguments, listing"
    );
}

#[test]
fn accepts_exactly_the_reviewed_unviable_mutants() {
    let report = "crates/a/src/b.rs:3:5: replace f -> T with Default::default()\n\ncrates/a/src/c.rs:10:1: replace g -> U with x\n";
    let reviewed = "# reviewed mutants\n\ncrates/a/src/c.rs: replace g -> U with x -- U has no x\n  crates/a/src/b.rs: replace f -> T with Default::default() -- T has no Default  \n";
    assert_eq!(
        unviable_gate(report, Extent::Complete, reviewed),
        Ok(()),
        "same set, lines ignored"
    );
    assert_eq!(
        unviable_gate("", Extent::Complete, ""),
        Ok(()),
        "nothing unviable"
    );
}

#[test]
fn rejects_unreviewed_stale_and_malformed_entries() {
    let error = unviable_gate(
        "crates/a/src/b.rs:3:5: replace f -> T with y\nnot a mutant line\ncrates/a/src/b.rs:9:9\n",
        Extent::Complete,
        "crates/a/src/z.rs: replace z -> Z with w -- old\ncrates/a/src/q.rs: replace q -> Q with w --  \nno separator\n",
    );
    assert_eq!(
        error,
        failed(format!(
            "unviable mutants must be reviewed in {UNVIABLE_REVIEWED}: 6 problems, listed above\n{}",
            [
                "unrecognized mutant: not a mutant line",
                "unrecognized mutant: crates/a/src/b.rs:9:9",
                "review without a reason: crates/a/src/q.rs: replace q -> Q with w --",
                "review without a reason: no separator",
                "not reviewed: crates/a/src/b.rs: replace f -> T with y",
                "stale review: crates/a/src/z.rs: replace z -> Z with w",
            ]
            .join("\n")
        )),
        "every problem is listed"
    );
}

#[test]
fn accepts_exactly_the_reviewed_equivalent_mutants() {
    let source = |text: &str| vec![("a.ts".to_owned(), text.to_owned())];
    let disabled =
        "x\n  // Stryker disable next-line EqualityOperator: overwritten later\n  i < n\n";
    assert_eq!(
        equivalent_gate(
            &source(disabled),
            "# header\n\na.ts: EqualityOperator at i < n -- overwritten later\n"
        ),
        Ok(()),
        "reviewed"
    );
    assert_eq!(
        equivalent_gate(&source("plain"), ""),
        Ok(()),
        "nothing disabled"
    );
    let error = equivalent_gate(&source(disabled), "b.ts: X at y -- old\nno reason here\n");
    assert_eq!(
        error,
        failed(format!(
            "disabled mutants must be reviewed in {EQUIVALENT_REVIEWED}: 3 problems, listed above\n{}",
            [
                "review without a reason: no reason here",
                "not reviewed: a.ts: EqualityOperator at i < n",
                "stale review: b.ts: X at y",
            ]
            .join("\n")
        )),
        "every problem is listed"
    );
}

#[test]
fn refuses_broad_or_unexplained_disables() {
    for (text, line) in [
        ("// Stryker disable all\nx\n", 1_usize),
        ("// Stryker disable next-line all: everything\nx\n", 1),
        ("a\n// Stryker disable next-line A,B: two\nx\n", 2),
        (
            "// Stryker disable next-line Equality Operator: spaced\nx\n",
            1,
        ),
        ("// Stryker disable next-line EqualityOperator:   \nx\n", 1),
        ("// Stryker disable next-line EqualityOperator\nx\n", 1),
        (
            "// Stryker disable next-line EqualityOperator: last line",
            1,
        ),
        (
            "// Stryker disable next-line EqualityOperator: blank next\n   \n",
            1,
        ),
    ] {
        assert_eq!(
            equivalent_gate(&[("a.ts".to_owned(), text.to_owned())], ""),
            failed(format!(
                "disabled mutants must be reviewed in {EQUIVALENT_REVIEWED}: a.ts:{line}: write `// Stryker disable next-line <Mutator>: <reason>` above the line"
            )),
            "{text:?}"
        );
    }
}

/// The disable comment of [`DISABLED_SOURCE`].
fn disabled() -> Vec<crate::Disable> {
    vec![crate::Disable {
        path: DISABLED_SOURCE.0.to_owned(),
        line: 3,
        mutator: "EqualityOperator".to_owned(),
    }]
}

#[test]
fn accepts_the_mutants_of_one_operation_per_disable_comment() {
    // A comparison's two mutants replace one range: one operation, reviewed by one comment.
    let one_operation = r#"{"files":{"x.ts":{"mutants":[{"status":"Ignored","mutatorName":"EqualityOperator","location":{"start":{"line":3,"column":10},"end":{"line":3,"column":15}}},{"status":"Ignored","mutatorName":"EqualityOperator","location":{"start":{"line":3,"column":10},"end":{"line":3,"column":15}}}]}}}"#;
    assert_eq!(
        ts_mutation_gate(
            "tooling/config",
            one_operation,
            &disabled(),
            Extent::Complete
        ),
        Ok(()),
        "every mutant of the one operation the comment reviews"
    );
    let two_operations = r#"{"files":{"x.ts":{"mutants":[{"status":"Ignored","mutatorName":"EqualityOperator","location":{"start":{"line":3,"column":10}}},{"status":"Ignored","mutatorName":"EqualityOperator","location":{"start":{"line":3,"column":20}}}]}}}"#;
    assert_eq!(
        ts_mutation_gate(
            "tooling/config",
            two_operations,
            &disabled(),
            Extent::Complete
        ),
        failed(
            "tooling/config: mutants not killed: tooling/config/x.ts:3 EqualityOperator: mutants of 2 operations ignored by one disable comment, which reviews one: put the equivalent operation on a line of its own"
        ),
        "a disable comment reviews one operation, never those beside it on its line"
    );
    // An operation nested in another starts where it does (`a < b` in `a < b && c`): two
    // ranges of one start are two operations.
    let nested = r#"{"files":{"x.ts":{"mutants":[{"status":"Ignored","mutatorName":"EqualityOperator","location":{"start":{"line":3,"column":10},"end":{"line":3,"column":15}}},{"status":"Ignored","mutatorName":"EqualityOperator","location":{"start":{"line":3,"column":10},"end":{"line":3,"column":20}}}]}}}"#;
    assert_eq!(
        ts_mutation_gate("tooling/config", nested, &disabled(), Extent::Complete),
        failed(
            "tooling/config: mutants not killed: tooling/config/x.ts:3 EqualityOperator: mutants of 2 operations ignored by one disable comment, which reviews one: put the equivalent operation on a line of its own"
        ),
        "two operations of one start, told apart by their ends"
    );
}

#[test]
fn accepts_only_typescript_packages_whose_mutants_are_all_killed() {
    assert_eq!(
        ts_mutation_gate(
            "tooling/config",
            TS_KILLED_OR_DISABLED,
            &disabled(),
            Extent::Complete
        ),
        Ok(()),
        "killed, timed out, or ignored where a reviewed comment says so"
    );
    for status in [
        "Survived",
        "NoCoverage",
        "RuntimeError",
        "CompileError",
        "Pending",
        "Whatever",
    ] {
        assert_eq!(
            ts_mutation_gate(
                "tooling/config",
                &one_mutant(status, "M", 7),
                &disabled(),
                Extent::Complete
            ),
            failed(format!(
                "tooling/config: mutants not killed: tooling/config/x.ts:7 M {status}"
            )),
            "{status} is not killed"
        );
    }
    for (mutator, line, package) in [
        ("EqualityOperator", 4, "tooling/config"),
        ("BooleanLiteral", 3, "tooling/config"),
        ("EqualityOperator", 3, "tooling/oracle"),
    ] {
        assert_eq!(
            ts_mutation_gate(
                package,
                &one_mutant("Ignored", mutator, line),
                &disabled(),
                Extent::Complete
            ),
            failed(format!(
                "{package}: mutants not killed: {package}/x.ts:{line} {mutator} Ignored"
            )),
            "ignored where no reviewed comment says so: {mutator} at {package}/x.ts:{line}"
        );
    }
    assert_eq!(
        ts_mutation_gate(
            "p",
            r#"{"files":{"a.ts":{"mutants":[{"status":"Survived","mutatorName":"A","location":{"start":{"line":1}}},{"status":"NoCoverage","mutatorName":"B","location":{"start":{"line":2}}},{"status":"Killed"}]}}}"#,
            &[],
            Extent::Partial
        ),
        failed(
            "p: mutants not killed: 2 problems, listed above\np/a.ts:1 A Survived\np/a.ts:2 B NoCoverage"
        ),
        "every mutant named"
    );
    for empty in [r#"{"files":{}}"#, r#"{"files":{"a.ts":{}}}"#, "{}"] {
        assert_eq!(
            ts_mutation_gate("p", empty, &[], Extent::Complete),
            failed("p: mutants not killed: no mutant in the mutation report"),
            "{empty}"
        );
        assert_eq!(
            ts_mutation_gate("p", empty, &[], Extent::Partial),
            Ok(()),
            "{empty}: a partial run may mutate nothing"
        );
    }
    assert!(
        ts_mutation_gate("p", "[", &[], Extent::Partial)
            .is_err_and(|error| error.summary.starts_with("p: invalid mutation report: ")),
        "unreadable"
    );
}

#[test]
fn refuses_stryker_configurations_that_hide_mutants() {
    assert_eq!(
        crate::stryker_config_gate("p", STRYKER_CONFIG),
        Ok(()),
        "nothing hidden"
    );
    let hiding = r#"{"ignoreStatic":true,"mutator":{"excludedMutations":["StringLiteral"]},"mutate":["src/**/*.ts","!src/x.ts"],"reporters":["html"]}"#;
    assert_eq!(
        crate::stryker_config_gate("p", hiding),
        failed(
            "p/stryker.config.json hides mutants: 4 problems, listed above\n\
             `ignoreStatic` is not allowed: it could hide mutants\n\
             `mutator` is not allowed: it could hide mutants\n\
             `!src/x.ts` in mutate excludes files from mutation\n\
             the `json` reporter is missing: the mutation report is not written"
        ),
        "a key, a negated glob and no report"
    );
    assert!(
        crate::stryker_config_gate("p", "{").is_err_and(|failure| failure
            .summary
            .starts_with("p: invalid Stryker configuration: ")),
        "unreadable"
    );
    assert_eq!(
        crate::stryker_config_gate("p", "[]"),
        failed("p: the Stryker configuration is not an object"),
        "not an object"
    );
}

#[test]
fn requires_every_bypass_of_a_proof_to_be_reviewed() {
    let rust = format!(
        "#[test]\n#[{}]\nfn a() {{}}\n#[{}::{}]\nfn b() {{}}\nlet c = d.iter().{}(1);\n",
        "ignore", "mutants", "skip", "skip"
    );
    let typescript = format!(
        "it.{}(\"x\", () => {{}});\n/* {} {} next */\n",
        "only", "v8", "ignore"
    );
    let sources = vec![
        ("crates/a/src/lib.rs".to_owned(), rust),
        ("apps/a/src/a.test.ts".to_owned(), typescript.clone()),
        (
            "tooling/config/test/policy/focused.ts".to_owned(),
            typescript,
        ),
        ("crates/a/mutants.toml".to_owned(), String::new()),
        ("README.md".to_owned(), format!("it.{}(", "only")),
    ];
    let ignore = format!("#[{}]", "ignore");
    let skip = format!("#[{}::{}]", "mutants", "skip");
    let v8 = format!("{} {}", "v8", "ignore");
    let mutants_skip = format!("{}::{}", "mutants", "skip");
    assert_eq!(
        bypass_gate_text(&sources, ""),
        failed(format!(
            "bypasses of coverage or mutation testing must be reviewed in proof-bypasses.txt: \
             5 problems, listed above\n\
             not reviewed: apps/a/src/a.test.ts: .{only}( at it.{only}(\"x\", () => {{}});\n\
             not reviewed: apps/a/src/a.test.ts: {v8} at /* {v8} next */\n\
             not reviewed: crates/a/mutants.toml: a cargo-mutants configuration\n\
             not reviewed: crates/a/src/lib.rs: #[{ignore_word} at {ignore}\n\
             not reviewed: crates/a/src/lib.rs: {mutants_skip} at {skip}",
            only = "only",
            ignore_word = "ignore",
        )),
        "every token in the language where it acts, a policy fixture and prose apart, an \
         iterator's skip apart"
    );
    let reviewed = format!(
        "crates/a/mutants.toml: a cargo-mutants configuration -- reason\n\
         crates/a/src/lib.rs: #[{} at {ignore} -- reason\n",
        "ignore"
    );
    assert_eq!(
        bypass_gate_text(&sources[2..4], &reviewed),
        failed(format!(
            "bypasses of coverage or mutation testing must be reviewed in proof-bypasses.txt: \
             stale review: crates/a/src/lib.rs: #[{} at {ignore}",
            "ignore"
        )),
        "a reviewed occurrence passes, a review of nothing is stale"
    );
}

/// The words of the bypasses, built so that this file does not hold them: a doc test fence,
/// the lint of unexpected `cfg`s, the declaration of an expected one, the comments of the
/// TypeScript coverage tools (v8, c8, istanbul, Node.js) and their option ignoring methods.
fn bypass_words() -> [String; 8] {
    [
        "`".repeat(3),
        format!("{}_{}", "unexpected", "cfgs"),
        format!("{}-{}", "check", "cfg"),
        format!("{} {}", "v8", "ignore"),
        format!("{} {}", "c8", "ignore"),
        format!("{} {}", "istanbul", "ignore"),
        format!("{} {}", "node:coverage", "ignore"),
        format!("{}{}", "ignore", "ClassMethods"),
    ]
}

/// Requires the bypass gate to refuse each of `refused` (path, text, token), reported at the
/// line where it starts: past a first line of code or a manifest section.
fn assert_refused(refused: Vec<(&str, String, String)>) {
    for (path, text, token) in refused {
        let at = text
            .lines()
            .find(|line| !line.starts_with("const ") && !line.starts_with('['))
            .unwrap_or_default();
        let sources = [(path.to_owned(), text.clone())];
        let failure = bypass_gate_text(&sources, "");
        assert!(
            failure.as_ref().is_err_and(|failure| failure
                .summary
                .contains(&format!("not reviewed: {path}: {token} at {}", at.trim()))),
            "{path}: {text:?} refused as {token}: {failure:?}"
        );
    }
}

#[test]
fn refuses_every_form_of_a_rust_bypass() {
    let [fence, cfgs, check_cfg, ..] = bypass_words();
    let attr = format!("{}_{}", "cfg", "attr");
    assert_refused(vec![
        (
            "crates/a/src/lib.rs",
            format!("/// {fence}{}\n", "ignore"),
            format!("{fence}{}", "ignore"),
        ),
        (
            "crates/a/src/lib.rs",
            format!("//! {fence}rust,{}\n", "ignore"),
            format!("{fence}{}", "ignore"),
        ),
        (
            "crates/a/src/lib.rs",
            format!("/// {}{}\n", "~".repeat(3), "no_run"),
            format!("{fence}{}", "no_run"),
        ),
        (
            "crates/a/src/lib.rs",
            format!("#[{}(any(), {})]\n", attr, "ignore"),
            attr.clone(),
        ),
        (
            "crates/a/src/lib.rs",
            format!("#[{} (all(), {})]\n", attr, "ignore"),
            attr,
        ),
        (
            "crates/a/src/lib.rs",
            format!("#[cfg(all(test, not({})))]\n", "miri"),
            "miri".to_owned(),
        ),
        (
            "crates/a/src/lib.rs",
            format!("#[cfg(not({}))]\n", "miri"),
            "miri".to_owned(),
        ),
        (
            "crates/a/src/lib.rs",
            format!("#[cfg(not( r#{} ))]\n", "miri"),
            "miri".to_owned(),
        ),
        (
            "crates/a/src/lib.rs",
            format!("if cfg!({}) {{\n", "miri"),
            "miri".to_owned(),
        ),
        (
            "crates/a/src/lib.rs",
            format!("# [ {} ]\n", "ignore"),
            format!("#[{}", "ignore"),
        ),
        (
            "crates/a/src/lib.rs",
            format!("#[{} = \"slow\"]\n", "ignore"),
            format!("#[{}", "ignore"),
        ),
        (
            "crates/a/src/lib.rs",
            format!("#[r#{}]\n", "ignore"),
            format!("r#{}", "ignore"),
        ),
        (
            "crates/a/src/lib.rs",
            format!("#![allow({cfgs}, reason = \"x\")]\n"),
            cfgs.clone(),
        ),
        (
            "crates/a/src/lib.rs",
            format!("#[expect({cfgs})]\n"),
            cfgs.clone(),
        ),
        (
            "crates/a/build.rs",
            format!("println!(\"cargo::rustc-{check_cfg}=cfg(x)\");\n"),
            check_cfg.clone(),
        ),
        (
            "crates/a/Cargo.toml",
            format!("[lints.rust]\n{cfgs} = \"allow\"\n"),
            cfgs,
        ),
        (
            "Cargo.toml",
            format!(
                "[workspace.lints.rust.x]\n{check_cfg} = [\"cfg({})\"]\n",
                "coverage"
            ),
            check_cfg,
        ),
    ]);
}

#[test]
fn refuses_what_only_a_build_no_check_makes_compiles() {
    let assertions = format!("{}_{}", "debug", "assertions");
    let doc = "doc";
    let doc_test = format!("{doc}{}", "test");
    assert_refused(vec![
        (
            "crates/a/src/lib.rs",
            format!("#[cfg(not({assertions}))]\n"),
            assertions.clone(),
        ),
        (
            "crates/a/src/lib.rs",
            format!("if cfg!({assertions}) {{\n"),
            assertions,
        ),
        (
            "crates/a/src/lib.rs",
            format!("#[cfg({doc_test})]\n"),
            format!("({doc_test})"),
        ),
        (
            "crates/a/src/lib.rs",
            format!("#[cfg(any({doc_test} , unix))]\n"),
            format!("({doc_test},"),
        ),
        (
            "crates/a/src/lib.rs",
            format!("#[cfg(any(unix, {doc_test}))]\n"),
            format!(",{doc_test})"),
        ),
        (
            "crates/a/src/lib.rs",
            format!("#[cfg(any(unix,\n    {doc_test},\n    windows,\n))]\n"),
            format!(",{doc_test},"),
        ),
        (
            "crates/a/src/lib.rs",
            format!("#[cfg(not( {doc} ))]\n"),
            format!("({doc})"),
        ),
        (
            "crates/a/src/lib.rs",
            format!("#[cfg(any({doc}, unix))]\n"),
            format!("({doc},"),
        ),
        (
            "crates/a/src/lib.rs",
            format!("#[cfg(any(unix, {}))]\n", doc.to_uppercase()),
            format!(",{doc})"),
        ),
        (
            "crates/a/src/lib.rs",
            format!("#[cfg(any(unix, {doc}, windows))]\n"),
            format!(",{doc},"),
        ),
    ]);
    let features = format!("[{}]", "features");
    for manifest in [
        format!("{features}\nx = []\n"),
        format!("[ {} ]\nx = []\n", "features"),
    ] {
        assert_eq!(
            bypass_gate_text(&[("crates/a/Cargo.toml".to_owned(), manifest.clone())], ""),
            failed(format!(
                "bypasses of coverage or mutation testing must be reviewed in \
                 proof-bypasses.txt: not reviewed: crates/a/Cargo.toml: {features} at {}",
                manifest.lines().next().unwrap_or_default()
            )),
            "{manifest}"
        );
    }
    for (path, text) in [
        (
            "crates/a/src/lib.rs",
            format!(
                "#[{doc}(hidden)]\nfn f() {{ debug_assert!(true); }}\n// the {doc} tests\n\
                 let {doc}_tested = ({doc}s, x);\n"
            ),
        ),
        (
            "crates/a/Cargo.toml",
            format!(
                "[dependencies]\nx = {{ version = \"1\", {} = [\"a\"] }}\n",
                "features"
            ),
        ),
        (
            "crates/a/README.md",
            format!("#[cfg({doc_test})] {features}\n"),
        ),
    ] {
        assert_eq!(
            bypass_gate_text(&[(path.to_owned(), text.clone())], ""),
            Ok(()),
            "{path}: {text:?} is no bypass"
        );
    }
}

#[test]
fn refuses_the_harness_key_in_every_manifest() {
    let key = format!("{}{}", "har", "ness");
    assert_refused(vec![
        (
            "crates/a/Cargo.toml",
            format!("[[test]]\n{key} = false\n"),
            key.clone(),
        ),
        (
            "Cargo.toml",
            format!("[[bench]]\n\"{}{}\"=true\n", "HAR", "NESS"),
            key.clone(),
        ),
    ]);
    let manifest = [(
        "crates/a/Cargo.toml".to_owned(),
        format!("[[bench]]\n{key} = false\n"),
    )];
    assert_eq!(
        bypass_gate_text(
            &manifest,
            &format!("crates/a/Cargo.toml: {key} at {key} = false -- a benchmark's own main\n")
        ),
        Ok(()),
        "a reviewed key"
    );
    assert_eq!(
        bypass_gate_text(
            &[(
                "crates/a/src/lib.rs".to_owned(),
                format!("// {key} = false\n")
            )],
            ""
        ),
        Ok(()),
        "the word in Rust is no manifest key"
    );
}

#[test]
fn refuses_every_form_of_a_typescript_bypass() {
    let [_, _, _, v8, c8, istanbul, node, methods] = bypass_words();
    assert_refused(vec![
        (
            "apps/a/src/a.ts",
            format!("/* {} next */\n", v8.replace(' ', "  ")),
            v8.clone(),
        ),
        (
            "apps/a/src/a.ts",
            format!("/* {} start */\n", v8.replace(' ', "\t")),
            v8.clone(),
        ),
        (
            "apps/a/src/a.ts",
            format!("/* {} next */\n", v8.to_uppercase()),
            v8.clone(),
        ),
        (
            "apps/a/src/a.ts",
            format!("const a = 1;\n/* {}\n   next */\n", v8.replace(' ', "\n  ")),
            v8.clone(),
        ),
        ("apps/a/src/a.mts", format!("/* {node} next */\n"), node),
        (
            "apps/a/src/a.cts",
            format!("/* {istanbul} else */\n"),
            istanbul,
        ),
        ("apps/a/src/a.cjs", format!("// {c8} next\n"), c8),
        ("apps/a/src/a.jsx", format!("/* {v8} next */\n"), v8),
        (
            "apps/a/vitest.config.ts",
            format!("coverage: {{ {methods}: [\"run\"] }},\n"),
            methods,
        ),
        (
            "apps/a/src/a.test.jsx",
            format!("it.{}(\"x\", () => {{}});\n", "only"),
            format!(".{}(", "only"),
        ),
    ]);
}

#[test]
fn accepts_what_only_resembles_a_bypass() {
    let [fence, cfgs, check_cfg, v8, _, _, _, methods] = bypass_words();
    let attr = format!("{}_{}", "cfg", "attr");
    let accepted = [
        (
            "crates/a/src/lib.rs",
            format!("/// {fence}text\n/// {fence}\n"),
        ),
        (
            "crates/a/src/lib.rs",
            format!(
                "/// {fence}compile_fail,E0599\n// we {} nothing\n",
                "ignore"
            ),
        ),
        ("crates/a/src/lib.rs", format!("/* {v8} next */\n")),
        (
            "crates/a/src/lib.rs",
            format!(
                "let tool = {:?};\n// Miri checks it\nlet {}n = x_{};\nlet v = vec![{}];\n",
                "miri", "miri", "miri", "ignore"
            ),
        ),
        (
            "README.md",
            format!("#[{}(not({}), {})]\n", attr, "miri", "ignore"),
        ),
        (
            "apps/a/src/a.ts",
            "coverage: { provider: \"v8\" },\n".to_owned(),
        ),
        (
            "apps/a/src/a.ts",
            "const c8 = 1;\nconst nore = c8;\n".to_owned(),
        ),
        ("README.md", format!("/* {v8} next */ {cfgs} {check_cfg}\n")),
        ("apps/a/a.json", format!("{{\"{methods}\": []}}\n")),
    ];
    for (path, text) in accepted {
        assert_eq!(
            bypass_gate_text(&[(path.to_owned(), text.clone())], ""),
            Ok(()),
            "{path}: {text:?} is no bypass"
        );
    }
}

#[test]
fn refuses_a_package_script_running_stryker_otherwise() {
    let manifest = |scripts: serde_json::Value| {
        serde_json::json!({ "name": "p", "scripts": scripts }).to_string()
    };
    for accepted in [
        manifest(serde_json::json!({ "test:mutation": "stryker run stryker.config.json" })),
        manifest(serde_json::json!({ "check": "vitest run" })),
        r#"{"name":"p"}"#.to_owned(),
    ] {
        assert_eq!(
            crate::stryker_scripts_gate("p/package.json", &accepted),
            Ok(()),
            "{accepted}"
        );
    }
    assert_eq!(
        crate::stryker_scripts_gate(
            "p/package.json",
            &manifest(serde_json::json!({
                "a": "stryker run",
                "b": "stryker run stryker.conf.json",
                "c": "stryker run stryker.config.json --mutate src/a.ts",
                "d": "pnpm exec stryker run stryker.config.json",
                "e": "vitest run",
            }))
        ),
        failed(
            "p/package.json runs Stryker otherwise than `stryker run stryker.config.json`: \
             4 problems, listed above\n\
             `a`: stryker run\n\
             `b`: stryker run stryker.conf.json\n\
             `c`: stryker run stryker.config.json --mutate src/a.ts\n\
             `d`: pnpm exec stryker run stryker.config.json"
        ),
        "another configuration, another argument, another wrapper"
    );
    assert!(
        crate::stryker_scripts_gate("p/package.json", "{").is_err_and(|failure| failure
            .summary
            .starts_with("p/package.json: invalid manifest: ")),
        "unreadable"
    );
    assert_eq!(
        crate::stryker_scripts_gate("p/package.json", r#"{"scripts":{"a":1}}"#),
        failed(
            "p/package.json runs Stryker otherwise than `stryker run stryker.config.json`: \
             `a` is not a command"
        ),
        "a script that is no string is refused, not skipped"
    );
}

#[test]
fn the_stryker_step_reads_every_manifest() {
    let step = [Step {
        name: "Stryker".to_owned(),
        action: Action::StrykerConfigGate,
    }];
    assert_eq!(
        run_steps(&step, &mut passing(), &mut Vec::new()),
        Ok(()),
        "the fake passes"
    );
    for path in package_jsons() {
        let mut fake = passing();
        let _script = fake.files.insert(
            path.clone(),
            r#"{"scripts":{"m":"stryker run"}}"#.to_owned(),
        );
        assert_eq!(
            run_steps(&step, &mut fake, &mut Vec::new()),
            failed(format!(
                "Stryker: {path} runs Stryker otherwise than `stryker run stryker.config.json`: \
                 `m`: stryker run"
            )),
            "{path}"
        );
    }
    for missing in [
        "apps/roadmap/stryker.config.json",
        "apps/roadmap/package.json",
    ] {
        let mut lacking = passing();
        let removed = lacking.files.remove(missing);
        assert!(removed.is_some(), "{missing} removed");
        assert_eq!(
            run_steps(&step, &mut lacking, &mut Vec::new()),
            failed(format!("Stryker: cannot read {missing}: missing")),
            "{missing} cannot be read"
        );
    }
}

/// Runs [`crate::bypass_gate`].
fn bypass_gate_text(sources: &[(String, String)], reviewed: &str) -> Result<(), Failure> {
    crate::bypass_gate(sources, reviewed)
}

#[test]
fn knows_exactly_the_packages_and_crates_of_the_repository() {
    let packages = package_jsons();
    let members: Vec<(String, String)> = crate::RUST_CRATES
        .iter()
        .map(|krate| (format!("{}/Cargo.toml", krate.dir), MEMBER.to_owned()))
        .collect();
    let mut crates = members.clone();
    crates.push(("Cargo.toml".to_owned(), "[workspace]".to_owned()));
    crates.push((
        format!("{}x/Cargo.toml", crate::FIXTURES),
        "[package]\nversion = \"0.0.0\"\n".to_owned(),
    ));
    assert_eq!(
        membership_gate(&packages, &crates, &[".gitignore".to_owned()]),
        Ok(()),
        "known, a fixture of xtask apart"
    );
    assert_eq!(
        membership_gate(&packages, &crates, &[".gitignore".to_owned(), "bindings/audio-wasm/.gitignore".to_owned()]),
        found("the manifests differ from what xtask knows", &["bindings/audio-wasm/.gitignore: build outputs go to dist/ or target/, not hidden in a source folder".to_owned()]),
        "a nested .gitignore"
    );

    let mut more = packages.clone();
    more.push("apps/editor/package.json".to_owned());
    let mut own = members.clone();
    own[0].1 =
        "[package]\nversion = \"9.0.0\"\nrepository.workspace = true\n\n[lints]\nworkspace = true\n"
            .to_owned();
    own.push(("crates/b/Cargo.toml".to_owned(), MEMBER.to_owned()));
    assert_eq!(
        membership_gate(&more, &own, &[]),
        found(
            "the manifests differ from what xtask knows",
            &[
                "apps/editor/package.json is not a package of PACKAGES: its checks and reports would not run"
                    .to_owned(),
                "crates/b/Cargo.toml is not a crate of RUST_CRATES: its coverage would not be measured"
                    .to_owned(),
                "crates/audio-core/Cargo.toml does not take the workspace version".to_owned(),
            ]
        ),
        "an unknown package and crate, a crate with its own version"
    );
    let fewer: Vec<String> = packages
        .iter()
        .filter(|package| *package != "apps/roadmap/package.json")
        .cloned()
        .collect();
    assert_eq!(
        membership_gate(&fewer, &members[1..], &[]),
        found(
            "the manifests differ from what xtask knows",
            &[
                "apps/roadmap/package.json belongs to PACKAGES but is not tracked".to_owned(),
                "crates/audio-core/Cargo.toml belongs to RUST_CRATES but is not tracked".to_owned(),
            ]
        ),
        "a package and a crate that are gone"
    );
}

#[test]
fn reviews_each_occurrence_of_a_mutant() {
    let report = "crates/a/src/b.rs:10:5: replace && with || in f\ncrates/a/src/b.rs:20:9: replace && with || in f\n";
    let once = "crates/a/src/b.rs: replace && with || in f -- a let chain\n";
    assert_eq!(
        unviable_gate(report, Extent::Complete, once),
        found(
            &format!("unviable mutants must be reviewed in {UNVIABLE_REVIEWED}"),
            &[
                "not reviewed: crates/a/src/b.rs: replace && with || in f (2 found, 1 reviewed)"
                    .to_owned()
            ]
        ),
        "the second occurrence is not reviewed"
    );
    let twice = format!("{once}crates/a/src/b.rs: replace && with || in f -- another let chain\n");
    assert_eq!(
        unviable_gate(report, Extent::Complete, &twice),
        Ok(()),
        "each occurrence reviewed"
    );
    let thrice = format!("{twice}{once}");
    assert_eq!(
        unviable_gate(report, Extent::Complete, &thrice),
        found(
            &format!("unviable mutants must be reviewed in {UNVIABLE_REVIEWED}"),
            &[
                "stale review: crates/a/src/b.rs: replace && with || in f (2 found, 3 reviewed)"
                    .to_owned()
            ]
        ),
        "a review too many"
    );
    let source = "// Stryker disable next-line EqualityOperator: same\nx < 1\n// Stryker disable next-line EqualityOperator: same\nx < 1\n";
    assert_eq!(
        equivalent_gate(
            &[("a.ts".to_owned(), source.to_owned())],
            "a.ts: EqualityOperator at x < 1 -- same\n"
        ),
        found(
            &format!("disabled mutants must be reviewed in {EQUIVALENT_REVIEWED}"),
            &["not reviewed: a.ts: EqualityOperator at x < 1 (2 found, 1 reviewed)".to_owned()]
        ),
        "the same disable twice in a file needs two reviews"
    );
}

#[test]
fn requires_every_crate_to_inherit_the_workspace_lints() {
    let packages = package_jsons();
    let crates: Vec<(String, String)> = crate::RUST_CRATES
        .iter()
        .map(|krate| {
            let text = if krate.name == "audio-core" {
                format!(
                    "[package]\nversion.workspace = true\nrepository.workspace = true\n\n\
                     [lints.rust]\n{}_{} = \"allow\"\n",
                    "unsafe", "code"
                )
            } else if krate.name == "audio-hash" {
                "[package]\nversion.workspace = true\nrepository.workspace = true\n\n[lints]\n\n\
                 workspace = true\n"
                    .to_owned()
            } else {
                MEMBER.to_owned()
            };
            (format!("{}/Cargo.toml", krate.dir), text)
        })
        .collect();
    assert_eq!(
        membership_gate(&packages, &crates, &[]),
        found(
            "the manifests differ from what xtask knows",
            &[
                "crates/audio-core/Cargo.toml does not inherit the workspace lints (`[lints]` then `workspace = true`)".to_owned(),
                "crates/audio-hash/Cargo.toml does not inherit the workspace lints (`[lints]` then `workspace = true`)".to_owned(),
            ]
        ),
        "lints of its own, or the inheritance away from its table"
    );
}

#[test]
fn requires_every_crate_to_take_the_workspace_repository() {
    let packages = package_jsons();
    let crates: Vec<(String, String)> = crate::RUST_CRATES
        .iter()
        .map(|krate| {
            let text = if krate.name == "audio-core" {
                MEMBER.replace(
                    "repository.workspace = true",
                    "repository = \"https://example.invalid/poyetik\"",
                )
            } else {
                MEMBER.to_owned()
            };
            (format!("{}/Cargo.toml", krate.dir), text)
        })
        .collect();
    assert_eq!(
        membership_gate(&packages, &crates, &[]),
        found(
            "the manifests differ from what xtask knows",
            &["crates/audio-core/Cargo.toml does not take the workspace repository".to_owned()]
        ),
        "a repository of its own"
    );
}

#[test]
fn matches_the_globs_of_stryker_mutate() {
    for (glob, path, expected) in [
        ("src/**/*.ts", "src/a.ts", true),
        ("src/**/*.ts", "src/a/b/c.ts", true),
        ("src/**/*.ts", "src/a.tsx", false),
        ("src/**/*.ts", "lib/a.ts", false),
        ("src/**/*.ts", "src", false),
        ("csp.ts", "csp.ts", true),
        ("csp.ts", "src/csp.ts", false),
        ("csp.ts", "csp.tsx", false),
        ("*.ts", "a.ts", true),
        ("*.ts", "a/b.ts", false),
        ("src/*/x.ts", "src/a/x.ts", true),
        ("src/*/x.ts", "src/a/b/x.ts", false),
        ("a*b*c.ts", "abxbc.ts", true),
        ("a*b*c.ts", "ab.ts", false),
        ("a*b*c.ts", "acb.ts", false),
        ("a*b*c.ts", "axc.ts", false),
        ("a*a", "a", false),
        ("a*a", "aa", true),
        ("**", "a/b", true),
        ("**/x.ts", "x.ts", true),
        ("e2e/**", "e2e/a/b.ts", true),
    ] {
        let pattern: Vec<&str> = glob.split('/').collect();
        let segments: Vec<&str> = path.split('/').collect();
        assert_eq!(
            crate::reviews::glob_matches(&pattern, &segments),
            expected,
            "{glob} against {path}"
        );
    }
}

#[test]
fn requires_every_source_mutated_in_the_mutation_report() {
    let tracked: Vec<String> = ["p/src/a.ts", "p/src/b.ts", "p/src/c.ts", "p/src/env.d.ts"]
        .map(str::to_owned)
        .to_vec();
    let statements = |count: u64| serde_json::json!({ "statements": { "total": count } });
    let summary = serde_json::json!({
        "/r/p/src/a.ts": statements(4),
        "/r/p/src/b.ts": statements(1),
        "/r/p/src/c.ts": {},
        "/r/p/src/env.d.ts": statements(0),
        "total": statements(5),
    })
    .to_string();
    let only_a = crate::Scope {
        dir: "p",
        stryker: serde_json::json!({ "mutate": ["src/**/*.ts"] }).to_string(),
        summary,
        mutation: r#"{"files":{"src/a.ts":{"mutants":[]}}}"#.to_owned(),
    };
    let context = "TypeScript sources outside coverage or mutation testing must be reviewed in \
                   proof-bypasses.txt";
    assert_eq!(
        crate::scope_gate(&tracked, core::slice::from_ref(&only_a), ""),
        failed(format!(
            "{context}: 2 problems, listed above\n\
             not reviewed: p/src/b.ts: mutated, no mutant reported\n\
             not reviewed: p/src/c.ts: mutated, no mutant reported"
        )),
        "a source with statements, or whose statements are not counted, must be reported; \
         one without any (declarations only) need not"
    );
    let reviewed = "p/src/b.ts: mutated, no mutant reported -- constants only\n\
                    p/src/c.ts: mutated, no mutant reported -- constants only\n";
    assert_eq!(
        crate::scope_gate(&tracked, core::slice::from_ref(&only_a), reviewed),
        Ok(()),
        "reviewed"
    );
    assert_eq!(
        crate::bypass_gate(&[], reviewed),
        failed(
            "bypasses of coverage or mutation testing must be reviewed in proof-bypasses.txt: \
             2 problems, listed above\n\
             stale review: p/src/b.ts: mutated, no mutant reported (a file of no package)\n\
             stale review: p/src/c.ts: mutated, no mutant reported (a file of no package)"
        ),
        "a scope review of a file in no package of the checks: no scope gate reads it"
    );
    let packaged = "apps/roadmap/src/b.ts: mutated, no mutant reported -- constants only\n\
                    # p/src/b.ts: mutated, no mutant reported\n";
    assert_eq!(
        crate::bypass_gate(&[], packaged),
        Ok(()),
        "a scope review of a package's file is left to its scope gate; a comment is no review"
    );
    let lookalike = "apps/roadmapx/src/b.ts: not measured -- r\napps/roadmap: not measured -- r\n";
    assert_eq!(
        crate::bypass_gate(&[], lookalike),
        failed(
            "bypasses of coverage or mutation testing must be reviewed in proof-bypasses.txt: \
             2 problems, listed above\n\
             stale review: apps/roadmapx/src/b.ts: not measured (a file of no package)\n\
             stale review: apps/roadmap: not measured (a file of no package)"
        ),
        "below a package's directory, not beside it nor the directory itself"
    );
    for (mutation, problem) in [
        ("{", "invalid mutation report: "),
        ("{}", "the mutation report lists no `files`"),
    ] {
        let unreadable = crate::Scope {
            mutation: mutation.to_owned(),
            ..only_a.clone()
        };
        assert!(
            crate::scope_gate(&tracked, &[unreadable], "")
                .is_err_and(|failure| failure.summary.contains(&format!(": p: {problem}"))),
            "{mutation}"
        );
    }
}

#[test]
fn requires_every_typescript_source_measured_and_mutated_or_reviewed() {
    let tracked: Vec<String> = [
        "p/src/a.ts",
        "p/src/b.astro",
        "p/src/c.ts",
        "p/test/a.test.ts",
        "p/test/helper.ts",
        "p/e2e/x.spec.ts",
        "p/e2e/helper.ts",
        "p/vite.config.ts",
        "q/src/z.ts",
    ]
    .map(str::to_owned)
    .to_vec();
    let incomplete = scope(
        "p",
        &["src/**/*.ts", "{a,b}.ts", "lib/*.ts"],
        &[
            "/r/p/src/a.ts",
            "/r/p/src/b.astro",
            "/r/p/test/a.test.ts",
            "/r/q/src/z.ts",
        ],
    );
    let other = scope("q", &["src/**/*.ts"], &["/r/q/src/z.ts"]);
    let context = "TypeScript sources outside coverage or mutation testing must be reviewed in \
                   proof-bypasses.txt";
    assert_eq!(
        crate::scope_gate(&tracked, &[incomplete, other.clone()], ""),
        failed(format!(
            "{context}: 8 problems, listed above\n\
             p: the coverage measures /r/p/test/a.test.ts, which is not a tracked source of the package\n\
             p: the coverage measures /r/q/src/z.ts, which is not a tracked source of the package\n\
             p/stryker.config.json: `{{a,b}}.ts` in mutate: only `*` and `**` are understood\n\
             p/src/c.ts is mutated but not measured: the coverage `include` misses it\n\
             p/stryker.config.json: `lib/*.ts` in mutate matches no source\n\
             not reviewed: p/e2e/helper.ts: not measured\n\
             not reviewed: p/src/b.astro: measured, not mutated\n\
             not reviewed: p/vite.config.ts: not measured"
        )),
        "tests apart, every source measured and mutated or an occurrence; q is complete"
    );
    let reviewed = "# reviews\n\
                    p/e2e/helper.ts: not measured -- a reason\n\
                    p/src/b.astro: measured, not mutated -- a reason\n\
                    p/vite.config.ts: not measured -- a reason\n\
                    p/gone.ts: not measured -- a reason\n\
                    crates/a/src/lib.rs: a token at a line -- the bypass gate's\n";
    let complete = scope(
        "p",
        &["src/**/*.ts"],
        &["/r/p/src/a.ts", "/r/p/src/b.astro", "/r/p/src/c.ts"],
    );
    assert_eq!(
        crate::scope_gate(&tracked, &[complete, other.clone()], reviewed),
        failed(format!("{context}: stale review: p/gone.ts: not measured")),
        "reviewed occurrences pass, a review of nothing is stale, the other reviews are the \
         bypass gate's"
    );
    let other_package = format!("{reviewed}q/src/gone.ts: not measured -- q's review\n");
    let complete_alone = scope(
        "p",
        &["src/**/*.ts"],
        &["/r/p/src/a.ts", "/r/p/src/b.astro", "/r/p/src/c.ts"],
    );
    assert_eq!(
        crate::scope_gate(
            &tracked,
            core::slice::from_ref(&complete_alone),
            &other_package
        ),
        failed(format!("{context}: stale review: p/gone.ts: not measured")),
        "a package checked alone leaves the reviews of another package to its own gate"
    );
    assert_eq!(
        crate::scope_gate(&tracked, &[complete_alone, other.clone()], &other_package),
        failed(format!(
            "{context}: 2 problems, listed above\n\
             stale review: p/gone.ts: not measured\n\
             stale review: q/src/gone.ts: not measured"
        )),
        "each package checks the reviews of its own files"
    );
    let rust_only = "crates/a/src/lib.rs: a token at a line -- the bypass gate's\n";
    assert_eq!(
        crate::bypass_gate(&[], rust_only),
        failed(
            "bypasses of coverage or mutation testing must be reviewed in proof-bypasses.txt: \
             stale review: crates/a/src/lib.rs: a token at a line"
        ),
        "the bypass gate's own reviews"
    );
    let unreadable = crate::Scope {
        dir: "q",
        stryker: "{".to_owned(),
        summary: String::new(),
        mutation: String::new(),
    };
    assert!(
        crate::scope_gate(&tracked, &[unreadable], "").is_err_and(|failure| failure
            .summary
            .contains(": q: invalid Stryker configuration: ")),
        "an unreadable Stryker configuration"
    );
    let no_summary = crate::Scope {
        summary: "[".to_owned(),
        ..other
    };
    assert!(
        crate::scope_gate(&tracked, &[no_summary], "")
            .is_err_and(|failure| failure.summary.contains(": q: invalid coverage summary: ")),
        "an unreadable coverage summary"
    );
}

/// A package `p` of the scope gate: its Stryker configuration mutates `mutate`, its
/// coverage summary measures `measured` (absolute paths, below `/r/`) and its mutation
/// report lists each of them.
fn scope(dir: &'static str, mutate: &[&str], measured: &[&str]) -> crate::Scope {
    let files: serde_json::Map<String, serde_json::Value> = measured
        .iter()
        .chain(&["total"])
        .map(|file| ((*file).to_owned(), serde_json::json!({})))
        .collect();
    let reported: serde_json::Map<String, serde_json::Value> = measured
        .iter()
        .filter_map(|file| file.strip_prefix(&format!("/r/{dir}/")))
        .map(|file| (file.to_owned(), serde_json::json!({ "mutants": [] })))
        .collect();
    crate::Scope {
        dir,
        stryker: serde_json::json!({ "mutate": mutate }).to_string(),
        summary: serde_json::Value::Object(files).to_string(),
        mutation: serde_json::json!({ "files": reported }).to_string(),
    }
}

/// The manifest of a package named `name`, depending on `uses` (under `section`) and on
/// `react`, which is no workspace package.
fn manifest_using(name: &str, section: &str, uses: &[&str]) -> String {
    let mut dependencies = serde_json::Map::new();
    let _react = dependencies.insert("react".to_owned(), serde_json::json!("^19"));
    for used in uses {
        let _used = dependencies.insert((*used).to_owned(), serde_json::json!("workspace:*"));
    }
    serde_json::json!({ "name": name, section: dependencies }).to_string()
}

/// The manifests of the real packages, each depending on what its real one does, under
/// `section`, and exporting a file of its build outputs, if it has any.
fn real_manifests(section: &str) -> Vec<String> {
    crate::PACKAGES
        .iter()
        .map(|package| {
            let name = package.dir.rsplit('/').next().unwrap_or_default();
            let manifest = manifest_using(
                &format!("@poyetik/{name}"),
                section,
                crate::tests::workspace_dependencies(package.dir),
            );
            if package.outputs.is_empty() {
                manifest
            } else {
                exporting(&manifest, serde_json::json!({ "./a": "./dist/a.js" }))
            }
        })
        .collect()
}

/// `manifest` with `exports` as its exports.
fn exporting(manifest: &str, exports: serde_json::Value) -> String {
    let mut json: serde_json::Value = serde_json::from_str(manifest).expect("a manifest");
    if let Some(fields) = json.as_object_mut() {
        let _exports = fields.insert("exports".to_owned(), exports);
    }
    json.to_string()
}

#[test]
fn holds_each_package_s_build_outputs_to_what_it_exports() {
    let oracle = manifest_using(
        "@poyetik/oracle",
        "dependencies",
        crate::tests::workspace_dependencies("tooling/oracle"),
    );
    assert_eq!(
        order_gate(&manifests_with(
            "tooling/oracle",
            exporting(
                &oracle,
                serde_json::json!({ "./x": { "types": "./dist/x.d.ts" } })
            )
        )),
        failed(
            "tooling/oracle/package.json: exports its build outputs (dist/), which `outputs` \
             (`PACKAGES` in xtask's workspace.rs) does not list, so that they are never \
             emptied before the checks nor built for the packages that read them"
        ),
        "an export of a build output the table does not declare"
    );
    let visual = manifest_using(
        "@poyetik/visual-wasm",
        "dependencies",
        crate::tests::workspace_dependencies("bindings/visual-wasm"),
    );
    assert_eq!(
        order_gate(&manifests_with(
            "bindings/visual-wasm",
            exporting(
                &visual,
                serde_json::json!({ "./visual": "./src/visual.ts" })
            )
        )),
        failed(
            "bindings/visual-wasm/package.json: `outputs` (`PACKAGES` in xtask's \
             workspace.rs) lists dist/, which it does not export"
        ),
        "a build output the manifest no longer exports"
    );
    assert_eq!(
        order_gate(&manifests_with(
            "bindings/visual-wasm",
            exporting(
                &visual,
                serde_json::json!({ "./visual": { "types": "./dist/v.d.ts", "default": "./dist/v.js" }, "./m.wasm": "./dist/m.wasm" })
            )
        )),
        Ok(()),
        "nested conditions read"
    );
}

/// The package order gate over the packages of the checks, with `texts` as their manifests.
fn order_gate(texts: &[String]) -> Result<(), Failure> {
    let manifests: Vec<(crate::Package, &str)> = crate::PACKAGES
        .iter()
        .copied()
        .zip(texts.iter().map(String::as_str))
        .collect();
    crate::package_order_gate(&manifests)
}

/// The real manifests, but the one of the package in `dir`, which is `text`.
fn manifests_with(dir: &str, text: String) -> Vec<String> {
    let mut texts = real_manifests("dependencies");
    let at = crate::PACKAGES
        .iter()
        .position(|package| package.dir == dir)
        .expect("a package");
    if let Some(slot) = texts.get_mut(at) {
        *slot = text;
    }
    texts
}

#[test]
fn runs_each_package_after_the_workspace_packages_it_uses() {
    for section in crate::NPM_FIELDS {
        assert_eq!(
            order_gate(&real_manifests(section)),
            Ok(()),
            "the real order, from {section}"
        );
    }
    assert_eq!(
        order_gate(&manifests_with(
            "tooling/oracle",
            manifest_using(
                "@poyetik/oracle",
                "devDependencies",
                &["@poyetik/wasm-build"]
            )
        )),
        failed(
            "tooling/oracle/package.json: depends on @poyetik/wasm-build, whose checks do not \
             run before its own (`PACKAGES` in xtask's workspace.rs)"
        ),
        "a package checked later"
    );
    assert_eq!(
        order_gate(&manifests_with(
            "apps/roadmap",
            manifest_using(
                "@poyetik/roadmap",
                "dependencies",
                &["@poyetik/roadmap", "@poyetik/nowhere"]
            )
        )),
        Err(Failure {
            summary: "2 problems, listed above".to_owned(),
            details: vec![
                "apps/roadmap/package.json: depends on @poyetik/nowhere, which is no package of \
                 the checks"
                    .to_owned(),
                "apps/roadmap/package.json: depends on @poyetik/roadmap, whose checks do not run \
                 before its own (`PACKAGES` in xtask's workspace.rs)"
                    .to_owned(),
            ],
            usage: false,
        }),
        "no package at all, and itself (in the order of the manifest's keys)"
    );
    assert_eq!(
        order_gate(&manifests_with(
            "bindings/visual-wasm",
            exporting(
                &manifest_using(
                    "@poyetik/visual-wasm",
                    "dependencies",
                    &[
                        "@poyetik/config",
                        "@poyetik/oracle",
                        "@poyetik/wasm-build",
                        "@poyetik/visual-wasm"
                    ]
                ),
                serde_json::json!({ "./a": "./dist/a.js" })
            )
        )),
        failed(
            "bindings/visual-wasm/package.json: depends on @poyetik/visual-wasm, whose checks \
             do not run before its own (`PACKAGES` in xtask's workspace.rs)"
        ),
        "a package with builds that uses itself: refused once, its own build never one it needs"
    );
}

#[test]
fn prepares_exactly_the_builds_a_package_reads() {
    assert_eq!(
        order_gate(&manifests_with(
            "apps/roadmap",
            manifest_using(
                "@poyetik/roadmap",
                "peerDependencies",
                &["@poyetik/web-page"]
            )
        )),
        failed(
            "apps/roadmap/package.json: reads the builds of [\"bindings/visual-wasm\", \
             \"bindings/web-page\"], in that order, but its `needs` (`PACKAGES` in xtask's \
             workspace.rs) build []"
        ),
        "the builds it reads, through the page module the visual library's too, undeclared"
    );
    assert_eq!(
        order_gate(&manifests_with(
            "apps/playground",
            manifest_using(
                "@poyetik/playground",
                "dependencies",
                &["@poyetik/audio-wasm"]
            )
        )),
        failed(
            "apps/playground/package.json: reads the builds of [\"bindings/audio-wasm\"], in \
             that order, but its `needs` (`PACKAGES` in xtask's workspace.rs) build \
             [\"bindings/audio-wasm\", \"bindings/visual-wasm\", \"bindings/web-page\"]"
        ),
        "builds it no longer reads, still prepared"
    );
    assert_eq!(
        order_gate(&manifests_with("tooling/config", "{".to_owned()))
            .map_err(|failure| failure.summary),
        Err(
            "tooling/config/package.json: invalid manifest: EOF while parsing an object at \
             line 1 column 1"
                .to_owned()
        ),
        "an invalid manifest"
    );
    assert_eq!(
        order_gate(&manifests_with(
            "tooling/config",
            r#"{"name": 1}"#.to_owned()
        )),
        failed("tooling/config/package.json: no `name`"),
        "a manifest without a name"
    );
}

#[test]
fn ties_each_package_check_to_the_scripts_the_checks_run() {
    use crate::{check_script, package_check_gate};
    assert_eq!(
        check_script(&["typecheck", "test:e2e"]),
        "pnpm typecheck && pnpm test:e2e",
        "each script through pnpm, in order"
    );
    let checks = ["typecheck", "lint"];
    let context = "a/package.json: the checks run and time `pnpm typecheck && pnpm lint` \
                   (`Package::checks` in xtask's workspace.rs, which changes with the manifest)";
    assert_eq!(
        package_check_gate(
            "a/package.json",
            r#"{"scripts": {"check": "pnpm typecheck && pnpm lint", "test": "vitest"}}"#,
            &checks
        ),
        Ok(()),
        "exactly the scripts, in order"
    );
    for (manifest, problem) in [
        (
            r#"{"scripts": {"check": "pnpm lint && pnpm typecheck"}}"#,
            "its `check` script runs `pnpm lint && pnpm typecheck`",
        ),
        (
            r#"{"scripts": {"check": "pnpm typecheck && pnpm lint && pnpm test"}}"#,
            "its `check` script runs `pnpm typecheck && pnpm lint && pnpm test`",
        ),
        (
            r#"{"scripts": {"check": "pnpm run typecheck && pnpm run lint"}}"#,
            "its `check` script runs `pnpm run typecheck && pnpm run lint`",
        ),
        (
            r#"{"scripts": {"test": "vitest"}}"#,
            "it has no `check` script",
        ),
        (r#"{"scripts": {"check": 1}}"#, "it has no `check` script"),
        ("{}", "it has no `check` script"),
        (
            r#"{"scripts": {"check": "pnpm typecheck && pnpm lint", "precheck": "x"}}"#,
            "it has a `precheck` script, which the checks would not run",
        ),
        (
            r#"{"scripts": {"check": "pnpm typecheck && pnpm lint", "postcheck": "x"}}"#,
            "it has a `postcheck` script, which the checks would not run",
        ),
    ] {
        assert_eq!(
            package_check_gate("a/package.json", manifest, &checks),
            Err(Failure::from(format!("{context}: {problem}"))),
            "{manifest}"
        );
    }
    assert_eq!(
        package_check_gate(
            "a/package.json",
            r#"{"scripts": {"precheck": "x", "postcheck": "y"}}"#,
            &checks
        ),
        Err(Failure {
            summary: format!("{context}: 3 problems, listed above"),
            details: vec![
                "it has no `check` script".to_owned(),
                "it has a `precheck` script, which the checks would not run".to_owned(),
                "it has a `postcheck` script, which the checks would not run".to_owned(),
            ],
            usage: false,
        }),
        "every problem"
    );
    assert!(
        package_check_gate("a/package.json", "{", &checks).is_err_and(|failure| failure
            .summary
            .starts_with("a/package.json: invalid manifest: ")),
        "an invalid manifest"
    );
}

#[test]
fn reads_the_manifest_of_every_package_for_the_package_check_gate() {
    let step = [Step {
        name: "Package checks".to_owned(),
        action: Action::PackageCheckGate,
    }];
    let mut fake = passing();
    assert_eq!(run_steps(&step, &mut fake, &mut Vec::new()), Ok(()));
    for package in crate::PACKAGES {
        assert!(
            fake.ran
                .contains(&format!("read {}/package.json", package.dir)),
            "{} read",
            package.dir
        );
    }
    let replaced = fake.files.insert(
        "apps/roadmap/package.json".to_owned(),
        r#"{"name": "@poyetik/roadmap", "scripts": {"check": "pnpm test"}}"#.to_owned(),
    );
    assert!(replaced.is_some(), "a manifest replaced");
    assert_eq!(
        run_steps(&step, &mut fake, &mut Vec::new()),
        failed(
            "Package checks: apps/roadmap/package.json: the checks run and time `pnpm typecheck \
             && pnpm lint && pnpm test && pnpm build && pnpm test:e2e` (`Package::checks` in \
             xtask's workspace.rs, which changes with the manifest): its `check` script runs \
             `pnpm test`"
        )
    );
    let mut unreadable = passing();
    unreadable.broken = Some("read tooling/config/package.json".to_owned());
    assert_eq!(
        run_steps(&step, &mut unreadable, &mut Vec::new()),
        failed("Package checks: cannot read tooling/config/package.json: not UTF-8"),
        "an unreadable manifest"
    );
}
