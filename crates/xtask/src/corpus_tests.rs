//! Tests of the visual corpus's mutation step: the commands it runs, when the default check
//! runs it, and the gate on the mutants the corpus missed.

use core::num::NonZeroUsize;

use super::{
    CORPUS_EXEMPT, CORPUS_MISSED, corpus_baseline_command, corpus_gate, corpus_mutants_command,
    corpus_reached,
};
use crate::steps::Failure;
use crate::tests::{CHANGES_MODE, FAKE_JOBS, UNTRACKED, failed, passing};
use crate::{Action, Mode, Step, run_steps};

/// The step's listing of the rendering code's sources, as the fake records it.
const LISTED: &str = "capture git ls-files -z --cached --others --exclude-standard -- crates/visual-boxes/src/ \
     crates/visual-core/src/";

/// The corpus test, unmutated, as the step runs it.
const BASELINE: &str = "cargo nextest run --package=visual-cli --test=binary -E test(=tests::checks_the_repository_corpus) --locked --config=profile.test.debug=false";

/// The step's cargo-mutants command in the fake's three jobs: a timeout of 3 × 60 s.
const MUTANTS: &str = "cargo mutants --no-config --output=mutants.out-corpus --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --cargo-arg=--config=target.'cfg(target_os = \"linux\")'.runner=[\"prlimit\", \"--as=4294967296\"] --test-tool=nextest --cargo-test-arg=--max-fail=1:immediate --cargo-test-arg=--no-tests=warn --file crates/visual-boxes/src/square.rs --file crates/visual-core/src/coverage.rs --file crates/visual-core/src/image.rs --file crates/visual-core/src/render.rs --file crates/visual-core/src/display.rs --file crates/visual-core/src/rate.rs --test-package=visual-cli --baseline=skip --timeout 180 --jobs 3 --cargo-test-arg=--test=binary --cargo-test-arg=-E --cargo-test-arg=test(=tests::checks_the_repository_corpus)";

/// A mutant the corpus missed, as cargo-mutants writes it.
const MISSED: &str = "crates/visual-core/src/image.rs:76:9: replace ImageSize::point -> Option<SubpixelPoint> with None";

/// Its review.
const REVIEW: &str = "crates/visual-core/src/image.rs: replace ImageSize::point -> Option<SubpixelPoint> with None -- traced, never rendered";

/// The step alone, in `mode`.
fn step(mode: Mode) -> Vec<Step> {
    vec![Step {
        name: "corpus".to_owned(),
        action: Action::CorpusMutants(mode),
    }]
}

/// The full mode of the fake's jobs.
const FULL: Mode = Mode::Full { jobs: FAKE_JOBS };

#[test]
fn mutates_the_rendering_code_with_the_corpus_test_alone() {
    assert_eq!(corpus_baseline_command().line(), BASELINE);
    assert_eq!(corpus_mutants_command(FAKE_JOBS).line(), MUTANTS);
    let one = corpus_mutants_command(NonZeroUsize::MIN).line();
    assert!(
        one.contains(" --timeout 60 --jobs 1 "),
        "the timeout scaled by the jobs: {one}"
    );
}

#[test]
fn runs_the_unmutated_test_then_the_mutants_then_its_gate() {
    let mut fake = passing();
    let mut out = Vec::new();
    assert_eq!(run_steps(&step(FULL), &mut fake, &mut out), Ok(()));
    assert_eq!(
        fake.ran,
        [
            LISTED.to_owned(),
            BASELINE.to_owned(),
            MUTANTS.to_owned(),
            format!("read {CORPUS_MISSED}"),
            format!("read {CORPUS_EXEMPT}"),
        ],
        "the full check lists the sources, then runs it whatever changed"
    );
}

