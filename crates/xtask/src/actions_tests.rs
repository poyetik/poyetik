//! Tests of running the steps: the inputs each action reads and how its verdict is said.

use core::num::NonZeroUsize;
use core::time::Duration;
use std::io;

use crate::tests::{
    Broken, CHANGES, CHANGES_MODE, FAKE_JOBS, Fake, READMES, failed, found, hermetic_git,
    one_line_change, parts_refused, passing, scratch, system,
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
    let readmes: Vec<(String, String)> = READMES
        .iter()
        .map(|&(path, text)| (path.to_owned(), text.to_owned()))
        .collect();
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
        action: Action::TypeScriptScopeGate(PACKAGES[4]),
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
