//! Tests of running the steps: the inputs each action reads and how its verdict is said.

use core::num::NonZeroUsize;
use core::time::Duration;
use std::io;

use crate::actions::conflict_markers;
use crate::steps::Failure;
use crate::tests::{
    Broken, CHANGES, CHANGES_MODE, FAKE_JOBS, Fake, failed, found, hermetic_git,
    hermetic_git_ending, one_line_change, parts_refused, passing, readmes, scratch, system,
};
use crate::{
    Action, Cmd, CommandFailure, Dir, Extent, Mode, OnFailure, PACKAGES, Ran, Runner, Step,
    UNVIABLE_REPORT, check_steps, run_steps, run_steps_then,
};

#[test]
fn only_a_partial_run_may_leave_no_unviable_report() {
    let gate = |extent: Extent| {
        vec![Step {
            name: "gate".to_owned(),
            action: Action::UnviableGate(extent),
        }]
    };
    let mut partial = passing();
    let _report = partial.files.remove(UNVIABLE_REPORT);
    assert_eq!(
        run_steps(&gate(Extent::Partial), &mut partial, &mut Vec::new()),
        Ok(()),
        "cargo-mutants writes no report when the changes hold no mutant"
    );
    let mut complete = passing();
    let _complete = complete.files.remove(UNVIABLE_REPORT);
    assert_eq!(
        run_steps(&gate(Extent::Complete), &mut complete, &mut Vec::new()),
        failed(format!("gate: cannot read {UNVIABLE_REPORT}: missing")),
        "a complete run always writes it"
    );
    let mut unreadable = Unreadable(passing());
    assert_eq!(
        run_steps(&gate(Extent::Partial), &mut unreadable, &mut Vec::new()),
        failed(format!("gate: cannot read {UNVIABLE_REPORT}: denied")),
        "a report that exists but cannot be read is never taken for none"
    );
}

/// Five command steps, `One` to `Five`, each running `x <its number>`.
fn five_steps() -> Vec<Step> {
    ["one", "two", "three", "four", "five"]
        .into_iter()
        .map(|number| Step {
            name: number.to_uppercase(),
            action: Action::Run(Cmd::new("x", &[number], Dir::Root)),
        })
        .collect()
}

/// A fake where the second of [`five_steps`] fails and the fourth cannot start.
fn failing_twice() -> Fake {
    Fake {
        fail: Some("x two".to_owned()),
        broken: Some("x four".to_owned()),
        ..Fake::default()
    }
}

#[test]
fn runs_every_step_then_reports_every_failure_when_asked() {
    let mut fake = failing_twice();
    let mut out = Vec::new();
    assert_eq!(
        run_steps_then(&five_steps(), &mut fake, &mut out, OnFailure::RunTheRest),
        failed(
            "2 of 5 checks failed, listed above\n\
             TWO: `x two` exited with code 3\n\
             FOUR: cannot run `x four`: not found"
        ),
        "both failures, each with its step and its own error"
    );
    assert_eq!(
        fake.ran,
        ["x one", "x two", "x three", "x four", "x five"],
        "every step ran"
    );
    assert_eq!(
        String::from_utf8(out).expect("UTF-8"),
        "▶ [1/5] ONE\n  done in 1.5 s\n▶ [2/5] TWO\n  failed after 1.5 s\n▶ [3/5] THREE\n  done in \
         1.5 s\n▶ [4/5] FOUR\n  failed after 1.5 s\n▶ [5/5] FIVE\n  done in 1.5 s\n\
         ✘ 2 of 5 checks failed in 7.5 s\n",
        "no claim that the checks passed"
    );
    let mut passing = Fake::default();
    let mut progress = Vec::new();
    assert_eq!(
        run_steps_then(
            &five_steps(),
            &mut passing,
            &mut progress,
            OnFailure::RunTheRest
        ),
        Ok(()),
        "no failure"
    );
    assert!(
        String::from_utf8(progress)
            .expect("UTF-8")
            .ends_with("▶ [5/5] FIVE\n  done in 1.5 s\n✔ all 5 checks passed in 7.5 s\n"),
        "passed"
    );
}

#[test]
fn shows_tenths_below_a_minute_then_whole_seconds() {
    let cases = [
        (0_u64, "0.0 s"),
        (99, "0.0 s"),
        (100, "0.1 s"),
        (1_250, "1.2 s"),
        (59_999, "59.9 s"),
        (60_000, "1 min 00 s"),
        (61_999, "1 min 01 s"),
        (754_000, "12 min 34 s"),
        (3_599_999, "59 min 59 s"),
        (3_600_000, "1 h 00 min 00 s"),
        (3_725_000, "1 h 02 min 05 s"),
        (45_296_000, "12 h 34 min 56 s"),
    ];
    for (millis, text) in cases {
        assert_eq!(
            crate::duration_text(Duration::from_millis(millis)),
            text,
            "{millis} ms"
        );
    }
}

#[test]
fn stops_at_the_first_failure_unless_asked_to_run_the_rest() {
    let mut fake = failing_twice();
    let mut out = Vec::new();
    assert_eq!(
        run_steps_then(&five_steps(), &mut fake, &mut out, OnFailure::Stop),
        failed("TWO: `x two` exited with code 3"),
        "the first failure alone"
    );
    assert_eq!(fake.ran, ["x one", "x two"], "nothing after it");
    assert_eq!(
        String::from_utf8(out).expect("UTF-8"),
        "▶ [1/5] ONE\n  done in 1.5 s\n▶ [2/5] TWO\n  failed after 1.5 s\n\
         ✘ stopped after 3.0 s, 2 of 5 checks run\n",
        "the time of the failed step said too, then the total"
    );
    let mut stopped = failing_twice();
    assert_eq!(
        run_steps(&five_steps(), &mut stopped, &mut Vec::new()),
        failed("TWO: `x two` exited with code 3"),
        "run_steps stops"
    );
    assert_eq!(
        CHANGES_MODE.on_failure(),
        OnFailure::Stop,
        "the default check stops, for fast feedback"
    );
    assert_eq!(
        Mode::Full { jobs: FAKE_JOBS }.on_failure(),
        OnFailure::RunTheRest,
        "the full check runs every step"
    );
}