#[test]
fn accepts_missed_mutants_its_gate_then_judges() {
    let mut missed = passing();
    missed.refuse = Some(MUTANTS.to_owned());
    assert_eq!(
        run_steps(&step(FULL), &mut missed, &mut Vec::new()),
        Ok(()),
        "cargo-mutants exits with 2 when it missed mutants: the reviewed ones pass"
    );
    let mut unreviewed = passing();
    unreviewed.refuse = Some(MUTANTS.to_owned());
    let _emptied = unreviewed
        .files
        .insert(CORPUS_EXEMPT.to_owned(), "# None.\n".to_owned());
    assert_eq!(
        run_steps(&step(FULL), &mut unreviewed, &mut Vec::new()),
        failed(format!(
            "corpus: the visual corpus must kill every mutant of the rendering code: add a \
             case that tells it apart, or review one that cannot change an image byte in \
             {CORPUS_EXEMPT}: not reviewed: crates/visual-core/src/image.rs: replace \
             ImageSize::point -> Option<SubpixelPoint> with None"
        )),
        "a survivor no review names fails the step"
    );
}

#[test]
fn fails_on_any_other_failure_of_its_commands_or_reads() {
    let mut baseline = passing();
    baseline.fail = Some(BASELINE.to_owned());
    assert_eq!(
        run_steps(&step(FULL), &mut baseline, &mut Vec::new()),
        failed(format!(
            "corpus: the unmutated corpus test: `{BASELINE}` exited with code 3"
        )),
        "a corpus test failing unmutated proves nothing of its mutants"
    );
    assert_eq!(baseline.ran, [LISTED, BASELINE], "and nothing is mutated");
    let mut timeout = passing();
    timeout.fail = Some(MUTANTS.to_owned());
    assert_eq!(
        run_steps(&step(FULL), &mut timeout, &mut Vec::new()),
        failed(format!("corpus: `{MUTANTS}` exited with code 3")),
        "a timeout (3), or any failure but missed mutants"
    );
    for path in [CORPUS_MISSED, CORPUS_EXEMPT] {
        let mut unread = passing();
        let _removed = unread.files.remove(path);
        assert_eq!(
            run_steps(&step(FULL), &mut unread, &mut Vec::new()),
            failed(format!("corpus: cannot read {path}: missing")),
            "{path}"
        );
    }
}

#[test]
fn runs_in_the_default_check_only_when_a_change_reaches_it() {
    let mut unreached = passing();
    let mut out = Vec::new();
    assert_eq!(
        run_steps(&step(CHANGES_MODE), &mut unreached, &mut out),
        Ok(())
    );
    assert_eq!(
        unreached.ran,
        [
            LISTED,
            "capture git merge-base HEAD @{upstream}",
            "capture git diff --no-color --name-only -z --no-renames abc1230000000000000000000000000000000000",
            "capture git ls-files -z --others --exclude-standard",
        ],
        "the sources, the changes and the untracked files listed, nothing run"
    );
    let said = String::from_utf8(out).expect("UTF-8");
    assert!(
        said.contains(
            "  no change since abc1230000000000000000000000000000000000 (merge base with \
             @{upstream}) reaches the visual corpus or its rendering code: not run\n"
        ),
        "said: {said}"
    );
    let mut reached = passing();
    let _untracked = reached.files.insert(
        UNTRACKED.to_owned(),
        "notes.txt\0conformance/visual/cases/new.json\0".to_owned(),
    );
    let mut reached_out = Vec::new();
    assert_eq!(
        run_steps(&step(CHANGES_MODE), &mut reached, &mut reached_out),
        Ok(())
    );
    assert_eq!(
        reached.ran.get(4..),
        Some(
            &[
                BASELINE.to_owned(),
                MUTANTS.to_owned(),
                format!("read {CORPUS_MISSED}"),
                format!("read {CORPUS_EXEMPT}"),
            ][..]
        ),
        "a new case, untracked, reaches it: run as in the full check"
    );
    let reached_said = String::from_utf8(reached_out).expect("UTF-8");
    assert!(
        reached_said
            .contains("  conformance/visual/cases/new.json changed: the corpus is mutated\n"),
        "said: {reached_said}"
    );
}

