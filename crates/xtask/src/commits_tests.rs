//! Tests of commit messages: their types, scopes and CI markers, and the commit-msg gate.

use crate::tests::{failed, passing, run_cli};
use crate::{COMMIT_HEADER_MAX, commit_msg_gate};

#[test]
fn accepts_conventional_commit_messages() {
    for message in [
        "feat: add the plan",
        "fix(audio-core): keep box state across blocks\n\nBody text.\nCo-Authored-By: x",
        "refactor(xtask)!: rename the gate",
        "build(audio-wasm): bump the format",
        "chore(roadmap): tick",
        "docs(deep-review): add a check",
        "test(config): cover",
        "docs!: rewrite the readme",
        "# comment from git\nci: run on every push\n# another comment\n",
    ] {
        assert_eq!(commit_msg_gate(message), Ok(()), "{message:?}");
    }
    let header = format!("feat: {}", "x".repeat(COMMIT_HEADER_MAX - 6));
    assert_eq!(
        commit_msg_gate(&header),
        Ok(()),
        "header of exactly the maximum length"
    );
    for kind in [
        "feat", "fix", "docs", "test", "refactor", "perf", "build", "ci", "chore", "style",
        "revert",
    ] {
        assert_eq!(
            commit_msg_gate(&format!("{kind}: x")),
            Ok(()),
            "{kind} is allowed"
        );
    }
}

#[test]
fn rejects_other_commit_messages() {
    let expected = |reason: &str| {
        failed(format!(
            "invalid commit message: {reason}\nexpected: <type>(<scope>)!: <summary>, type one of feat, fix, docs, test, refactor, perf, build, ci, chore, style, revert"
        ))
    };
    let long = format!("feat: {}", "x".repeat(COMMIT_HEADER_MAX - 5));
    let unknown = |scope: &str| {
        format!(
            "unknown scope \"{scope}\": name a crate, a package or a scope of xtask's \
             COMMIT_SCOPE_EXTRAS (audio-core, audio-boxes, audio-patch, audio-hash, audio-host, \
             visual-core, visual-boxes, visual-hash, visual-testkit, visual-cli, visual-host, visual-gpu, \
             visual-wasm, \
             alloc-counter, fpenv, audio-battery, audio-testkit, audio-cli, \
             audio-wasm, coordinator, web-page, xtask, config, oracle, playground, roadmap, wasm-build, spec, \
             conformance, ci, build, release, changelog, glossary, deep-review)"
        )
    };
    let cases: [(&str, String); 18] = [
        ("refactor(wasm): rename", unknown("wasm")),
        ("feat(core)!: add", unknown("core")),
        ("docs(bindings): x", unknown("bindings")),
        ("fix(x2): y", unknown("x2")),
        ("", "missing \"<type>: \" prefix".to_owned()),
        (
            "Merge whatever I want",
            "missing \"<type>: \" prefix".to_owned(),
        ),
        (
            "Merge branch 'main' into feature",
            "missing \"<type>: \" prefix".to_owned(),
        ),
        (
            "Revert \"anything goes",
            "missing \"<type>: \" prefix".to_owned(),
        ),
        ("Add the plan", "missing \"<type>: \" prefix".to_owned()),
        ("feature: add", "unknown type \"feature\"".to_owned()),
        ("Feat: add", "unknown type \"Feat\"".to_owned()),
        (
            "feat(): add",
            "the scope must be lowercase letters, digits or hyphens".to_owned(),
        ),
        (
            "feat(Core): add",
            "the scope must be lowercase letters, digits or hyphens".to_owned(),
        ),
        (
            "feat(core: add",
            "the scope must be lowercase letters, digits or hyphens".to_owned(),
        ),
        (
            "feat:  ",
            "the summary must be non-empty and must not end with a period".to_owned(),
        ),
        (
            "feat: add the plan.",
            "the summary must be non-empty and must not end with a period".to_owned(),
        ),
        (
            long.as_str(),
            format!("the header is longer than {COMMIT_HEADER_MAX} characters"),
        ),
        (
            "feat: add\nbody right away",
            "a blank line must separate the header from the body".to_owned(),
        ),
    ];
    for (message, reason) in cases {
        assert_eq!(commit_msg_gate(message), expected(&reason), "{message:?}");
    }
}