#[test]
fn reports_a_broken_output() {
    assert_eq!(
        run_steps(&check_steps(CHANGES_MODE), &mut passing(), &mut Broken),
        failed("cannot write to stdout: closed"),
        "cannot even announce a step"
    );
    let one = vec![check_steps(CHANGES_MODE).remove(0)];
    let mut limited = Limited(1);
    assert_eq!(
        run_steps(&one, &mut passing(), &mut limited),
        failed("cannot write to stdout: closed"),
        "cannot write the summary"
    );
}

#[test]
fn reports_rust_files_it_cannot_list() {
    for (name, action) in [
        (
            "Unsafe code confined to the audited files",
            Action::UnsafeGate,
        ),
        (
            "Every root of both libraries denies its lints; every `no_std` crate built for every portability target or listed",
            Action::LibraryLintsGate,
        ),
        ("Test code only in tests/", Action::TestCodeGate),
        ("Test data declared", Action::TestDataGate),
        (
            "Rust mutation testing of the changes",
            Action::RustMutantsOfChanges {
                jobs: NonZeroUsize::MIN,
            },
        ),
    ] {
        let mut fake = passing();
        fake.broken = Some("list *.rs".to_owned());
        // A change to test code, which widens the mutation testing to the crate, whose
        // sources are then listed.
        let _changes = fake.files.insert(
            format!("capture {}", CHANGES.0),
            one_line_change("crates/audio-core/src/lib_tests.rs"),
        );
        let step = Step {
            name: name.to_owned(),
            action,
        };
        assert_eq!(
            run_steps(&[step], &mut fake, &mut Vec::new()),
            failed(format!(
                "{name}: cannot list the Rust files: `git ls-files -- *.rs` exited with code 128: fatal: not a git repository"
            )),
            "named"
        );
    }
}

#[test]
fn requires_every_part_to_be_described() {
    let readmes = readmes();
    let mut parts: Vec<String> = PACKAGES
        .iter()
        .map(|package| package.dir.to_owned())
        .collect();
    assert_eq!(crate::readme_gate(&parts, &readmes), Ok(()), "described");
    parts.push("crates/audio-core".to_owned());
    parts.push("lonely".to_owned());
    let without: Vec<(String, String)> = readmes
        .iter()
        .filter(|(path, _)| {
            !["tooling/oracle/README.md", "crates/README.md", "README.md"].contains(&path.as_str())
        })
        .cloned()
        .chain([
            ("crates/README.md".to_owned(), "| `audio-hash` |".to_owned()),
            (
                "README.md".to_owned(),
                "| `apps/` | | `bindings/` | | `tooling/` |".to_owned(),
            ),
        ])
        .collect();
    assert_eq!(
        crate::readme_gate(&parts, &without),
        failed(
            "parts not described: 4 problems, listed above\n\
             tooling/oracle has no README.md\n\
             crates/README.md does not name audio-core\n\
             lonely is not inside a folder of parts (apps/, crates/ …)\n\
             README.md does not name crates/"
        ),
        "a package without a README, a crate the table forgets, a part outside any folder"
    );
    let unnamed: Vec<(String, String)> = readmes
        .iter()
        .filter(|(path, _)| path != "apps/README.md")
        .cloned()
        .collect();
    assert_eq!(
        crate::readme_gate(&PACKAGES.map(|package| package.dir.to_owned()), &unnamed),
        found("parts not described", &["apps has no README.md".to_owned()]),
        "a folder without its README, said once for its two parts"
    );
}

/// A fake whose work tree is clean at `head`, and where each of `objects` peels to its
/// commit.
fn pushing(head: &str, objects: &[(&str, &str)]) -> Fake {
    let mut fake = passing();
    for (command, answer) in
        core::iter::once(("git rev-parse HEAD".to_owned(), format!("{head}\n"))).chain(
            objects.iter().map(|(object, commit)| {
                (
                    format!("git rev-parse --verify {object}^{{commit}}"),
                    format!("{commit}\n"),
                )
            }),
        )
    {
        let fresh = fake.files.insert(format!("capture {command}"), answer);
        assert!(fresh.is_none(), "fresh fake");
    }
    fake
}

#[test]
fn compares_the_commits_pushed_not_their_tags() {
    let head = "1111111111111111111111111111111111111111";
    let tag = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let other = "2222222222222222222222222222222222222222";
    let deleted = "0000000000000000000000000000000000000000";
    let push = |fake: &mut Fake, objects: &[&str]| {
        let pushed: Vec<String> = objects.iter().map(|&object| object.to_owned()).collect();
        crate::actions::pre_push(fake, &pushed)
    };
    let mut on_head = pushing(head, &[(tag, head)]);
    assert_eq!(
        push(&mut on_head, &[tag, deleted]),
        Ok(()),
        "an annotated tag of the commit checked out, a deleted branch"
    );
    assert!(
        !on_head.ran.iter().any(|line| line.contains(deleted)),
        "a deleted branch is not resolved"
    );
    assert_eq!(
        push(&mut pushing(head, &[(tag, other)]), &[tag]),
        failed(format!(
            "the push cannot be checked: pushing {other}, but {head} is checked out: check out the pushed commit, then push again"
        )),
        "a tag of another commit, named by its commit"
    );
    assert_eq!(
        push(&mut pushing(head, &[]), &[tag]),
        failed(format!(
            "cannot resolve the pushed {tag}: `git rev-parse --verify {tag}^{{commit}}` exited with code 1: failed"
        )),
        "an object that is not a commit"
    );
}