#[test]
fn fails_in_the_default_check_when_the_changes_cannot_be_listed() {
    let mut baseless = passing();
    for base in crate::MUTATION_BASES {
        let _answer = baseless
            .files
            .remove(&format!("capture git merge-base HEAD {base}"));
    }
    let baseless_verdict = run_steps(&step(CHANGES_MODE), &mut baseless, &mut Vec::new());
    assert!(
        baseless_verdict.as_ref().is_err_and(|failure| failure
            .summary
            .starts_with("corpus: no base to measure the changes against")),
        "no merge base: {baseless_verdict:?}"
    );
    for (broken, why) in [
        (
            "git diff --no-color --name-only -z --no-renames abc1230000000000000000000000000000000000",
            "cannot list the changes",
        ),
        (
            "git ls-files -z --others --exclude-standard",
            "cannot list the untracked files",
        ),
    ] {
        let mut fake = passing();
        fake.broken = Some(broken.to_owned());
        let verdict = run_steps(&step(CHANGES_MODE), &mut fake, &mut Vec::new());
        assert!(
            verdict
                .as_ref()
                .is_err_and(|failure| failure.summary.starts_with(&format!("corpus: {why}"))),
            "{broken}: {verdict:?}"
        );
    }
}

#[test]
fn takes_a_change_below_each_of_its_inputs_and_nothing_else() {
    let changed =
        |paths: &[&str]| -> Vec<String> { paths.iter().map(|&path| path.to_owned()).collect() };
    for path in [
        "crates/visual-core/src/display.rs",
        "crates/visual-boxes/src/square.rs",
        "crates/visual-hash/src/lib.rs",
        "crates/visual-cli/tests/binary.rs",
        "conformance/visual/references.json",
        "mutants-corpus-exempt.txt",
        "Cargo.toml",
        "Cargo.lock",
        ".config/nextest.toml",
        "crates/xtask/src/corpus.rs",
        "crates/xtask/src/actions.rs",
        "crates/xtask/src/changes.rs",
        "crates/xtask/src/reviews.rs",
        "crates/xtask/src/runner.rs",
        "crates/xtask/src/steps.rs",
        "crates/xtask/src/workspace.rs",
        "rust-toolchain.toml",
        ".cargo/config.toml",
    ] {
        let paths = changed(&["README.md", path, "crates/visual-core/src/lib.rs"]);
        assert_eq!(
            corpus_reached(&paths),
            Some(path),
            "{path}: the first reached"
        );
    }
    assert_eq!(
        corpus_reached(&changed(&[
            "README.md",
            "crates/visual-gpu/src/lib.rs",
            "conformance/references.json",
            "crates/xtask/src/cli.rs",
            "crates/visual-testkit/src/lib.rs",
            "spec/visual-design.md",
        ])),
        None,
        "the GPU renderer, the audio corpus, the rest of xtask, the visual test kit and the \
         specification reach none of its inputs"
    );
    assert_eq!(corpus_reached(&[]), None, "no change");
}

#[test]
fn requires_every_mutant_missed_reviewed_and_every_review_current() {
    let reviewed = format!("# A comment.\n\n{REVIEW}\n");
    assert_eq!(corpus_gate(&format!("{MISSED}\n\n"), &reviewed), Ok(()));
    let context = format!(
        "the visual corpus must kill every mutant of the rendering code: add a case that \
         tells it apart, or review one that cannot change an image byte in {CORPUS_EXEMPT}"
    );
    let refused =
        |problems: &[&str]| Err(Failure::from(format!("{context}: {}", problems.join("; "))));
    assert_eq!(
        corpus_gate(&format!("{MISSED}\n"), ""),
        refused(&[
            "not reviewed: crates/visual-core/src/image.rs: replace ImageSize::point -> \
             Option<SubpixelPoint> with None"
        ]),
        "the review list emptied: every survivor refused"
    );
    assert_eq!(
        corpus_gate("", &reviewed),
        refused(&[
            "stale review: crates/visual-core/src/image.rs: replace ImageSize::point -> \
             Option<SubpixelPoint> with None"
        ]),
        "a review of a mutant the corpus now kills is stale: the run mutates every file whole"
    );
    assert_eq!(
        corpus_gate(&format!("{MISSED}\n{MISSED}\n"), &reviewed),
        refused(&[
            "not reviewed: crates/visual-core/src/image.rs: replace ImageSize::point -> \
             Option<SubpixelPoint> with None (2 found, 1 reviewed)"
        ]),
        "each occurrence needs its review"
    );
    assert_eq!(
        corpus_gate("garbage\n", "crates/a.rs: replace f with ()\n"),
        Err(Failure {
            summary: format!("{context}: 2 problems, listed above"),
            details: vec![
                "unrecognized mutant: garbage".to_owned(),
                "review without a reason: crates/a.rs: replace f with ()".to_owned(),
            ],
            usage: false,
        }),
        "a line that is no mutant, and a review without its reason"
    );
}

