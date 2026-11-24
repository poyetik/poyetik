//! Tests of running the steps: the inputs each action reads and how its verdict is said.

use alloc::collections::BTreeSet;
use core::num::NonZeroUsize;
use core::time::Duration;
use std::io;

use crate::actions::conflict_markers;
use crate::rust_source::TokenKind;
use crate::steps::Failure;
use crate::tests::{
    Broken, CHANGES, CHANGES_MODE, FAKE_JOBS, Fake, failed, found, hermetic_git,
    hermetic_git_ending, one_line_change, own_sources, parts_refused, passing, readmes, scratch,
    system,
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
    assert_eq!(
        elsewhere.map_err(|failure| failure.summary),
        Err(format!(
            "the push cannot be checked: pushing {parent}, but {} is checked out: check out the pushed commit, then push again",
            object("HEAD")
        )),
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
        assert_eq!(
            conflict_markers(&mut runner),
            failed(
                "cannot read logo.png: stream did not contain valid UTF-8 (a binary file is declared `*.<extension> binary` in \
                 .gitattributes)"
            ),
            "{style}: says how to declare it"
        );
        let declared = b"* text=auto eol=lf\n*.wasm binary\n*.png binary\n";
        write(".gitattributes", declared);
        write("gain.rs", b"main\n");
        assert_eq!(
            conflict_markers(&mut runner),
            Ok(()),
            "{style}: the remedy followed"
        );
    }
}