#[test]
fn pushes_an_annotated_tag_of_the_commit_checked_out() {
    let repo = scratch("tag");
    let _init = hermetic_git(&repo, &["init", "--quiet"]);
    for message in ["first", "second"] {
        let _commit = hermetic_git(
            &repo,
            &["commit", "--quiet", "--allow-empty", "-m", message],
        );
    }
    let _here = hermetic_git(&repo, &["tag", "-a", "here", "-m", "on HEAD"]);
    let _before = hermetic_git(&repo, &["tag", "-a", "before", "-m", "on HEAD~1", "HEAD~1"]);
    let object = |name: &str| hermetic_git(&repo, &["rev-parse", name]).trim().to_owned();
    let (here, before, parent) = (object("here"), object("before"), object("HEAD~1"));
    assert_ne!(here, object("HEAD"), "a tag object, not the commit");
    let mut runner = system(repo.path());
    let on_head = crate::actions::pre_push(&mut runner, core::slice::from_ref(&here));
    let elsewhere = crate::actions::pre_push(&mut runner, &[before]);
    let _hidden = hermetic_git(&repo, &["config", "status.showUntrackedFiles", "no"]);
    std::fs::create_dir_all(repo.join("src")).expect("folder");
    std::fs::write(repo.join("src/gain.rs"), "").expect("untracked file");
    let untracked = crate::actions::pre_push(&mut runner, &[here]);
    assert_eq!(
        untracked,
        failed(
            "the push cannot be checked: the working tree has changes not committed, which the \
             check would include: ?? src/gain.rs"
        ),
        "a file never added, which the user's configuration hides from `git status`"
    );
    assert_eq!(on_head, Ok(()), "the tag of the commit checked out");
    assert!(
        elsewhere.is_err_and(|error| error.summary.contains(&format!("pushing {parent}, but "))),
        "the tag of another commit, named by the commit"
    );
}

#[test]
fn fails_when_the_scope_gate_cannot_read_its_inputs() {
    let step = [Step {
        name: "Scope".to_owned(),
        action: Action::TypeScriptScopeGate(PACKAGES[7]),
    }];
    assert_eq!(
        run_steps(&step, &mut passing(), &mut Vec::new()),
        Ok(()),
        "the fake passes"
    );
    let mut fake = passing();
    fake.broken = Some("list *.ts *.tsx *.mts *.cts *.js *.jsx *.mjs *.cjs *.astro".to_owned());
    assert_eq!(
        run_steps(&step, &mut fake, &mut Vec::new()),
        failed(
            "Scope: cannot list the TypeScript sources: `git ls-files -- *.ts *.tsx *.mts *.cts \
             *.js *.jsx *.mjs *.cjs *.astro` exited with code 128: fatal: not a git repository"
        ),
        "the sources cannot be listed"
    );
    for missing in [
        "apps/roadmap/stryker.config.json",
        "apps/roadmap/coverage/coverage-summary.json",
        "apps/roadmap/reports/mutation/mutation.json",
        crate::BYPASSES_REVIEWED,
    ] {
        let mut lacking = passing();
        let removed = lacking.files.remove(missing);
        assert!(removed.is_some(), "{missing} removed");
        assert_eq!(
            run_steps(&step, &mut lacking, &mut Vec::new()),
            failed(format!("Scope: cannot read {missing}: missing")),
            "{missing} cannot be read"
        );
    }
}

#[test]
fn knows_a_deletion_by_its_all_zero_object_name() {
    for (object, deletion) in [
        ("0".repeat(40), true),
        ("0".repeat(64), true),
        (String::new(), false),
        ("0".repeat(39), false),
        ("0".repeat(41), false),
        ("0".repeat(63), false),
        (format!("{}1", "0".repeat(39)), false),
    ] {
        assert_eq!(crate::actions::is_deletion(&object), deletion, "{object:?}");
    }
}

#[test]
fn checks_the_parts_of_the_full_check_as_a_step() {
    assert_eq!(
        crate::actions::parts_of_the_full_check(crate::full_parts()),
        Ok(()),
        "every part"
    );
    let without_miri: Vec<crate::Part> = crate::full_parts()
        .into_iter()
        .filter(|part| *part != crate::Part::Miri)
        .collect();
    assert_eq!(
        crate::actions::parts_of_the_full_check(without_miri),
        parts_refused(&["`Unsafe code under Miri` runs in no part"]),
        "a part left out"
    );
}

/// A runner whose files all exist but cannot be read.
struct Unreadable(Fake);

impl Runner for Unreadable {
    fn run(&mut self, cmd: &Cmd) -> Result<(), CommandFailure> {
        self.0.run(cmd)
    }
    fn read(&mut self, _path: &str) -> io::Result<String> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"))
    }
    fn fresh_dir(&mut self, path: &str) -> io::Result<()> {
        self.0.fresh_dir(path)
    }
    fn tracked(&mut self, patterns: &[&str]) -> Result<Vec<String>, CommandFailure> {
        self.0.tracked(patterns)
    }
    fn capture(&mut self, cmd: &Cmd) -> Result<String, CommandFailure> {
        self.0.capture(cmd)
    }
    fn run_for_stdout(&mut self, cmd: &Cmd) -> Ran {
        self.0.run_for_stdout(cmd)
    }
    fn write(&mut self, path: &str, text: &str) -> io::Result<()> {
        self.0.write(path, text)
    }
    fn parallelism(&mut self) -> NonZeroUsize {
        self.0.parallelism()
    }
    fn environment(&mut self) -> crate::Environment {
        self.0.environment()
    }
    fn clock(&mut self) -> Duration {
        self.0.clock()
    }
}

/// An output that accepts `n` complete lines, then fails.
struct Limited(usize);