#[test]
fn cli_checks_commit_messages() {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let args = ["commit-msg".to_owned(), "COMMIT_EDITMSG".to_owned()];
    assert_eq!(
        run_cli(&args, &mut passing(), &mut out, &mut err),
        0,
        "a valid message"
    );
    assert!(out.is_empty() && err.is_empty(), "silent on success");

    let mut bad = passing();
    let replaced = bad
        .files
        .insert("COMMIT_EDITMSG".to_owned(), "wip".to_owned());
    assert!(replaced.is_some(), "message replaced");
    let (mut bad_out, mut bad_err) = (Vec::new(), Vec::new());
    assert_eq!(
        run_cli(&args, &mut bad, &mut bad_out, &mut bad_err),
        1,
        "an invalid message"
    );
    assert_eq!(
        String::from_utf8(bad_err).expect("UTF-8"),
        "error: invalid commit message: missing \"<type>: \" prefix\n",
        "one line on stderr"
    );
    assert_eq!(
        String::from_utf8(bad_out).expect("UTF-8"),
        "expected: <type>(<scope>)!: <summary>, type one of feat, fix, docs, test, refactor, \
         perf, build, ci, chore, style, revert\n",
        "the expected form on stdout"
    );

    let mut missing_err = Vec::new();
    let missing = ["commit-msg".to_owned(), "nowhere".to_owned()];
    assert_eq!(
        run_cli(&missing, &mut passing(), &mut Vec::new(), &mut missing_err),
        1,
        "a missing file"
    );
    assert_eq!(
        missing_err, b"error: cannot read nowhere: missing\n",
        "named on stderr"
    );
}

#[test]
fn confines_the_ci_marker_to_a_line_of_its_own() {
    let ok = "ci: rerun the check\n\nAsked by the maintainer.\n\n[ci]\n";
    assert_eq!(commit_msg_gate(ok), Ok(()), "alone on a line");
    assert_eq!(
        crate::ci_marker("docs: x\n\n  [ci]  \n"),
        None,
        "spaces around it"
    );
    assert_eq!(
        crate::ci_marker("docs: x\n\nno marker at all\n"),
        None,
        "none"
    );
    let expected = Some(
        "invalid commit message: [ci] asks CI to run, and `[ci <part> <part>…]` some parts of it (`cargo xtask list --full --parts` names them, in their order, final apart); write one alone on a line, or none at all"
            .to_owned(),
    );
    for message in [
        "docs: x\n\n[CI]\n",
        "docs: x\n\n [Ci] \n",
        "docs: x\n\n[cı]\n",
    ] {
        assert_eq!(
            crate::ci_marker(message),
            None,
            "GitHub reads the marker in any case: {message:?}"
        );
    }
    for message in [
        "docs: x\n\nA push runs CI when it says [ci] in its message.\n",
        "docs: run [ci] now\n",
        "docs: x\n\n[ci] please\n",
        "docs: x\n\nA push runs CI when it says [CI] in its message.\n",
        "docs: run [Ci] now\n",
        "docs: x\n\n[cI] please\n",
        "docs: x\n\nsee [cı] here\n",
    ] {
        assert_eq!(crate::ci_marker(message), expected, "{message:?}");
        assert!(commit_msg_gate(message).is_err(), "{message:?}");
    }
    let record = |hash: &str, message: &str| format!("\u{1e}{hash}\u{1f}{message}");
    let sentence = "docs: x\n\nIt runs with [ci] in the message.\n";
    assert!(
        crate::history_gate(
            &record("1111111111111111111111111111111111111111", sentence),
            "",
            ""
        )
        .is_err(),
        "the history refuses it too"
    );
    assert_eq!(
        crate::history_gate(&record(crate::CI_MARKER_EXEMPT[0], sentence), "", ""),
        Ok(()),
        "except the published commit that predates the rule"
    );
}