#[test]
fn requires_every_source_of_the_rendering_code_mutated_or_left_out_with_a_reason() {
    // Finding F16-8 of the fourth deep review of visual block 2 (#34): exact time
    // (`rate.rs`), whose terms place the square, was never mutated against the corpus, and
    // no list said why. Every source of the rendering code's crates is now in one list.
    let listed = |extra: &[&str]| -> Vec<String> {
        crate::tests::corpus_sources()
            .into_iter()
            .chain(extra.iter().copied())
            .map(str::to_owned)
            .collect()
    };
    assert!(super::CORPUS_MUTATED.contains(&"crates/visual-core/src/rate.rs"));
    assert_eq!(super::corpus_sources_gate(&listed(&[])), Ok(()));
    assert_eq!(
        super::corpus_sources_gate(&listed(&[
            "crates/visual-core/src/time.rs",
            "crates/visual-hash/src/lib.rs",
            "crates/visual-boxes/README.md",
        ])),
        failed(
            "crates/visual-core/src/time.rs: a source of the rendering code's crates the \
             visual corpus's mutation neither mutates nor leaves out with a reason: add it to \
             CORPUS_MUTATED, or to CORPUS_NOT_MUTATED with why no image byte depends on it \
             (crates/xtask/src/corpus.rs)"
        ),
        "a new source of a crate of the rendering code; another crate and a README are not read"
    );
    let gone: Vec<String> = listed(&[])
        .into_iter()
        .filter(|path| {
            path != "crates/visual-core/src/rate.rs" && path != "crates/visual-core/src/lib.rs"
        })
        .collect();
    assert_eq!(
        super::corpus_sources_gate(&gone),
        Err(Failure {
            summary: "2 problems, listed above".to_owned(),
            details: vec![
                "crates/visual-core/src/rate.rs: listed by the visual corpus's mutation, but \
                 not tracked: remove or correct it (crates/xtask/src/corpus.rs)"
                    .to_owned(),
                "crates/visual-core/src/lib.rs: listed by the visual corpus's mutation, but \
                 not tracked: remove or correct it (crates/xtask/src/corpus.rs)"
                    .to_owned(),
            ],
            usage: false,
        }),
        "a file listed but gone, from either list"
    );
    for (path, reason) in super::CORPUS_NOT_MUTATED {
        assert!(!super::CORPUS_MUTATED.contains(&path), "{path}");
        assert!(!reason.is_empty(), "{path}");
    }
    let mut unlisted = passing();
    let _gone = unlisted.files.remove(crate::tests::CORPUS_SOURCES_LISTED);
    assert_eq!(
        run_steps(&step(FULL), &mut unlisted, &mut Vec::new()).map_err(|failure| failure.summary),
        Err("corpus: cannot list the sources of the rendering code: `git ls-files -z --cached --others --exclude-standard -- crates/visual-boxes/src/ crates/visual-core/src/` exited with code 1: failed".to_owned()),
        "the sources cannot be listed"
    );
    let mut unclassified = passing();
    let _added = unclassified.files.insert(
        crate::tests::CORPUS_SOURCES_LISTED.to_owned(),
        listed(&["crates/visual-boxes/src/new.rs"]).join("\0"),
    );
    assert_eq!(
        run_steps(&step(FULL), &mut unclassified, &mut Vec::new())
            .map_err(|failure| failure.summary),
        Err(
            "corpus: crates/visual-boxes/src/new.rs: a source of the rendering code's crates the \
             visual corpus's mutation neither mutates nor leaves out with a reason: add it to \
             CORPUS_MUTATED, or to CORPUS_NOT_MUTATED with why no image byte depends on it \
             (crates/xtask/src/corpus.rs)"
                .to_owned()
        ),
        "a source of neither list fails the step before anything runs"
    );
    assert_eq!(unclassified.ran, [LISTED]);
}