impl io::Write for Limited {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.0 == 0 {
            return Err(io::Error::other("closed"));
        }
        if buf.contains(&b'\n') {
            self.0 = self.0.saturating_sub(1);
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn refuses_the_conflict_markers_a_real_merge_left_in_each_style() {
    for (style, lines) in [
        ("merge", [1_u8, 3, 5].as_slice()),
        ("zdiff3", &[1, 3, 5, 7]),
    ] {
        let repo = scratch(&format!("conflict-{style}"));
        let _init = hermetic_git(&repo, &["init", "--quiet", "--initial-branch=main"]);
        let write =
            |path: &str, bytes: &[u8]| std::fs::write(repo.join(path), bytes).expect("a file");
        write(".gitattributes", b"* text=auto eol=lf\n*.wasm binary\n");
        // A binary file that holds a marker line: declared binary, never read.
        write("engine.wasm", b"\xff\x00\n<<<<<<< not text\n");
        write("gain.rs", b"base\n");
        let commit = |message: &str| {
            let _added = hermetic_git(&repo, &["add", "--all"]);
            let _committed = hermetic_git(&repo, &["commit", "--quiet", "-m", message]);
        };
        commit("base");
        let _branch = hermetic_git(&repo, &["checkout", "--quiet", "-b", "side"]);
        write("gain.rs", b"side\n");
        commit("side");
        let _back = hermetic_git(&repo, &["checkout", "--quiet", "main"]);
        write("gain.rs", b"main\n");
        commit("main");
        let mut runner = system(repo.path());
        assert_eq!(
            conflict_markers(&mut runner),
            Ok(()),
            "{style}: no conflict yet"
        );
        let (merged, _) = hermetic_git_ending(
            &repo,
            &[
                "-c",
                &format!("merge.conflictStyle={style}"),
                "merge",
                "--quiet",
                "side",
            ],
        );
        assert!(!merged, "{style}: the merge conflicts");
        // The honest mistake: the conflict committed unresolved.
        commit("merged");
        assert_eq!(
            conflict_markers(&mut runner),
            Err(Failure {
                summary: format!("{} problems, listed above", lines.len()),
                details: lines
                    .iter()
                    .map(|line| format!("gain.rs:{line}: a conflict marker git left, unresolved"))
                    .collect(),
                usage: false,
            }),
            "{style}: every marker git wrote, none in the binary file"
        );
        // A binary file the attributes do not declare cannot be read as text.
        write("logo.png", b"\xff\xfe");
        commit("logo");
        let undeclared = conflict_markers(&mut runner).expect_err("a binary file not declared");
        assert!(
            undeclared.summary.starts_with("cannot read logo.png: ")
                && undeclared.summary.ends_with(
                    " (a binary file is declared `*.<extension> binary` in .gitattributes)"
                ),
            "{style}: says how to declare it: {}",
            undeclared.summary
        );
    }
}

#[test]
fn says_why_it_cannot_read_the_attributes_or_list_the_files() {
    let mut missing = passing();
    let _removed = missing.files.remove(crate::GIT_ATTRIBUTES);
    assert!(
        conflict_markers(&mut missing)
            .is_err_and(|failure| failure.summary.starts_with("cannot read .gitattributes: ")),
        "no attributes: nothing tells the binary files apart"
    );
    let mut unreadable = passing();
    let _replaced = unreadable.files.insert(
        crate::GIT_ATTRIBUTES.to_owned(),
        "*.{png,jpg} binary\n".to_owned(),
    );
    assert_eq!(
        conflict_markers(&mut unreadable),
        failed(
            ".gitattributes:1: declares files binary in another form than `*.<extension> \
             binary`, which the conflict marker gate reads"
        ),
        "a form the gate cannot read, refused rather than skipped"
    );
    let mut unlisted = passing();
    unlisted.broken = Some("list *".to_owned());
    assert_eq!(
        conflict_markers(&mut unlisted),
        failed(
            "cannot list the tracked files: `git ls-files -- *` exited with code 128: fatal: \
             not a git repository"
        ),
        "git cannot list the files"
    );
    assert_eq!(
        conflict_markers(&mut passing()),
        Ok(()),
        "the fake's files hold none"
    );
}

/// A cargo-mutants run whose report is [`crate::MUTANTS_OUT`], as a step, and its
/// doc-test pass on `missed`.
fn mutants_run(missed: &[&str]) -> (Vec<Step>, Cmd, Cmd) {
    let first = Cmd::new(
        "cargo",
        &[
            "mutants",
            "--output=.",
            crate::MUTANT_TEST_TOOL,
            crate::MUTANT_FAIL_FAST,
            "--",
            "--",
            crate::INCLUDE_IGNORED,
        ],
        Dir::Root,
    );
    let pass = crate::steps::doc_test_pass_command(&first, crate::MUTANTS_OUT, missed);
    let step = Step {
        name: "m".to_owned(),
        action: Action::RustMutants(first.clone()),
    };
    (vec![step], first, pass)
}

#[test]
fn tests_again_with_the_doc_tests_what_nextest_missed_and_nothing_else() {
    let names = [
        "crates/audio-core/src/block.rs:61:5: replace engine_block with ()",
        "crates/audio-core/src/math.rs:9:5: replace f -> u32 with 0",
    ];
    let missed_file = format!("{}/missed.txt", crate::MUTANTS_OUT);
    let (steps, first, pass) = mutants_run(&names);
    let missing = |fake: &mut Fake| {
        fake.refuse = Some(first.line());
        let _missed = fake
            .files
            .insert(missed_file.clone(), format!("{}\n{}\n", names[0], names[1]));
    };
    let mut clean = passing();
    assert_eq!(run_steps(&steps, &mut clean, &mut Vec::new()), Ok(()));
    let baseline = format!("read {}/log/baseline.log", crate::MUTANTS_OUT);
    assert_eq!(
        clean.ran,
        [first.line(), baseline.clone()],
        "nothing missed: one run, then its baseline's times read for the test order"
    );
    let caught_file = format!("{}/doc-tests/mutants.out/caught.txt", crate::MUTANTS_OUT);
    let mut caught = passing();
    missing(&mut caught);
    let _caught = caught
        .files
        .insert(caught_file.clone(), format!("{}\n{}\n", names[1], names[0]));
    let mut out = Vec::new();
    assert_eq!(run_steps(&steps, &mut caught, &mut out), Ok(()), "caught");
    assert_eq!(
        caught.ran,
        [
            first.line(),
            baseline,
            format!("read {missed_file}"),
            pass.line(),
            format!("read {caught_file}")
        ],
        "the missed mutants, read from the first run's report, tested again, then the \
         mutants the pass caught read from its own"
    );
    let mut unreported = passing();
    missing(&mut unreported);
    assert_eq!(
        run_steps(&steps, &mut unreported, &mut Vec::new()),
        failed(format!("m: cannot read {caught_file}: missing")),
        "a pass that succeeds without its report proves nothing"
    );
    for (listed, not_caught) in [
        (String::new(), format!("{}, {}", names[0], names[1])),
        (format!("{}\n", names[0]), names[1].to_owned()),
    ] {
        let mut partly = passing();
        missing(&mut partly);
        let _listed = partly.files.insert(caught_file.clone(), listed);
        assert_eq!(
            run_steps(&steps, &mut partly, &mut Vec::new()),
            failed(format!(
                "m: the doc-test pass did not catch every mutant it tested again: \
                 {not_caught}"
            )),
            "a pass that tested fewer mutants than it was given (a name its filters no \
             longer select) proves nothing of the others: {not_caught}"
        );
    }
    let said = String::from_utf8(out).expect("UTF-8");
    assert!(
        said.contains(&format!(
            "  2 mutants no test caught under nextest, tested again by cargo test, doc tests \
             included:\n    {}\n    {}\n  caught with the doc tests: 2 mutants\n",
            names[0], names[1]
        )),
        "said: {said}"
    );
    let mut still = passing();
    missing(&mut still);
    still.fail = Some(pass.line());
    assert_eq!(
        run_steps(&steps, &mut still, &mut Vec::new()),
        failed(format!("m: `{}` exited with code 3", pass.line())),
        "any other failure of the pass fails the step"
    );
}

#[test]
fn judges_only_the_mutants_it_was_given_whatever_the_pass_exits_with() {
    let names = [
        "crates/audio-core/src/block.rs:61:5: replace engine_block with ()",
        "crates/audio-core/src/math.rs:9:5: replace f -> u32 with 0",
    ];
    let missed_file = format!("{}/missed.txt", crate::MUTANTS_OUT);
    let caught_file = format!("{}/doc-tests/mutants.out/caught.txt", crate::MUTANTS_OUT);
    let (steps, first, pass) = mutants_run(&names);
    let missing = |fake: &mut Fake| {
        fake.refuse = Some(first.line());
        let _missed = fake
            .files
            .insert(missed_file.clone(), format!("{}\n{}\n", names[0], names[1]));
    };
    // cargo-mutants 27.1.0 adds every deletion of a struct literal's field whatever its
    // filters: the pass then exits as missing mutants it was not given.
    let mut added = passing();
    missing(&mut added);
    let _added_refused = added
        .files
        .insert(format!("refuses {}", pass.line()), String::new());
    let _added_caught = added
        .files
        .insert(caught_file.clone(), format!("{}\n{}\n", names[0], names[1]));
    assert_eq!(
        run_steps(&steps, &mut added, &mut Vec::new()),
        Ok(()),
        "mutants it was not given, missed by the doc tests alone: judged by the first run"
    );
    let mut refused = passing();
    missing(&mut refused);
    let _refused_pass = refused
        .files
        .insert(format!("refuses {}", pass.line()), String::new());
    let _refused_caught = refused.files.insert(caught_file, format!("{}\n", names[0]));
    assert_eq!(
        run_steps(&steps, &mut refused, &mut Vec::new()),
        failed(format!(
            "m: the doc-test pass did not catch every mutant it tested again: {}",
            names[1]
        )),
        "a mutant given and missed by the doc tests too fails the step"
    );
}

#[test]
fn says_a_single_mutant_tested_again_as_one() {
    let name = "src/lib.rs:7:5: replace check with ()";
    let (steps, first, pass) = mutants_run(&[name]);
    let mut fake = passing();
    fake.refuse = Some(first.line());
    let _missed = fake.files.insert(
        format!("{}/missed.txt", crate::MUTANTS_OUT),
        format!("\n{name}\n\n"),
    );
    let _caught = fake.files.insert(
        format!("{}/doc-tests/mutants.out/caught.txt", crate::MUTANTS_OUT),
        format!("{name}\n"),
    );
    let mut out = Vec::new();
    assert_eq!(run_steps(&steps, &mut fake, &mut out), Ok(()));
    assert!(
        fake.ran.contains(&pass.line()),
        "blank lines name no mutant"
    );
    let said = String::from_utf8(out).expect("UTF-8");
    assert!(
        said.contains(&format!(
            "  1 mutant no test caught under nextest, tested again by cargo test, doc tests \
             included:\n    {name}\n  caught with the doc tests: 1 mutant\n"
        )),
        "said: {said}"
    );
}

#[test]
fn tests_nothing_again_after_a_failure_worse_than_a_missed_mutant() {
    let (steps, first, _) = mutants_run(&[]);
    let line = first.line();
    let missed_file = format!("{}/missed.txt", crate::MUTANTS_OUT);
    let mut timeout = passing();
    timeout.fail = Some(line.clone());
    let _missed = timeout
        .files
        .insert(missed_file.clone(), "src/lib.rs:1:1: x\n".to_owned());
    assert_eq!(
        run_steps(&steps, &mut timeout, &mut Vec::new()),
        failed(format!("m: `{line}` exited with code 3")),
        "a timeout (or a failing baseline) fails the step, missed mutants or not"
    );
    assert_eq!(
        timeout.ran,
        [
            line.clone(),
            format!("read {}/log/baseline.log", crate::MUTANTS_OUT)
        ],
        "no report read, no pass: only the baseline's times, for the test order"
    );
    let mut killed = passing();
    killed.killed = Some(line.clone());
    assert_eq!(
        run_steps(&steps, &mut killed, &mut Vec::new()),
        failed(format!("m: `{line}` was stopped by signal 9 (SIGKILL)")),
        "stopped"
    );
    let mut unlisted = passing();
    unlisted.refuse = Some(line.clone());
    let _blank = unlisted.files.insert(missed_file.clone(), "\n".to_owned());
    assert_eq!(
        run_steps(&steps, &mut unlisted, &mut Vec::new()),
        failed(format!("m: `{line}` exited with code 2")),
        "a run that says it missed mutants but names none fails as it ended"
    );
    let mut unreported = passing();
    unreported.refuse = Some(line);
    assert_eq!(
        run_steps(&steps, &mut unreported, &mut Vec::new()),
        failed(format!("m: cannot read {missed_file}: missing")),
        "its report is read"
    );
}

/// What the steps say when they run on `fake`, which they must pass.
fn said_after(steps: &[Step], fake: &mut Fake) -> String {
    let mut out = Vec::new();
    assert_eq!(run_steps(steps, fake, &mut out), Ok(()), "never fails");
    String::from_utf8(out).expect("UTF-8")
}

#[test]
fn ends_each_mutation_run_with_what_its_baseline_suggests_of_the_test_order() {
    let (steps, _, _) = mutants_run(&[]);
    let log = format!("{}/log/baseline.log", crate::MUTANTS_OUT);
    let config = crate::CargoWorkspace::Root.nextest_config();
    let mut slow = passing();
    let _slow_log = slow.files.insert(
        log.clone(),
        "        PASS [   2.500s] (1/2) audio-core limiter::tests::new\n        PASS [   \
         0.010s] (2/2) audio-core limiter::tests::holds\n"
            .to_owned(),
    );
    let suggested = said_after(&steps, &mut slow);
    assert!(
        suggested.contains(&format!(
            "  test order: 2 tests to move in the slow group of {config} (moving them in saves \
             up to 2.500 s a mutant a faster test catches waits; a test slower than measured \
             under load may rather need speeding up):\n    run last: audio-core \
             limiter::tests::new (2.500 s)\n    run with the others: audio-core \
             limiter::tests::holds (0.010 s)\n"
        )),
        "said: {suggested}"
    );
    let mut unreadable = passing();
    let _colored = unreadable.files.insert(
        log.clone(),
        "        \u{1b}[32mPASS\u{1b}[0m [   2.500s] (1/1) audio-core limiter::tests::new\n"
            .to_owned(),
    );
    let unmeasured = said_after(&steps, &mut unreadable);
    assert!(
        unmeasured.contains(&format!(
            "  test order: no suggestion, no test time read from {log}, in a form of nextest's \
             output this reader does not know\n"
        )),
        "a log the reader cannot read says so, never as a group as measured: {unmeasured}"
    );
    let mut unconfigured = passing();
    let _empty_log = unconfigured.files.insert(log.clone(), String::new());
    let _config = unconfigured.files.remove(config);
    let unread = said_after(&steps, &mut unconfigured);
    assert!(
        unread.contains(&format!(
            "  test order: no suggestion, cannot read {config}: missing\n"
        )),
        "said: {unread}"
    );
    let mut misconfigured = passing();
    let _blank_log = misconfigured.files.insert(log, String::new());
    let _retries = misconfigured
        .files
        .insert(config.to_owned(), "retries = 2\n".to_owned());
    let refused = said_after(&steps, &mut misconfigured);
    assert!(
        refused.contains(&format!(
            "  test order: no suggestion, {config} holds more than its slow group (the nextest \
             configuration gate says where)\n"
        )),
        "said: {refused}"
    );
}

#[test]
fn reads_the_gpu_workspace_s_baseline_and_slow_group_for_its_test_order() {
    let mut fake = passing();
    let _log = fake.files.insert(
        format!("{}/log/baseline.log", crate::GPU_MUTANTS_REPORT),
        "        PASS [   8.266s] (1/1) visual-gpu tests::gives\n".to_owned(),
    );
    let _unviable = fake.files.insert(
        format!("{}/unviable.txt", crate::GPU_MUTANTS_REPORT),
        String::new(),
    );
    let _root = fake.files.insert(UNVIABLE_REPORT.to_owned(), String::new());
    let mut out = Vec::new();
    let step = Step {
        name: "g".to_owned(),
        action: Action::GpuMutants(Extent::Complete),
    };
    assert_eq!(run_steps(&[step], &mut fake, &mut out), Ok(()));
    let said = String::from_utf8(out).expect("UTF-8");
    assert!(
        said.contains(&format!(
            "slow group of {}",
            crate::CargoWorkspace::Gpu.nextest_config()
        )) && said.contains("    run last: visual-gpu tests::gives (8.266 s)\n"),
        "said: {said}"
    );
}

#[test]
fn refuses_nextest_configurations_it_cannot_list_or_read() {
    let step = [Step {
        name: "c".to_owned(),
        action: Action::CargoConfigGate,
    }];
    let mut unlisted = passing();
    unlisted.broken = Some("list *nextest.toml".to_owned());
    assert_eq!(
        run_steps(&step, &mut unlisted, &mut Vec::new()).map_err(|failure| failure.summary),
        Err(
            "c: cannot list the nextest configurations: `git ls-files -- *nextest.toml` \
             exited with code 128: fatal: not a git repository"
                .to_owned()
        )
    );
    let root = crate::CargoWorkspace::Root.nextest_config();
    let mut unread = passing();
    unread.broken = Some(format!("read {root}"));
    assert_eq!(
        run_steps(&step, &mut unread, &mut Vec::new()).map_err(|failure| failure.summary),
        Err(format!("c: cannot read {root}: not UTF-8"))
    );
    let mut both = passing();
    both.broken = Some(format!("read {}", crate::GPU_MANIFEST));
    let _extra = both
        .files
        .insert("tooling/nextest.toml".to_owned(), String::new());
    assert_eq!(
        run_steps(&step, &mut both, &mut Vec::new()).map_err(|failure| failure.summary),
        Err("c: 2 gates failed, listed above".to_owned()),
        "the manifests' gate and the nextest configurations' gate, each run"
    );
}

#[test]
fn names_each_gpu_mutant_from_the_root_and_drops_blank_lines() {
    assert_eq!(
        crate::actions::from_the_root("src/a.rs:1:1: x\n\n  \nsrc/b.rs:2:2: y"),
        "crates/visual-gpu/src/a.rs:1:1: x\ncrates/visual-gpu/src/b.rs:2:2: y\n"
    );
    assert_eq!(crate::actions::from_the_root(""), "", "none");
}

#[test]
fn names_what_it_cannot_read_of_the_cpu_reference_and_the_build_time_code() {
    for workspace in crate::CargoWorkspace::ALL {
        let mut fake = passing();
        fake.broken = Some(crate::cpu_reference_command(workspace).line());
        let failure = super::cpu_reference(&mut fake).expect_err("cannot read");
        assert!(
            failure
                .summary
                .starts_with("cannot read the CPU reference's tree: "),
            "{workspace:?}: {}",
            failure.summary
        );
        let mut listing = passing();
        listing.broken = Some(crate::resolved_metadata_command(workspace).line());
        let unlisted = super::build_time(&mut listing).expect_err("cannot list");
        assert!(
            unlisted
                .summary
                .starts_with("cannot list the crates the builds use: "),
            "{workspace:?}: {}",
            unlisted.summary
        );
    }
}

#[test]
fn refuses_a_gpu_source_left_mutated() {
    let gate = [Step {
        name: "No mutant left in place in the GPU workspace".to_owned(),
        action: Action::MutantLeftGate,
    }];
    let search = crate::preflight::mutant_search_command().line();
    // The fake answers a capture it holds no answer for with exit code 1: grep's "none".
    assert_eq!(
        run_steps(&gate, &mut passing(), &mut Vec::new()),
        Ok(()),
        "grep found none"
    );
    let mut found = passing();
    let _found = found.files.insert(
        format!("capture {search}"),
        "crates/visual-gpu/src/lib.rs:2:    0 /* ~ changed by cargo-mutants ~ */\n".to_owned(),
    );
    assert_eq!(
        run_steps(&gate, &mut found, &mut Vec::new()),
        failed(
            "No mutant left in place in the GPU workspace: a mutant left in place by a stopped \
             mutation run (restore each file named, with `git checkout -- <file>` when it holds \
             no change of yours): crates/visual-gpu/src/lib.rs:2 holds cargo-mutants' marker"
        ),
        "a source holding the marker"
    );
    // Any other ending, here grep not started, fails the search.
    let mut fake = passing();
    fake.broken = Some(search.clone());
    assert_eq!(
        run_steps(&gate, &mut fake, &mut Vec::new()),
        failed(format!(
            "No mutant left in place in the GPU workspace: cannot search the GPU workspace for \
             a mutant left in place: cannot run `{search}`: not found"
        )),
        "grep not started"
    );
}

#[test]
fn requires_the_spec_index_to_link_every_specification() {
    use crate::actions::spec_index_gate;
    let files = [
        "spec/README.md",
        "spec/render-hash.md",
        "spec/render-hash-vectors.json",
    ]
    .map(str::to_owned);
    let index = "# spec\n\n| [`render-hash.md`](render-hash.md) | The hash, its [vectors](render-hash-vectors.json#format) |\nSee [the project](../README.md) and [the site](https://example.org/a.md).\n";
    assert_eq!(
        spec_index_gate(&files, index),
        Ok(()),
        "every specification linked, an anchor apart; links outside spec/ are not its"
    );
    assert_eq!(
        spec_index_gate(&files, "# spec\n\n[`render-hash.md`](render-hash.md)\n"),
        failed(
            "specifications not indexed: spec/README.md does not link render-hash-vectors.json \
             (`[…](render-hash-vectors.json)`)"
        ),
        "a specification no reader finds from the index (named, not linked, does not count)"
    );
    assert_eq!(
        spec_index_gate(
            &files,
            &format!("{index}[the old one](render-hash-v1.md) and [nothing]()\n")
        ),
        failed(
            "specifications not indexed: spec/README.md links render-hash-v1.md, which is not a \
             tracked file of spec/"
        ),
        "a link to a specification that is gone; an empty link is none"
    );
    assert_eq!(
        spec_index_gate(
            &files,
            "[`render-hash.md`](render-hash.md) [a](render-hash-vectors.json"
        ),
        failed(
            "specifications not indexed: spec/README.md does not link render-hash-vectors.json \
             (`[…](render-hash-vectors.json)`)"
        ),
        "a link never closed is none"
    );
}

#[test]
fn holds_the_tracked_specifications_to_their_index() {
    let step = [Step {
        name: "Every part described in a README".to_owned(),
        action: Action::ReadmeGate,
    }];
    let with_index = |index: &str| {
        let mut fake = passing();
        for (path, text) in [
            ("spec/render-hash.md", "# The hash\n"),
            ("spec/README.md", index),
        ] {
            let _spec = fake.files.insert(path.to_owned(), text.to_owned());
        }
        fake
    };
    assert_eq!(
        run_steps(
            &step,
            &mut with_index("[`render-hash.md`](render-hash.md)\n"),
            &mut Vec::new()
        ),
        Ok(()),
        "the index, read among the READMEs, links the specification"
    );
    assert_eq!(
        run_steps(&step, &mut with_index("# spec\n"), &mut Vec::new()),
        failed(
            "Every part described in a README: specifications not indexed: spec/README.md does \
             not link render-hash.md (`[…](render-hash.md)`)"
        ),
        "a specification the index does not link"
    );
}

#[test]
fn reads_the_reviews_of_the_last_version_and_the_specifications() {
    let step = [Step {
        name: "names".to_owned(),
        action: Action::UnviableNamesGate,
    }];
    let at_base = format!(
        "capture git show {}:{}",
        crate::tests::MAIN_BASE.1.trim(),
        crate::UNVIABLE_REVIEWED
    );
    let mut unread = passing();
    let _gone = unread.files.remove(&at_base);
    assert!(
        run_steps(&step, &mut unread, &mut Vec::new()).is_err_and(|failure| failure
            .summary
            .starts_with(
                "names: cannot read mutants-unviable.txt at the merge base \
                 fed4560000000000000000000000000000000000"
            )),
        "the reviews of the last version cannot be read"
    );
    let mut cited = passing();
    let _spec = cited.files.insert(
        "spec/benchmarks.md".to_owned(),
        "Held by `rebuilds_a_new_path_by_sixteen_turns`.\n".to_owned(),
    );
    assert_eq!(
        run_steps(&step, &mut cited, &mut Vec::new()),
        failed(
            "names: spec/benchmarks.md:1: names `rebuilds_a_new_path_by_sixteen_turns`, which \
             no Rust source defines as a function: a specification names a test as it is now, \
             or says what it measured without the name"
        ),
        "a specification read among the tracked files"
    );
    let _added = cited.files.insert(
        crate::UNVIABLE_REVIEWED.to_owned(),
        "crates/a/src/b.rs: replace f -> T with Default::default() -- T has no Default\n\
         crates/a/src/b.rs: replace g with () -- a reason naming no test\n"
            .to_owned(),
    );
    assert!(
        run_steps(&step, &mut cited, &mut Vec::new()).is_err_and(|failure| failure.summary
            == "names: 2 gates failed, listed above"
            && failure.details.len() == 2),
        "both refusals, the review added since the last version and the specification"
    );
    // Finding F5-3 of the fourth deep review of visual block 2 (#34): the corpus's
    // exemptions name the test that proves each instead, and no gate read those names, so
    // a test renamed left its exemption's reason stale.
    for (reason, problem) in [
        (
            "killed by renamed_since_it_was_reviewed",
            "names `renamed_since_it_was_reviewed`, which no Rust source defines as a \
             function: a review names the test that kills its viable variant as it is now",
        ),
        (
            "no test named",
            "its reason names no test: a review added since the last version names the test \
             that kills its mutant's viable variant",
        ),
    ] {
        let mut stale = passing();
        let _exempt = stale.files.insert(
            crate::CORPUS_EXEMPT.to_owned(),
            format!(
                "# Reviewed.\ncrates/visual-core/src/image.rs: replace f with () -- {reason}\n"
            ),
        );
        let found = run_steps(&step, &mut stale, &mut Vec::new());
        assert!(
            found.as_ref().is_err_and(|failure| failure
                .details
                .iter()
                .chain([&failure.summary])
                .any(|line| line.contains(&format!("{}:2: {problem}", crate::CORPUS_EXEMPT)))),
            "{reason}: {found:?}"
        );
    }
}

#[test]
fn ties_the_versions_both_lock_files_share_in_the_build_time_step() {
    let mut fake = passing();
    let _drifted = fake.files.insert(
        format!(
            "capture {}",
            crate::resolved_metadata_command(crate::CargoWorkspace::Gpu).line()
        ),
        r#"{"packages":[{"name":"libm","version":"0.2.15","source":"registry+x","targets":[{"kind":["lib"]}]}]}"#.to_owned(),
    );
    let refused = super::build_time(&mut fake).expect_err("two versions of libm");
    assert_eq!(
        refused.summary,
        "the crates both Cargo workspaces resolve must be at one version in both lock files: \
         libm: Cargo.lock resolves 0.2.16, crates/visual-gpu/Cargo.lock 0.2.15: bring them to \
         one version (see: cargo update --manifest-path crates/visual-gpu/Cargo.toml \
         -p libm@0.2.15 --precise 0.2.16)",
        "the root's metadata against the GPU workspace's, beside the build-time code gate"
    );
}

#[test]
fn refuses_a_package_source_its_readme_does_not_name() {
    // Lesson 5 of the third deep review of visual block 2 (#34), from finding C10-11: the
    // playground's README listed some of its modules and not the seven added after its table.
    let readme = |text: &str| vec![("apps/app/README.md".to_owned(), text.to_owned())];
    let sources: Vec<String> = [
        "apps/app/src/a.ts",
        "apps/app/src/cli/bin.ts",
        "apps/app/src/env.d.ts",
        "apps/app/ts/b.tsx",
        "apps/app/test/a.test.ts",
        "apps/app/e2e/a.spec.ts",
        "apps/app/vite.config.ts",
        "apps/other/src/c.ts",
    ]
    .map(str::to_owned)
    .to_vec();
    assert_eq!(
        crate::actions::module_table_gate(
            &sources,
            &readme("| `src/a.ts` | a |\n| `src/cli/` | the tool |\n| `ts/b.tsx`, `x` | b |"),
            &["apps/app"]
        ),
        Ok(()),
        "every source named, by its path or its directory's; declarations, tests and \
         configurations apart"
    );
    assert_eq!(
        crate::actions::module_table_gate(
            &sources,
            &readme("`src/a.ts` src/cli/bin.ts `lib/` `src/cli`"),
            &["apps/app"]
        ),
        failed(
            "modules not described: 2 problems, listed above\n\
             apps/app/README.md does not name src/cli/bin.ts (`src/cli/bin.ts`, or its directory)\n\
             apps/app/README.md does not name ts/b.tsx (`ts/b.tsx`, or its directory)"
        )
    );
    assert_eq!(
        crate::actions::module_table_gate(&sources, &[], &["apps/app"]),
        failed(
            "modules not described: 3 problems, listed above\n\
             apps/app/README.md does not name src/a.ts (`src/a.ts`, or its directory)\n\
             apps/app/README.md does not name src/cli/bin.ts (`src/cli/bin.ts`, or its directory)\n\
             apps/app/README.md does not name ts/b.tsx (`ts/b.tsx`, or its directory)"
        ),
        "no README: the readme gate says it once"
    );
}