/// The parts named by `names`, separated by spaces, as the matrix names them.
fn parts_named(names: &str) -> Vec<crate::Part> {
    names
        .split(' ')
        .map(|name| {
            crate::matrix_parts()
                .into_iter()
                .find(|part| part.name() == name)
                .expect("a part of the matrix")
        })
        .collect()
}

#[test]
fn writes_the_ci_marker_of_some_parts_in_one_form() {
    assert_eq!(
        crate::ci_parts_marker(&crate::matrix_parts()),
        "[ci]",
        "every part"
    );
    assert_eq!(crate::ci_parts_marker(&[]), "[ci]", "none: every part");
    assert_eq!(
        crate::ci_parts_marker(&parts_named("bindings-audio-wasm gates")),
        "[ci gates bindings-audio-wasm]",
        "in the order of the matrix, one space apart"
    );
    assert_eq!(
        crate::ci_parts_marker(&[crate::Part::Final, crate::Part::Coverage]),
        "[ci coverage]",
        "the final part is no part of the matrix"
    );
}

#[test]
fn reads_parts_named_by_hand_in_any_order_each_once() {
    assert_eq!(
        crate::named_parts(" bindings-audio-wasm \t rust\n"),
        Ok(parts_named("rust bindings-audio-wasm")),
        "in the order of the matrix, whatever the spaces"
    );
    assert_eq!(
        crate::named_parts("final"),
        Err(
            "unknown part \"final\" (`cargo xtask list --full --parts` lists them, final apart)"
                .to_owned()
        ),
        "the final part runs only after every part"
    );
    assert_eq!(
        crate::named_parts("rust Rust"),
        Err(
            "unknown part \"Rust\" (`cargo xtask list --full --parts` lists them, final apart)"
                .to_owned()
        ),
        "names as the matrix writes them"
    );
    assert_eq!(
        crate::named_parts("gates coverage gates"),
        Err("part \"gates\" named twice".to_owned())
    );
    assert_eq!(crate::named_parts("  "), Err("no part named".to_owned()));
}

#[test]
fn accepts_a_marker_of_some_parts_only_in_its_one_form() {
    for message in [
        "ci: x\n\n[ci gates bindings-audio-wasm]\n",
        "ci: x\n\n  [ci tooling-oracle]  \n",
        "ci: x\n\n[CI GATES BINDINGS-AUDIO-WASM]\n",
        "ci: x\n\n[ci gates]\n\nAgain:\n[ci gates]\n",
        "ci: x\n\n[cite] a source, [circa] then\n",
    ] {
        assert_eq!(crate::ci_marker(message), None, "{message:?}");
    }
    let refused = Some(
        "invalid commit message: [ci] asks CI to run, and `[ci <part> <part>…]` some parts of it (`cargo xtask list --full --parts` names them, in their order, final apart); write one alone on a line, or none at all"
            .to_owned(),
    );
    for message in [
        "ci: x\n\n[ci bindings-audio-wasm gates]\n",
        "ci: x\n\n[ci gates  bindings-audio-wasm]\n",
        "ci: x\n\n[ci nope]\n",
        "ci: x\n\n[ci final]\n",
        "ci: x\n\n[ci gates gates]\n",
        "ci: x\n\n[ci ]\n",
        "ci: x\n\n[ci gates\n",
        "ci: x\n\nrun [ci gates] now\n",
    ] {
        assert_eq!(crate::ci_marker(message), refused, "{message:?}");
        assert!(commit_msg_gate(message).is_err(), "{message:?}");
    }
    let every: String = crate::matrix_parts()
        .into_iter()
        .map(crate::Part::name)
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(
        crate::ci_marker(&format!("ci: x\n\n[ci {every}]\n")),
        refused,
        "every part is written [ci]"
    );
    assert_eq!(
        crate::ci_marker("ci: x\n\n[ci]\n[ci gates]\n"),
        Some("invalid commit message: its CI markers ask for different parts".to_owned()),
        "two markers, two sets of parts"
    );
}