#[test]
fn says_why_it_cannot_read_the_attributes_or_list_the_files() {
    let mut missing = passing();
    let _removed = missing.files.remove(crate::GIT_ATTRIBUTES);
    assert_eq!(
        conflict_markers(&mut missing).map_err(|failure| failure.summary),
        Err("cannot read .gitattributes: missing".to_owned()),
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
        assert_eq!(
            super::cpu_reference(&mut fake),
            failed(format!(
                "cannot read the CPU reference's tree: cannot run `{}`: not found",
                crate::cpu_reference_command(workspace).line()
            )),
            "{workspace:?}"
        );
        let mut listing = passing();
        listing.broken = Some(crate::resolved_metadata_command(workspace).line());
        assert_eq!(
            super::build_time(&mut listing),
            failed(format!(
                "cannot list the crates the builds use: cannot run `{}`: not found",
                crate::resolved_metadata_command(workspace).line()
            )),
            "{workspace:?}"
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
             mutation run (each file named restores from the index, which keeps what you \
             staged, by the hint after it): crates/visual-gpu/src/lib.rs:2 holds \
             cargo-mutants' marker: restore it when it holds no change of yours (see: git \
             restore --worktree -- ':/crates/visual-gpu/src/lib.rs')"
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
    assert_eq!(
        run_steps(&step, &mut unread, &mut Vec::new()).map_err(|failure| failure.summary),
        Err("names: cannot read mutants-unviable.txt at the merge base fed4560000000000000000000000000000000000: `git show fed4560000000000000000000000000000000000:mutants-unviable.txt` exited with code 1: failed".to_owned()),
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
    assert_eq!(
        run_steps(&step, &mut cited, &mut Vec::new()).map_err(|failure| format!(
            "{}\n{}",
            failure.summary,
            failure.details.join("\n")
        )),
        Err("names: 2 gates failed, listed above\nmutants-unviable.txt:2: its reason names no test: a review added since the last version names the test that kills its mutant's viable variant\nspec/benchmarks.md:1: names `rebuilds_a_new_path_by_sixteen_turns`, which no Rust source defines as a function: a specification names a test as it is now, or says what it measured without the name".to_owned()),
        "both refusals, the review added since the last version and the specification"
    );
    // The TypeScript tests, and the index among the specifications: the test of a reader no
    // row names, then named by one; test code alone is read.
    let mut reader = passing();
    for path in ["bindings/a/ts/a.ts", "bindings/a/test/a.test.ts"] {
        let _script = reader.files.insert(
            path.to_owned(),
            "\"reads it only as bless writes it\"\n".to_owned(),
        );
    }
    assert_eq!(
        run_steps(&step, &mut reader, &mut Vec::new()),
        failed(
            "names: bindings/a/test/a.test.ts:1: `## Files only a tool writes` of spec/README.md \
             does not name \"reads it only as bless writes it\", the test of a reader of a file \
             only a tool writes: name it in that file's row"
        ),
        "a reader's test the index does not name"
    );
    let _index = reader.files.insert(
        "spec/README.md".to_owned(),
        "## Files only a tool writes\n\n| \"reads it only as bless writes it\" |\n".to_owned(),
    );
    assert_eq!(
        run_steps(&step, &mut reader, &mut Vec::new()),
        Ok(()),
        "named by the index"
    );
    // Finding F5-3 of the fourth deep review of visual block 2 (#34): the corpus's
    // exemptions name the test that proves each instead, and no gate read those names, so
    // a test renamed left its exemption's reason stale.
    for (reason, problem) in [
        (
            "killed by renamed_since_it_was_reviewed",
            "names: 2 gates failed, listed above\n\
             mutants-corpus-exempt.txt:2: names `renamed_since_it_was_reviewed`, which no Rust \
             source defines as a function: a review names the test that kills its viable variant \
             as it is now\n\
             mutants-corpus-exempt.txt:2: its reason names no test: a review added since the \
             last version names the test that kills its mutant's viable variant",
        ),
        (
            "no test named",
            "names: mutants-corpus-exempt.txt:2: its reason names no test: a review added since \
             the last version names the test that kills its mutant's viable variant\n",
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
        assert_eq!(
            found.map_err(|failure| format!("{}\n{}", failure.summary, failure.details.join("\n"))),
            Err(problem.to_owned()),
            "{reason}"
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
         one version (see: cargo update --manifest-path \"$(git rev-parse --show-toplevel)/crates/visual-gpu/Cargo.toml\" \
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

/// A step of the checks run on a runner.
type TextStep = fn(&mut dyn Runner) -> Result<(), Failure>;

#[test]
fn names_how_to_declare_a_binary_file_where_declaring_it_helps() {
    // Findings H13-5 of the fifth deep review of visual block 2 (#34) and K13-8 of the sixth:
    // one step of the two over the tracked text files failed bare, then both gave the remedy
    // whatever the error, and the hints step, which the declaration does not reach, gave it too.
    const REMEDY: &str = " (a binary file is declared `*.<extension> binary` in .gitattributes)";
    let steps: [(&str, TextStep, &str); 3] = [
        ("conflict markers", conflict_markers, REMEDY),
        ("retired patterns", super::retired_patterns, REMEDY),
        ("runnable hints", super::runnable_hints, ""),
    ];
    for (step, run, remedy) in steps {
        let mut binary = passing();
        binary.broken = Some("read README.md".to_owned());
        assert_eq!(
            run(&mut binary),
            failed(format!("cannot read README.md: not UTF-8{remedy}")),
            "{step}"
        );
        let mut denied = passing();
        put(&mut denied, "denies README.md", "");
        assert_eq!(
            run(&mut denied),
            failed("cannot read README.md: denied"),
            "{step}: no declaration helps"
        );
    }
}

/// A rule of a gate: the gate, the action of the step that runs it, what the step then says
/// (a hint's opening written `%see%`, its word `%word%`, a word the vocabulary refuses `%banned%`, so that no gate reads one
/// here), and the edit of the fake repository it refuses.
type Rule = (&'static str, Action, &'static str, fn(&mut Fake));

/// Puts `text` in the file `path` of `fake`.
fn put(fake: &mut Fake, path: &str, text: &str) {
    let _before = fake.files.insert(path.to_owned(), text.to_owned());
}

/// Puts `to` in place of `from` in the file `path` of `fake`, which holds it.
fn swap(fake: &mut Fake, path: &str, from: &str, to: &str) {
    let text = fake.files.get_mut(path).expect("a file of the fake");
    let edited = text.replacen(from, to, 1);
    assert_ne!(*text, edited, "{path} holds {from}");
    *text = edited;
}

/// The file of the fake holding what it answers to `cmd`.
fn captured(cmd: &Cmd) -> String {
    format!("capture {}", cmd.line())
}

/// The file of the fake holding the root workspace's metadata.
fn metadata() -> String {
    captured(&crate::metadata_command(crate::CargoWorkspace::Root))
}

/// The file of the fake holding the GPU workspace's resolved metadata.
fn gpu_resolved() -> String {
    captured(&crate::resolved_metadata_command(
        crate::CargoWorkspace::Gpu,
    ))
}

/// Adds `text` at the end of the file `path` of `fake`.
fn add(fake: &mut Fake, path: &str, text: &str) {
    fake.files
        .entry(path.to_owned())
        .or_default()
        .push_str(text);
}

/// The rules of the gates over test code.
fn test_code_rules() -> Vec<Rule> {
    vec![
        (
            "exact_errors_gate",
            Action::TestCodeGate,
            "step: crates/audio-core/tests/vague.rs:2: `f` judges an error by `is_err`: compare it whole, or review the function in INEXACT_ERRORS with why its text cannot be known (crates/xtask/src/exact_errors.rs)",
            |fake| {
                put(
                    fake,
                    "crates/audio-core/tests/vague.rs",
                    "#![forbid(unsafe_code)]\nfn f() { x.is_err(); }\n",
                );
            },
        ),
        (
            "test_data_gate",
            Action::TestDataGate,
            "step: crates/audio-core/src/a_tests.rs:1: the tests of audio-core read `crates/x.json`, which is not in its test_data",
            |fake| {
                put(
                    fake,
                    "crates/audio-core/src/a_tests.rs",
                    "const A: &str = include_str!(\"../../x.json\");\n",
                );
            },
        ),
        (
            "safe_vectors_gate",
            Action::TestDataGate,
            "visual-cli: declares `conformance/visual/` as test data, which no test of it reads\nvisual-cli: declares `spec/safe-message-vectors.json` as test data, which no test of it reads\n2 problems, listed above\nvisual-cli: a tool whose tests read no spec/safe-message-vectors.json: escape and bound the untrusted text it prints by the shared rule and test it against the vectors, or list it in ESCAPED_ELSEWHERE with why (crates/xtask/src/test_code.rs)\nstep: 2 gates failed, listed above",
            |fake| {
                put(fake, "crates/visual-cli/tests/data.rs", "");
            },
        ),
        (
            "unviable_names_gate",
            Action::UnviableNamesGate,
            "mutants-unviable.txt:1: names `a_test_now_gone`, which no Rust source defines as a function: a review names the test that kills its viable variant as it is now\nmutants-unviable.txt:1: its reason names no test: a review added since the last version names the test that kills its mutant's viable variant\nstep: 2 gates failed, listed above",
            |fake| {
                swap(
                    fake,
                    crate::UNVIABLE_REVIEWED,
                    "T has no Default",
                    "T has no Default; killed by a_test_now_gone",
                );
            },
        ),
        (
            "reader_tests_gate",
            Action::UnviableNamesGate,
            "step: spec/README.md: `## Files only a tool writes` does not name `reads_it_only_as_bless_writes_it`, the test of a reader of a file only a tool writes: name it in that file's row",
            |fake| {
                put(
                    fake,
                    "crates/audio-core/tests/reader.rs",
                    "#[test]\nfn reads_it_only_as_bless_writes_it() {}\n",
                );
            },
        ),
        (
            "reader_tests_gate",
            Action::UnviableNamesGate,
            "step: bindings/a/test/a.test.ts:1: `## Files only a tool writes` of spec/README.md does not name \"reads it only as bless writes it\", the test of a reader of a file only a tool writes: name it in that file's row",
            |fake| {
                put(
                    fake,
                    "bindings/a/test/a.test.ts",
                    "test(\"reads it only as bless writes it\", () => {});\n",
                );
            },
        ),
    ]
}

/// The rules of the gates over Rust code.
fn rust_rules() -> Vec<Rule> {
    vec![
        (
            "unsafe_gate",
            Action::UnsafeGate,
            "step: the lint against unsafe code: crates/audio-core/src/u.rs:1: names `unsafe` outside UNSAFE_FILES and the unit tests they declare: rustc does not lint code expanded from another crate's macro, so unsafe code is written only where Miri and the audit see it",
            |fake| put(fake, "crates/audio-core/src/u.rs", "unsafe fn f() {}\n"),
        ),
        (
            "library_lints_gate",
            Action::LibraryLintsGate,
            "step: the lints of the libraries: crates/audio-core/src/lib.rs holds no leading attribute exactly `#![deny(clippy::exhaustive_enums)]` (one lint per attribute)",
            |fake| {
                swap(
                    fake,
                    "crates/audio-core/src/lib.rs",
                    "#![deny(clippy::exhaustive_enums)]\n",
                    "",
                );
            },
        ),
        (
            "no_std_portability_gate",
            Action::LibraryLintsGate,
            "step: the portability of the `no_std` crates: crates/audio-core/src/x.rs declares `no_std` but is not the root `src/lib.rs` of a crate of RUST_CRATES, the one root this gate reads",
            |fake| put(fake, "crates/audio-core/src/x.rs", "#![no_std]\n"),
        ),
        (
            "untested_targets_gate",
            Action::UntestedTargetsGate,
            "step: crates/x/src/lib.rs: its doc tests are turned off (`doctest = false`): they would never run",
            |fake| swap(fake, &metadata(), "\"doctest\":true", "\"doctest\":false"),
        ),
        (
            "compile_fail_codes_gate",
            Action::UntestedTargetsGate,
            "step: crates/audio-core/src/x.rs:1: a compile_fail doc test that names no error code (`compile_fail,E…`), so rustdoc accepts it whatever it fails with: name the code it fails with",
            |fake| put(fake, "crates/audio-core/src/x.rs", "//! ```compile_fail\n"),
        ),
        (
            "cpu_reference_gate",
            Action::CpuReferenceGate,
            "the root's tree:\nvisual-core v0.2.0 (/r/crates/visual-core)\n\nthe GPU workspace's tree:\nvisual-core v0.3.0 (/r/crates/visual-core)\n\nstep: the GPU workspace builds the CPU reference (visual-core, visual-boxes) otherwise than the root, so that its tests would compare the GPU's images with another reference than the shipped one",
            |fake| {
                let tree = crate::cpu_reference_command(crate::CargoWorkspace::Gpu);
                put(
                    fake,
                    &captured(&tree),
                    "visual-core v0.3.0 (/r/crates/visual-core)\n",
                );
            },
        ),
        (
            "float_gate",
            Action::FloatMethodGate,
            "`f64::sin` is neither banned in clippy.toml nor reviewed as deterministic (DETERMINISTIC_FLOAT_METHODS in xtask)\nclippy.toml bans `f64::tan`, which the pinned toolchain does not have\nstep: the float methods of the pinned toolchain: 2 problems, listed above",
            |fake| swap(fake, crate::CLIPPY_TOML, "f64::sin", "f64::tan"),
        ),
        (
            "named_sources_gate",
            Action::MathLayerGate,
            "step: crates/audio-core/src/math.rs: the math layer gate names it but does not read it: moved or removed, its rule would hold for nothing; name the file where it is",
            |fake| drop(fake.files.remove(crate::MATH_LAYER_FILE)),
        ),
        (
            "math_layer_gate",
            Action::MathLayerGate,
            "step: box arithmetic only through the math layer (audio-core, audio-testkit only may depend on libm; box code names no raw float, libm, host or delivery, and computes no float; no iterator reduction outside the math layer; code comes in by `mod` alone): x depends on libm: its arithmetic must go through the math layer of audio-core",
            |fake| swap(fake, &metadata(), "serde_json", "libm"),
        ),
        (
            "loaded_files_gate",
            Action::LoadedFilesGate,
            "step: a.txt: read by the build of `fpenv` (fpenv), outside its crate",
            |fake| {
                let stem = "/t/debug/deps/fpenv-1";
                put(
                    fake,
                    &format!("{stem}.d"),
                    &crate::tests::dep_info(stem, &[crate::ASSEMBLY_FILES[0], "a.txt"]),
                );
            },
        ),
        (
            "validated_shaders_gate",
            Action::LoadedFilesGate,
            "step: crates/visual-gpu/tests/shaders.rs validates crates/visual-boxes/src/square.wgsl as `visual_core::SQUARE_WGSL`, a constant of another crate than the one holding the shader",
            |fake| {
                swap(
                    fake,
                    crate::loaded_files::SHADER_TEST_SOURCE,
                    "visual_boxes::SQUARE_WGSL",
                    "visual_core::SQUARE_WGSL",
                );
            },
        ),
    ]
}

/// The rules of the gates over dependencies and licenses.
fn dependency_rules() -> Vec<Rule> {
    vec![
        (
            "dependency_gate",
            Action::DependencyGate,
            "step: direct dependencies must be reviewed in dependencies.txt and keep their names: x depends on serde_json renamed `core`: a dependency keeps its package's name, so that no name (`core`, `std` …) can stand for another crate",
            |fake| {
                swap(
                    fake,
                    &metadata(),
                    "{\"name\":\"serde_json\",",
                    "{\"name\":\"serde_json\",\"rename\":\"core\",",
                );
            },
        ),
        (
            "pin_gate",
            Action::PinGate,
            "step: x: serde_json (normal) is required as ``: pin it to one version, `=X.Y.Z`, in its Cargo.toml, so that an update, which can move bits, is decided alone (spec/engine-design.md, decision 29)",
            |fake| {
                swap(
                    fake,
                    crate::DEPENDENCIES_REVIEWED,
                    "reads JSON",
                    "bit-defining: reads JSON",
                );
            },
        ),
        (
            "library_gate",
            Action::LibraryGate,
            "step: audio-patch: not in the cargo metadata, so its dependencies cannot be checked",
            |fake| swap(fake, &metadata(), "\"audio-patch\"", "\"other\""),
        ),
        (
            "shared_tests_gate",
            Action::LibraryGate,
            "step: visual-hash: only tests reach it, so a change to it can weaken the tests of every crate using it: give it CrateRole::SharedTests",
            |fake| {
                swap(
                    fake,
                    &metadata(),
                    "{\"name\":\"visual-hash\",\"kind\":null",
                    "{\"name\":\"visual-hash\",\"kind\":\"dev\"",
                );
            },
        ),
        (
            "part_license_gate",
            Action::PartLicenseGate,
            "step: crates/x: the crate x is under MIT, not MIT OR Apache-2.0",
            |fake| {
                swap(
                    fake,
                    &metadata(),
                    "{\"license\":\"MIT OR Apache-2.0\",\"name\":\"x\"",
                    "{\"license\":\"MIT\",\"name\":\"x\"",
                );
            },
        ),
        (
            "license_gate",
            Action::LicenseGate,
            "step: licenses not allowed: g is under GPL-3.0",
            |fake| {
                swap(
                    fake,
                    &format!("capture {}", crate::tests::LICENSE_LINES[1]),
                    "\"Apache-2.0\"",
                    "\"GPL-3.0\":[{\"name\":\"g\"}],\"Apache-2.0\"",
                );
            },
        ),
        (
            "shipped_exceptions_gate",
            Action::LicenseGate,
            "step: pnpm audit's exceptions: `GHSA-fc5q-39xc-vjpf` (react) is ignored by pnpm audit in a package an app ships (pnpm-workspace.yaml:12): an exception covers development tools only",
            |fake| {
                add(
                    fake,
                    crate::PNPM_WORKSPACE,
                    "# GHSA-fc5q-39xc-vjpf (react): r\n",
                );
            },
        ),
    ]
}

/// The rules of the gates over what the builds read beside the code.
fn build_rules() -> Vec<Rule> {
    vec![
        (
            "build_time_gate",
            Action::BuildTimeGate,
            "step: code third-party crates run at build time must be reviewed in build-time-code.txt: not reviewed: m@1.0.0 (proc macro)",
            |fake| {
                swap(
                    fake,
                    &gpu_resolved(),
                    "{\"packages\":[",
                    "{\"packages\":[{\"name\":\"m\",\"version\":\"1.0.0\",\"source\":\"registry+x\",\"targets\":[{\"kind\":[\"proc-macro\"]}]},",
                );
            },
        ),
        (
            "shared_versions_gate",
            Action::BuildTimeGate,
            "step: the crates both Cargo workspaces resolve must be at one version in both lock files: libm: Cargo.lock resolves 0.2.16, crates/visual-gpu/Cargo.lock 0.2.15: bring them to one version %see%cargo update --manifest-path \"$(git rev-parse --show-toplevel)/crates/visual-gpu/Cargo.toml\" -p libm@0.2.15 --precise 0.2.16)",
            |fake| {
                swap(fake, &gpu_resolved(), "0.2.16", "0.2.15");
                add(
                    fake,
                    crate::BUILD_TIME_REVIEWED,
                    "libm@0.2.15 (build script) -- emits cfgs\n",
                );
            },
        ),
        (
            "cargo_config_gate",
            Action::CargoConfigGate,
            "expected:\n[alias]\n# The single entry point for project automation: cargo xtask check runs every check.\nxtask = \"run --package xtask --locked --quiet --\"\n\nfound:\n[alias]\n# The single entry point for project automation: cargo xtask check runs every check.\nxtask = \"run --package xtask --locked --quiet --\"\nx = 1\n\nstep: .cargo/config.toml differs from the configuration xtask expects (no build flag, a locked alias)",
            |fake| add(fake, crate::CARGO_CONFIG_FILE, "x = 1\n"),
        ),
        (
            "nextest_configs_gate",
            Action::CargoConfigGate,
            "step: tooling/nextest.toml: a nextest configuration outside a workspace's own (`.config/nextest.toml` at its root), which no gate reads",
            |fake| put(fake, "tooling/nextest.toml", ""),
        ),
        (
            "gpu_manifest_gate",
            Action::CargoConfigGate,
            "crates/visual-gpu/Cargo.toml holds other sections than [workspace], [workspace.package], [workspace.lints.rust], [workspace.lints.rustdoc], [workspace.lints.clippy], [package], [dependencies], [dev-dependencies], [lints], in that order\ncrates/visual-gpu/Cargo.toml: its [lints] table is not exactly `workspace = true`\nstep: the GPU workspace shares the root's package fields and lints: 2 problems, listed above",
            |fake| add(fake, crate::GPU_MANIFEST, "[features]\n"),
        ),
    ]
}

/// The rules of the gates over the packages and the history.
fn package_rules() -> Vec<Rule> {
    vec![
        (
            "module_table_gate",
            Action::ReadmeGate,
            "step: modules not described: tooling/config/README.md does not name src/b.ts (`src/b.ts`, or its directory)",
            |fake| put(fake, "tooling/config/src/b.ts", ""),
        ),
        (
            "stryker_config_gate",
            Action::StrykerConfigGate,
            "step: tooling/config/stryker.config.json hides mutants: `!src/**/*.ts` in mutate excludes files from mutation",
            |fake| {
                swap(
                    fake,
                    "tooling/config/stryker.config.json",
                    "\"src/",
                    "\"!src/",
                );
            },
        ),
        (
            "package_order_gate",
            Action::PackageCheckGate,
            "step: tooling/oracle/package.json: depends on @poyetik/web-page, whose checks do not run before its own (`PACKAGES` in xtask's workspace.rs)",
            |fake| {
                swap(
                    fake,
                    "tooling/oracle/package.json",
                    "\"@poyetik/config\"",
                    "\"@poyetik/web-page\"",
                );
            },
        ),
        (
            "test_exports_gate",
            Action::PackageCheckGate,
            "step: package exports into test code, which neither coverage nor mutation testing measures (the scope gate skips it): move each under the package's measured sources (src/testing/, ts/testing/): bindings/audio-wasm/package.json: ./b exports ./test/b.ts",
            |fake| {
                swap(
                    fake,
                    "bindings/audio-wasm/package.json",
                    "./dist/a.js\"",
                    "./dist/a.js\", \"./b\": \"./test/b.ts\"",
                );
            },
        ),
        (
            "bypass_gate",
            Action::BypassGate,
            "step: bypasses of coverage or mutation testing must be reviewed in proof-bypasses.txt: not reviewed: mutants.toml: a cargo-mutants configuration",
            |fake| put(fake, "mutants.toml", ""),
        ),
        (
            "scope_gate",
            Action::TypeScriptScopeGate(PACKAGES[0]),
            "step: TypeScript sources outside coverage or mutation testing must be reviewed in proof-bypasses.txt: tooling/config/src/b.ts is mutated but not measured: the coverage `include` misses it",
            |fake| put(fake, "tooling/config/src/b.ts", ""),
        ),
        (
            "ts_coverage_gate",
            Action::TypeScriptReportsGate(PACKAGES[0]),
            "step: tooling/config: coverage is not 100%: lines 3/4",
            |fake| {
                swap(
                    fake,
                    "tooling/config/coverage/coverage-summary.json",
                    "\"covered\":4",
                    "\"covered\":3",
                );
            },
        ),
        (
            "history_gate",
            Action::HistoryGate,
            "step: the history breaks the commit rules: commit aaaaaaa: invalid commit message: missing \"<type>: \" prefix",
            |fake| {
                swap(
                    fake,
                    &format!("capture {}", crate::tests::HISTORY_LINES[0]),
                    "docs: start",
                    "start",
                );
            },
        ),
        (
            "conflict_marker_gate",
            Action::ConflictMarkerGate,
            "step: a.rs:1: a conflict marker git left, unresolved",
            |fake| put(fake, "a.rs", "<<<<<<< HEAD\n"),
        ),
        (
            "doc_tests",
            Action::DocTests,
            "step: the doc tests of crates/xtask/tests/fixtures/doc-codes passed: rustdoc did not check the error code of a compile_fail doc test",
            |fake| {
                let fixture = crate::doc_codes_fixture_command().line();
                fake.files.retain(|path, _| !path.ends_with(&fixture));
            },
        ),
    ]
}

/// The rules of the gates over typed lists, registers, git, kinds of files and wordings.
fn register_and_list_rules() -> Vec<Rule> {
    vec![
        (
            "list_counts_gate",
            Action::ListCountsGate,
            "step: the counts of the typed lists: README.md:1: says 3, how many entries `KINDS` holds: name the list, never its count, which its next entry makes wrong",
            |fake| {
                put(
                    fake,
                    "crates/a/src/list.rs",
                    "const KINDS: [&str; 3] = [\"aaa\", \"bbb\", \"ccc\"];\n",
                );
                put(fake, "README.md", "The three `KINDS`.\n");
            },
        ),
        (
            "list_counts_gate",
            Action::ListCountsGate,
            "step: the counts of the typed lists: README.md:1: quotes 3 entries of `KINDS`: name the list, never a copy of its entries, which its next entry makes wrong",
            |fake| {
                put(
                    fake,
                    "crates/a/src/list.rs",
                    "const KINDS: [&str; 3] = [\"aaa\", \"bbb\", \"ccc\"];\n",
                );
                put(fake, "README.md", "Either `aaa`, `bbb` or `ccc`.\n");
            },
        ),
        (
            "register_names_gate",
            Action::RegisterNamesGate,
            "step: proof-bypasses.txt:4: the reason cites gone/b.ts, which no tracked file is (from the root or a directory above a.ts): cite the file as it is now",
            |fake| {
                add(fake, crate::BYPASSES_REVIEWED, "a.ts: x -- see gone/b.ts\n");
            },
        ),
        (
            "register_names_gate",
            Action::RegisterNamesGate,
            "step: mutants-equivalent.txt:2: the reason names `gone_word`, which no tracked code holds: name what the code holds now",
            |fake| {
                put(fake, "crates/a/src/word.rs", "fn held_word() {}\n");
                add(
                    fake,
                    crate::EQUIVALENT_REVIEWED,
                    "a.ts: x -- `held_word` and `gone_word`\n",
                );
            },
        ),
        (
            "hermetic_git_gate",
            Action::HermeticGitGate,
            "step: crates/a/src/git.rs:1: `HERMETIC_GIT` is not xtask's (crates/xtask/src/runner.rs): declare the same entries, in the same order, as literals",
            |fake| {
                put(
                    fake,
                    "crates/a/src/git.rs",
                    "const HERMETIC_GIT: [(&str, &str); 3] = [(\"GIT_CONFIG_NOSYSTEM\", \"1\"), (\"GIT_CONFIG_GLOBAL\", \"/dev/null\"), (\"XDG_CONFIG_HOME\", \"/dev/null\")];\n",
                );
            },
        ),
        (
            "source_kinds_gate",
            Action::SourceKindsGate,
            "step: Makefile: a tracked text file of no known kind: add its pattern to SOURCE_KINDS, with what of it is prose and whether it is code, or why no gate of code reads it (crates/xtask/src/workspace.rs)",
            |fake| {
                put(fake, "Makefile", "all:\n");
            },
        ),
        (
            "retired_patterns_gate",
            Action::RetiredPatternsGate,
            "step: a.md:1: holds `WEAK wording`, which a fix retired: r",
            |fake| {
                put(fake, "a.md", "The WEAK\nwording.\n");
                put(
                    fake,
                    crate::RETIRED_PATTERNS,
                    "retire `WEAK wording` -- r\n",
                );
            },
        ),
        (
            "vocabulary_gate",
            Action::VocabularyGate,
            "step: words the glossary replaces (reviewed in vocabulary.txt): a.md:1: \"%banned%\": say \"box\" (the glossary)",
            |fake| {
                put(fake, "a.md", concat!("A no", "de.\n"));
            },
        ),
    ]
}

/// The rules of the runnable hints gate.
fn hint_rules() -> Vec<Rule> {
    vec![
        (
            "runnable_hints_gate",
            Action::RunnableHintsGate,
            "step: a.rs:1: the hint `%see%x help)` names no command a user can run: write the exact command, starting with one of cargo xtask, cargo run, cargo update, pnpm, git, %banned%",
            |fake| {
                put(
                    fake,
                    "a.rs",
                    &format!("\"{}x help)\"\n", crate::HINT_OPENING),
                );
            },
        ),
        (
            "runnable_hints_gate",
            Action::RunnableHintsGate,
            "step: a.rs:1: a hint opened `( %word%: `: write it `%see%<command>)`, the one form the hints gate reads here",
            |fake| {
                put(fake, "a.rs", concat!("\"( see", ": cargo xtask help)\"\n"));
            },
        ),
        (
            "runnable_hints_gate",
            Action::RunnableHintsGate,
            "step: a.rs:1: the hint `%see%git log <b>)` holds the placeholder `<b>`: name the exact value, one hint per value",
            |fake| {
                put(
                    fake,
                    "a.rs",
                    &format!("\"{}git log <b>)\"\n", crate::HINT_OPENING),
                );
            },
        ),
        (
            "runnable_hints_gate",
            Action::RunnableHintsGate,
            "step: a.rs:1: the hint `%see%git checkout b)` is no git command a hint may name: write one of git checkout HEAD --, git restore --worktree --, git add --, git add --force --, git add --renormalize --, then its paths, or one of git -C \"$(git rev-parse --show-toplevel)\" update-index, git -C \"$(git rev-parse --show-toplevel)\" check-attr, git diff, git log, git rev-parse, git show",
            |fake| {
                put(
                    fake,
                    "a.rs",
                    &format!("\"{}git checkout b)\"\n", crate::HINT_OPENING),
                );
            },
        ),
        (
            "runnable_hints_gate",
            Action::RunnableHintsGate,
            "step: a.rs:1: a message names the command `cargo xtask help` in code quotes: write it as a hint, `%see%cargo xtask help)`, which the hints gate reads",
            |fake| {
                put(fake, "a.rs", "\"run `cargo xtask help`\"\n");
            },
        ),
        (
            "runnable_hints_gate",
            Action::RunnableHintsGate,
            "step: a/README.md:1: the command `x help` of a README's hint is no command a user can run: write it as it is run, starting with one of cargo xtask, cargo run, cargo update, pnpm, git, %banned%",
            |fake| {
                put(
                    fake,
                    "a/README.md",
                    &format!("A {}`x help`)\n", crate::README_HINT_OPENING),
                );
            },
        ),
    ]
}

/// The rules of the citation gate, the changelog's gates and the preflight.
fn changelog_and_machine_rules() -> Vec<Rule> {
    vec![
        (
            "citation_gate",
            Action::CitationGate,
            "step: a.md:1: cites \u{23}999 step 1, but the board has no card \u{23}999",
            |fake| {
                put(fake, "a.md", "See \u{23}999\nstep 1.\n");
            },
        ),
        (
            "changed_names_gate",
            Action::VersionGate,
            "step: the changed and fixed entries of the changelog: CHANGELOG.md:13: a changed or fixed entry cites no name that fed4560000000000000000000000000000000000 (merge base with origin/main) holds (newer): name what it changes as the last version has it %see%git diff --stat fed4560000000000000000000000000000000000), or say it in the entry under `### Added` of what the release adds",
            |fake| {
                let log = fake
                    .files
                    .get(crate::CHANGELOG)
                    .cloned()
                    .unwrap_or_default();
                put(
                    fake,
                    crate::CHANGELOG,
                    &log.replacen("- x\n", "- x\n\n### Fixed\n\n- `newer` alone.\n", 1),
                );
                put(
                    fake,
                    "capture git grep --color=never --no-line-number --no-column --only-matching -h \
                 --text --word-regexp --fixed-strings -e Changelog -e newer \
                 fed4560000000000000000000000000000000000 --",
                    "Changelog\n",
                );
            },
        ),
        (
            "preflight_gate",
            Action::Preflight(crate::GpuDriver::Required),
            "step: the checks cannot run here: npm_config_registry is set, which changes how npm runs the TypeScript checks and the processes their tests start (a setting of every npm and pnpm command, read in any case): unset it",
            |fake| {
                fake.environment.variables = vec!["npm_config_registry".to_owned()];
            },
        ),
        (
            "preflight_gate",
            Action::Preflight(crate::GpuDriver::Required),
            "step: the checks cannot run here: /home/a/.npmrc configures every npm or pnpm command here: move it away (the configuration of the repository is pnpm-workspace.yaml)",
            |fake| {
                fake.environment.node_configs = vec!["/home/a/.npmrc".to_owned()];
            },
        ),
    ]
}

/// Every rule of the gates of the check.
fn rules() -> Vec<Rule> {
    [
        rust_rules(),
        dependency_rules(),
        build_rules(),
        package_rules(),
        test_code_rules(),
        register_and_list_rules(),
        hint_rules(),
        changelog_and_machine_rules(),
    ]
    .concat()
}

#[test]
fn each_gate_refuses_through_the_step_that_runs_it() {
    // Follow-up of the gates of the fifth deep review of visual block 2 (#34): each had its own
    // fixtures, and `safe_vectors_gate` could be emptied with no test failing. One edit of the
    // fake repository per rule, and the exact words of the step: where a gate reads two like
    // inputs, the edit is of the one a wrong call would not read.
    for (gate, action, said, edit) in rules() {
        let step = [Step {
            name: "step".to_owned(),
            action,
        }];
        let mut fake = passing();
        assert_eq!(
            run_steps(&step, &mut fake, &mut Vec::new()),
            Ok(()),
            "{gate}: the fake repository as it is"
        );
        edit(&mut fake);
        let refused = run_steps(&step, &mut fake, &mut Vec::new()).map_err(|failure| {
            let mut lines = failure.details;
            lines.push(failure.summary);
            lines.join("\n")
        });
        let said = said
            .replace("%see%", crate::HINT_OPENING)
            .replace("%word%", "see")
            .replace("%banned%", concat!("no", "de"));
        assert_eq!(refused, Err(said), "{gate}");
    }
}

/// The gates a step doing `action` runs: every action is classed here, with no wildcard, so
/// that a new one does not compile before it is, and
/// [`holds_every_gate_of_the_check_to_a_refusal`] then asks for the refusal of each.
///
/// A gate is a function named `<name>_gate`, a convention (a gate named otherwise is review
/// only), or one of the two functions that judge a step of no such gate.
const fn gates_of(action: &Action) -> &'static [&'static str] {
    match action {
        Action::TestCodeGate => &["test_code_gate", "exact_errors_gate"],
        Action::TestDataGate => &["test_data_gate", "safe_vectors_gate"],
        Action::UntestedTargetsGate => &["untested_targets_gate", "compile_fail_codes_gate"],
        Action::UnviableNamesGate => &[
            "unviable_names_gate",
            "unviable_reasons_gate",
            "review_names_gate",
            "review_reasons_gate",
            "spec_names_gate",
            "reader_tests_gate",
        ],
        Action::PinGate => &["pin_gate"],
        Action::LibraryGate => &["library_gate", "shared_tests_gate"],
        Action::PartLicenseGate => &["part_license_gate"],
        Action::CitationGate => &["citation_gate"],
        Action::ListCountsGate => &["list_counts_gate"],
        Action::ConflictMarkerGate => &["conflict_marker_gate"],
        Action::SourceKindsGate => &["source_kinds_gate"],
        Action::RetiredPatternsGate => &["retired_patterns_gate"],
        Action::RunnableHintsGate => &["runnable_hints_gate"],
        Action::HermeticGitGate => &["hermetic_git_gate"],
        Action::RegisterNamesGate => &["register_names_gate"],
        Action::LoadedFilesGate => &["loaded_files_gate", "validated_shaders_gate"],
        Action::BypassGate => &["bypass_gate"],
        Action::CargoConfigGate => &[
            "cargo_config_gate",
            "gpu_manifest_gate",
            "nextest_configs_gate",
        ],
        Action::DependencyGate => &["dependency_gate"],
        Action::EquivalentGate => &["equivalent_gate"],
        Action::FloatMethodGate => &["float_gate"],
        Action::HistoryGate => &["pins_gate", "history_gate", "breaking_bang_gate"],
        Action::LicenseGate => &["license_gate", "shipped_exceptions_gate"],
        Action::MathLayerGate => &["named_sources_gate", "math_layer_gate"],
        Action::MembershipGate => &["membership_gate"],
        Action::PartsGate => &["parts_gate"],
        Action::ReadmeGate => &["readme_gate", "spec_index_gate", "module_table_gate"],
        Action::StrykerConfigGate => &["stryker_config_gate", "stryker_scripts_gate"],
        Action::PackageCheckGate => &[
            "package_check_gate",
            "package_order_gate",
            "test_exports_gate",
            "shared_constants_gate",
            "linked_conformance_gate",
        ],
        Action::SupplyChainGate => &["supply_chain_gate"],
        Action::CpuReferenceGate => &["cpu_reference_gate"],
        Action::BuildTimeGate => &["build_time_gate", "shared_versions_gate"],
        Action::TypeScriptMutationGate { .. } => &["ts_mutation_gate"],
        Action::TypeScriptReportsGate(_) => &["ts_coverage_gate", "ts_mutation_gate"],
        Action::TypeScriptScopeGate(_) => &["scope_gate"],
        Action::UnsafeGate => &["unsafe_gate"],
        Action::LibraryLintsGate => &["library_lints_gate", "no_std_portability_gate"],
        Action::UnviableGate(_) | Action::ShardsUnviableGate(_) => &["unviable_gate"],
        Action::UnviableReviewsCurrent => &["unviable_reviews_gate"],
        Action::VersionGate => &[
            "release_gate",
            "unreleased_shape_gate",
            "duplicate_entries_gate",
            "breaking_spellings_gate",
            "released_sections_gate",
            "breaking_names_gate",
            "removed_names_gate",
            "changed_names_gate",
        ],
        Action::VocabularyGate => &["vocabulary_gate"],
        Action::WorkflowGate => &["workflow_gate"],
        Action::Coverage(_) => &["coverage_gate"],
        Action::CoverageLocality => &["coverage_locality"],
        Action::CorpusMutants(_) => &["corpus_gate", "corpus_sources_gate"],
        Action::FusedBuild => &["fused_gate"],
        Action::DocTests => &["doc_tests"],
        Action::TreeUnchanged => &["tree_gate"],
        Action::MutantLeftGate => &["mutant_left_gate"],
        Action::Preflight(_) => &["preflight_gate"],
        Action::Run(_)
        | Action::ToolGate(_)
        | Action::RustMutants(_)
        | Action::GpuMutants(_)
        | Action::RustMutantsOfChanges { .. }
        | Action::RustMutantsOfChangedLines { .. }
        | Action::Bless(_)
        | Action::Conformance
        | Action::MutationProofRecorded
        | Action::FreshDirs(_)
        | Action::TreeRecorded => &[],
    }
}

/// The tests that see a gate of the check no rule of [`rules`] holds refuse through the step
/// that runs it, each with the gates it holds.
///
/// But the parts gate's, which shows its function alone: its step reads constants, which no
/// test edits, so that the arm of `run_action` running it is review only.
const HELD_ELSEWHERE: [(&str, &[&str]); 29] = [
    (
        "names_a_failed_step_once_as_the_user_reads_it",
        &["test_code_gate"],
    ),
    (
        "reads_the_reviews_of_the_last_version_and_the_specifications",
        &[
            "unviable_reasons_gate",
            "review_names_gate",
            "review_reasons_gate",
            "spec_names_gate",
        ],
    ),
    (
        "reads_a_disable_comment_in_every_kind_stryker_mutates",
        &["equivalent_gate"],
    ),
    ("checks_the_pins_in_the_history_of_main", &["pins_gate"]),
    (
        "runs_the_breaking_rule_with_the_history",
        &["breaking_bang_gate"],
    ),
    (
        "reports_readme_problems_whatever_the_membership",
        &["membership_gate", "readme_gate"],
    ),
    (
        "checks_the_parts_of_the_full_check_as_a_step",
        &["parts_gate"],
    ),
    (
        "holds_the_tracked_specifications_to_their_index",
        &["spec_index_gate"],
    ),
    (
        "the_stryker_step_reads_every_manifest",
        &["stryker_scripts_gate"],
    ),
    (
        "reads_the_manifest_of_every_package_for_the_package_check_gate",
        &["package_check_gate"],
    ),
    (
        "reads_every_package_s_sources_for_the_shared_constants_gate",
        &["shared_constants_gate"],
    ),
    (
        "reads_the_crate_and_tests_of_each_package_for_the_linked_conformance_gate",
        &["linked_conformance_gate"],
    ),
    (
        "fails_when_a_supply_chain_file_cannot_be_read",
        &["supply_chain_gate"],
    ),
    (
        "fails_on_a_missing_or_failing_typescript_report",
        &["ts_mutation_gate"],
    ),
    (
        "fails_when_unviable_files_are_missing_or_unreviewed",
        &["unviable_gate"],
    ),
    (
        "the_default_check_refuses_stale_reviews_of_unviable_mutants",
        &["unviable_reviews_gate"],
    ),
    ("release_gate_runs_on_the_real_files", &["release_gate"]),
    (
        "reports_every_gate_of_the_version_step_that_fails",
        &[
            "unreleased_shape_gate",
            "duplicate_entries_gate",
            "breaking_spellings_gate",
        ],
    ),
    (
        "reads_the_released_sections_against_main",
        &["released_sections_gate"],
    ),
    (
        "searches_the_breaking_names_against_main",
        &["breaking_names_gate"],
    ),
    (
        "searches_the_removed_names_against_main",
        &["removed_names_gate"],
    ),
    (
        "says_where_the_workflow_differs_from_xtask_s",
        &["workflow_gate"],
    ),
    (
        "fails_on_missing_or_incomplete_coverage",
        &["coverage_gate"],
    ),
    (
        "requires_the_gate_to_refuse_a_path_only_an_integration_test_reaches",
        &["coverage_locality"],
    ),
    (
        "accepts_missed_mutants_its_gate_then_judges",
        &["corpus_gate"],
    ),
    (
        "requires_every_source_of_the_rendering_code_mutated_or_left_out_with_a_reason",
        &["corpus_sources_gate"],
    ),
    (
        "requires_the_fused_build_to_have_run_with_its_option",
        &["fused_gate"],
    ),
    (
        "refuses_checks_that_change_the_working_tree",
        &["tree_gate"],
    ),
    ("refuses_a_gpu_source_left_mutated", &["mutant_left_gate"]),
];

/// The functions of xtask's code named as gates that no step of the check runs: builders and
/// predicates of steps, gates of other commands, and the bodies of gates above.
const OTHER_GATES: [&str; 7] = [
    "cargo_gate",
    "commit_msg_gate",
    "crates_test_data_gate",
    "is_static_gate",
    "pre_push_gate",
    "reviewed_errors_gate",
    "tool_gate",
];

#[test]
fn holds_every_gate_of_the_check_to_a_refusal() {
    // Finding K5-2 of the sixth deep review of visual block 2 (#34): twenty-seven gates of the
    // check were seen refusing through no step, so each could be emptied, left out of its
    // step or given the wrong input with no test failing.
    let run: BTreeSet<&str> = [Mode::Full { jobs: FAKE_JOBS }, CHANGES_MODE]
        .into_iter()
        .flat_map(check_steps)
        .flat_map(|step| gates_of(&step.action))
        .copied()
        .collect();
    let mut held = BTreeSet::new();
    for (gate, action, _, _) in rules() {
        assert!(gates_of(&action).contains(&gate), "{gate}: {action:?}");
        let _again = held.insert(gate);
    }
    let tests = own_sources(true);
    for (test, gates) in HELD_ELSEWHERE {
        for gate in gates {
            assert!(held.insert(gate), "{gate}: held twice");
        }
        assert!(
            tests.iter().any(|(_, tokens)| tokens
                .windows(2)
                .any(|pair| pair[0].is_ident("fn") && pair[1].is_ident(test))),
            "{test}: a test of xtask"
        );
    }
    assert_eq!(held, run, "the gates held, the gates of the steps");
    // Every gate the code names is a step's, or one of the few that are not.
    let named: BTreeSet<String> = own_sources(false)
        .iter()
        .flat_map(|(_, tokens)| tokens)
        .filter(|token| token.kind == TokenKind::Ident && token.text.ends_with("_gate"))
        .map(|token| token.text.clone())
        .collect();
    let known: BTreeSet<String> = run
        .into_iter()
        .filter(|gate| gate.ends_with("_gate"))
        .chain(OTHER_GATES)
        .map(str::to_owned)
        .collect();
    assert_eq!(named, known, "the gates the code names, the gates known");
}