/// Every module of xtask the code of `text` names (`crate::<module>`, in a `use` group or
/// inline), read from its tokens, so that a doc comment or a string names none.
fn crate_modules(text: &str) -> alloc::collections::BTreeSet<String> {
    let tokens = crate::rust_source::tokens_of(text);
    let mut modules = alloc::collections::BTreeSet::new();
    for (at, token) in tokens.iter().enumerate() {
        let path = tokens.get(at.saturating_add(1)..at.saturating_add(3));
        if !token.is_ident("crate")
            || !path.is_some_and(|colons| colons.iter().all(|colon| colon.is_punct(':')))
        {
            continue;
        }
        let start = at.saturating_add(3);
        match tokens.get(start) {
            Some(group) if group.opens('{') => {
                let end = crate::rust_source::close_of(&tokens, start);
                let mut depth = 0_usize;
                let mut opens_path = true;
                for inner in tokens.get(start.saturating_add(1)..end).unwrap_or_default() {
                    if inner.kind == crate::rust_source::TokenKind::Open {
                        depth = depth.saturating_add(1);
                    } else if inner.kind == crate::rust_source::TokenKind::Close {
                        depth = depth.saturating_sub(1);
                    } else if inner.kind == crate::rust_source::TokenKind::Ident
                        && depth == 0
                        && opens_path
                    {
                        let _new = modules.insert(inner.text.clone());
                    }
                    opens_path = depth == 0 && inner.is_punct(',');
                }
            }
            Some(module) if module.kind == crate::rust_source::TokenKind::Ident => {
                let _new = modules.insert(module.text.clone());
            }
            _ => {}
        }
    }
    modules
}

#[test]
fn reaches_every_module_of_xtask_the_step_uses() {
    // Finding F17-8 of the fourth deep review of visual block 2 (#34): the step's inputs
    // named its own code and steps.rs, while it uses four more modules of xtask, whose
    // change the default check then did not run it for. Each module its `use crate::`
    // names is an input, read from the step's own source.
    // Finding H5-4 of the fifth deep review of visual block 2 (#34): the test read only the
    // first `use crate::{…}` block, so a second `use crate::x::y;` or an inline
    // `crate::z::w(…)` escaped it. Every `crate::<module>` of the code's tokens is read.
    assert_eq!(
        crate_modules(
            "use crate::{\n    a::b,\n    c::{d, e},\n};\nuse crate::f::g;\n/// [`crate::h`]\n\
             fn x() { crate::i::j(\"crate::k\"); let _ = crate::l::M; }\nuse crate::*;\n\
             pub(crate) fn y() {}\npub(crate)"
        ),
        ["a", "c", "f", "i", "l"].map(str::to_owned).into(),
        "every module the code names, in a group, a second use or inline; none a doc or a \
         string names"
    );
    let modules = crate_modules(include_str!("corpus.rs"));
    assert_eq!(
        modules,
        [
            "actions",
            "changes",
            "reviews",
            "runner",
            "steps",
            "workspace"
        ]
        .map(str::to_owned)
        .into()
    );
    for module in modules.iter().map(String::as_str).chain(["corpus"]) {
        let path = format!("crates/xtask/src/{module}.rs");
        assert!(
            super::CORPUS_INPUTS.contains(&path.as_str()),
            "{path}: a module the step uses"
        );
    }
}