#[test]
fn reads_the_parts_a_run_of_the_workflow_checks() {
    let every = crate::matrix_parts();
    assert_eq!(
        crate::ci_parts("pull_request", "", "gates"),
        Ok(every.clone()),
        "a pull request checks every part"
    );
    assert_eq!(
        crate::ci_parts("workflow_dispatch", "", " "),
        Ok(every.clone()),
        "by hand, no part given"
    );
    assert_eq!(
        crate::ci_parts("workflow_dispatch", "[ci gates]", "coverage rust"),
        Ok(parts_named("rust coverage")),
        "by hand, the parts given, the message not read"
    );
    assert_eq!(
        crate::ci_parts("workflow_dispatch", "", "nope"),
        Err(
            "unknown part \"nope\" (`cargo xtask list --full --parts` lists them, final apart)"
                .to_owned()
        )
    );
    assert_eq!(
        crate::ci_parts("push", "ci: x\n\n[ci]\n", "gates"),
        Ok(every),
        "a push asking for every part, the input not read"
    );
    assert_eq!(
        crate::ci_parts("push", "ci: x\n\n[ci gates bindings-audio-wasm]\n", ""),
        Ok(parts_named("gates bindings-audio-wasm"))
    );
    assert_eq!(
        crate::ci_parts("push", "docs: x\n", ""),
        Ok(Vec::new()),
        "a push asking for nothing"
    );
    assert!(
        crate::ci_parts("push", "ci: x\n\n[ci bindings-audio-wasm gates]\n", "").is_err(),
        "a marker in another form"
    );
    assert_eq!(
        crate::ci_parts("schedule", "", ""),
        Err("the workflow does not run on \"schedule\"".to_owned())
    );
}

#[test]
fn writes_the_parts_as_the_outputs_of_the_first_job() {
    assert_eq!(
        crate::ci_outputs(&parts_named("gates bindings-audio-wasm")),
        [
            "parts=[\"gates\",\"bindings-audio-wasm\"]".to_owned(),
            "full=false".to_owned()
        ]
    );
    assert_eq!(
        crate::ci_outputs(&[]),
        ["parts=[]".to_owned(), "full=false".to_owned()],
        "none: no job of the matrix"
    );
    let [parts, full] = crate::ci_outputs(&crate::matrix_parts());
    assert!(
        parts.starts_with("parts=[\"rust\",\"coverage\",\"mutation-0\","),
        "{parts}"
    );
    assert!(
        parts.ends_with(",\"apps-roadmap\",\"tooling-wasm-build\",\"bindings-visual-wasm\"]"),
        "{parts}"
    );
    assert_eq!(full, "full=true", "every part: the final part runs");
}

#[test]
fn cli_prints_the_parts_of_a_run_or_why_not() {
    let run = |list: &[&str]| {
        let args: Vec<String> = list.iter().map(|arg| (*arg).to_owned()).collect();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run_cli(&args, &mut passing(), &mut out, &mut err);
        (
            code,
            String::from_utf8(out).expect("UTF-8"),
            String::from_utf8(err).expect("UTF-8"),
        )
    };
    assert_eq!(
        run(&["ci-parts", "push", "ci: x\n\n[ci coverage]\n", ""]),
        (
            0,
            "parts=[\"coverage\"]\nfull=false\n".to_owned(),
            String::new()
        )
    );
    assert_eq!(
        run(&["ci-parts", "workflow_dispatch", "", "nope"]),
        (
            1,
            String::new(),
            "error: unknown part \"nope\" (`cargo xtask list --full --parts` lists them, final apart)\n"
                .to_owned()
        ),
        "a part that is none fails the run"
    );
    assert_eq!(
        run(&["ci-parts", "push", ""]),
        (
            2,
            String::new(),
            "error: `ci-parts` takes an event, a commit message and the parts given by hand (see: cargo xtask help)\n"
                .to_owned()
        ),
        "three arguments"
    );
}
