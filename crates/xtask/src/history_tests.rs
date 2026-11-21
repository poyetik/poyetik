//! Tests of the history gates: pinned commits, the history of the branch, and the pre-push gate.

use crate::tests::{
    BREAKING_LISTED, CHANGES_MODE, HISTORY, HISTORY_FILES, HISTORY_LINES, HISTORY_SCOPED,
    LICENSE_LINES, PINS, failed, found, hermetic_git, passing, scratch, system,
};
use crate::{
    Action, CHANGELOG, Cmd, Dir, Failure, Runner, Step, check_steps, commit_msg_gate, run_steps,
};

#[test]
fn tells_changes_of_behavior() {
    for (message, expected) in [
        ("feat: add", true),
        ("fix(roadmap): repair", true),
        ("perf(audio-core): faster", true),
        ("docs!: drop a page", true),
        ("refactor(roadmap)!: rename", true),
        (
            "refactor: rename\n\nBREAKING CHANGE: the command is renamed",
            true,
        ),
        ("docs: explain", false),
        ("test(xtask): cover", false),
        ("build: bump", false),
        ("chore: tidy\n\nnot a BREAKING CHANGE: inline", false),
        ("feature: not a type", false),
        ("", false),
    ] {
        assert_eq!(crate::changes_behavior(message), expected, "{message:?}");
    }
}

#[test]
fn checks_every_message_and_the_changelog_of_every_change() {
    assert_eq!(
        crate::history_gate(HISTORY, HISTORY_FILES, HISTORY_SCOPED),
        Ok(()),
        "a clean history"
    );
    let messages = "\u{1e}cccccccccc\u{1f}fix: repair\n\u{1e}bbbbbbbbbb\u{1f}Update stuff\n\u{1e}aaaaaaaaaa\u{1f}feat: before the rule\n";
    let files = "\u{1e}cccccccccc\n\nsrc/a.rs\n\u{1e}bbbbbbbbbb\n\nsrc/b.rs\n";
    assert_eq!(
        crate::history_gate(messages, files, ""),
        failed(
            "the history breaks the commit rules: 2 problems, listed above\n\
             commit ccccccc changes behavior but not CHANGELOG.md\n\
             commit bbbbbbb: invalid commit message: missing \"<type>: \" prefix"
        ),
        "a change without its entry, a malformed message; the commit before the rule is not held to it"
    );
    assert_eq!(crate::history_gate("", "", ""), Ok(()), "no commit");
    let named = "\u{1e}cccccccccc\u{1f}docs(wasm): x\n\u{1e}bbbbbbbbbb\u{1f}docs(wasm): y\n";
    assert_eq!(
        crate::history_gate(named, "", "cccccccccc\n"),
        Err(Failure {
            summary: format!(
                "the history breaks the commit rules: commit ccccccc: invalid commit message: \
                 unknown scope \"wasm\": name a crate, a package or a scope of xtask's \
                 COMMIT_SCOPE_EXTRAS ({})",
                crate::commit_scopes().join(", ")
            ),
            details: vec![],
            usage: false,
        }),
        "a scope held to the registries only after the scopes' boundary"
    );
    assert_eq!(
        crate::history_gate("\u{1e}eeeeeeeeee\u{1f}# fix: hidden\nfix: shown\n", "", ""),
        found(
            "the history breaks the commit rules",
            &["commit eeeeeee: invalid commit message: unknown type \"# fix\"".to_owned()]
        ),
        "a recorded message is read as it is: a first line starting with # is its header"
    );
    assert_eq!(
        crate::history_gate("\u{1e}dddddddddd", "\u{1e}dddddddddd\n", ""),
        found(
            "the history breaks the commit rules",
            &["commit ddddddd: invalid commit message: missing \"<type>: \" prefix".to_owned()]
        ),
        "a record without a message"
    );
    assert_eq!(crate::CHANGELOG_SINCE.len(), 40, "a full commit hash");
}

#[test]
fn pins_the_history_gate_to_exactly() {
    assert_eq!(
        crate::history_pins(),
        PINS.to_vec(),
        "the scopes' boundary, the changelog's, the breaking rule's, then each exempt marker"
    );
    assert_eq!(
        crate::pins_command("fed456").line(),
        "git log --format=%H fed456",
        "the base's history, one full hash per line"
    );
}

#[test]
fn refuses_a_pin_main_does_not_hold() {
    let base = ("fed4560", "origin/main");
    let held = "aaaaaaaaaa\nbbbbbbbbbb\ncccccccccc\n";
    let pins = [
        ("FIRST", "aaaaaaaaaa"),
        ("SECOND", "bbbbbbbbbb"),
        ("THIRD", "cccccccccc"),
    ];
    assert_eq!(
        crate::pins_gate(&pins, held, base),
        Ok(()),
        "every pin held"
    );
    assert_eq!(crate::pins_gate(&[], "", base), Ok(()), "no pin");
    for missing in 0..pins.len() {
        let mut shown = pins;
        shown[missing].1 = "dddddddddd";
        assert_eq!(
            crate::pins_gate(&shown, held, base),
            failed(format!(
                "the history gate is pinned to commits main does not hold: {} names \
                 dddddddddd, which is not in the history of fed4560 (merge base with \
                 origin/main): pin a commit main holds",
                pins[missing].0
            )),
            "{}: named by its constant",
            pins[missing].0
        );
    }
    assert_eq!(
        crate::pins_gate(&pins, "aaaaaaaaaa\n", base),
        found(
            "the history gate is pinned to commits main does not hold",
            &[
                "SECOND names bbbbbbbbbb, which is not in the history of fed4560 (merge base \
                 with origin/main): pin a commit main holds"
                    .to_owned(),
                "THIRD names cccccccccc, which is not in the history of fed4560 (merge base \
                 with origin/main): pin a commit main holds"
                    .to_owned()
            ]
        ),
        "every pin missing, each on its line"
    );
    assert_eq!(
        crate::pins_gate(
            &[("FIRST", "aaaaaaaaaa")],
            "aaaaaaaaaab\nbaaaaaaaaaa\n",
            base
        ),
        failed(
            "the history gate is pinned to commits main does not hold: FIRST names \
             aaaaaaaaaa, which is not in the history of fed4560 (merge base with \
             origin/main): pin a commit main holds"
        ),
        "a whole line, never a part of one"
    );
}

#[test]
fn checks_the_pins_git_really_lists() {
    let repo = scratch("pins");
    let _init = hermetic_git(&repo, &["init", "--quiet", "--initial-branch=main"]);
    let commit = |message: &str| {
        let _commit = hermetic_git(
            &repo,
            &["commit", "--quiet", "--allow-empty", "-m", message],
        );
        hermetic_git(&repo, &["rev-parse", "HEAD"])
            .trim()
            .to_owned()
    };
    let on_main = commit("docs: on main");
    let _branch = hermetic_git(&repo, &["checkout", "--quiet", "-b", "work"]);
    let rewritten = commit("docs: on the branch");
    let _amend = hermetic_git(
        &repo,
        &[
            "commit",
            "--quiet",
            "--amend",
            "--allow-empty",
            "-m",
            "docs: amended",
        ],
    );
    let only_branch = commit("docs: on the branch only");
    let mut runner = system(repo.path());
    let base = runner
        .capture(&Cmd::new("git", &["merge-base", "HEAD", "main"], Dir::Root))
        .expect("merge base");
    let ancestors = runner
        .capture(&crate::pins_command(base.trim()))
        .expect("git log");
    let at = (base.trim(), "main");
    assert_eq!(
        crate::pins_gate(&[("MAIN", &on_main)], &ancestors, at),
        Ok(()),
        "a commit of main"
    );
    for (name, pin) in [("REWRITTEN", &rewritten), ("BRANCH", &only_branch)] {
        assert_eq!(
            crate::pins_gate(&[(name, pin)], &ancestors, at),
            failed(format!(
                "the history gate is pinned to commits main does not hold: {name} names \
                 {pin}, which is not in the history of {} (merge base with main): pin a \
                 commit main holds",
                base.trim()
            )),
            "{name}: its object still exists, but main does not hold it"
        );
    }
}

/// What git says of a work tree whose status is `status`, whose files with their tags are
/// `listed`, and which no filter rewrites.
fn view(status: &str, listed: &str) -> crate::TreeView {
    crate::TreeView {
        status: status.to_owned(),
        listed: listed.to_owned(),
        filtered: None,
    }
}

#[test]
fn pushes_only_what_was_checked() {
    let head = "1111111111111111111111111111111111111111\n";
    let pushed = |commits: &[&str]| {
        commits
            .iter()
            .map(|&commit| commit.to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        crate::pre_push_gate(
            head,
            &view("", "H a.rs\0H b.rs\0"),
            &pushed(&["1111111111111111111111111111111111111111"])
        ),
        Ok(()),
        "the checked-out commit"
    );
    for (hidden, path, flag) in [
        ("h a.rs\0", "a.rs", "--no-assume-unchanged"),
        ("H b.rs\0S c d.rs\0", "c d.rs", "--no-skip-worktree"),
    ] {
        assert_eq!(
            crate::pre_push_gate(head, &view("", hidden), &[]),
            failed(format!(
                "the push cannot be checked: the working tree may differ from what is \
                 committed, which the check would include: `{path}` is hidden from git status \
                 (assume-unchanged or skip-worktree), so no diff of git shows its changes: clear \
                 it with `git update-index {flag} -- '{path}'`"
            )),
            "{hidden:?}"
        );
    }
    assert_eq!(
        crate::pre_push_gate(
            head,
            &view("", ""),
            &pushed(&["0000000000000000000000000000000000000000"])
        ),
        Ok(()),
        "a deleted branch"
    );
    assert_eq!(
        crate::pre_push_gate(head, &view("\n", ""), &[]),
        Ok(()),
        "nothing pushed, nothing changed"
    );
    assert_eq!(
        crate::pre_push_gate(
            head,
            &view(" M src/a.rs\n?? notes.txt\n", "h hidden.rs\0"),
            &pushed(&["2222222222222222222222222222222222222222"])
        ),
        failed(
            "the push cannot be checked: 2 problems, listed above\n\
             the working tree has changes not committed, which the check would include: M src/a.rs, ?? notes.txt\n\
             pushing 2222222222222222222222222222222222222222, but 1111111111111111111111111111111111111111 \
             is checked out: check out the pushed commit, then push again"
        ),
        "a dirty tree, another commit"
    );
}

#[test]
fn checks_the_history_git_really_writes() {
    let repo = scratch("history");
    let _init = hermetic_git(&repo, &["init", "--quiet"]);
    let commit = |message: &str, file: Option<&str>| {
        if let Some(path) = file {
            std::fs::write(repo.join(path), message).expect("file written");
            let _add = hermetic_git(&repo, &["add", path]);
        }
        let _commit = hermetic_git(
            &repo,
            &["commit", "--quiet", "--allow-empty", "-m", message],
        );
        hermetic_git(&repo, &["rev-parse", "HEAD"])
            .trim()
            .to_owned()
    };
    let _before = commit("fix: before the boundary", Some("a.rs"));
    let since = commit("fix: the boundary itself", Some("b.rs"));
    let _free = commit("docs(wasm): before the scopes' boundary", None);
    let scopes_since = commit("docs(wasm): the scopes' boundary itself", None);
    let _recorded = commit("fix: recorded\n\nWhy.", Some(CHANGELOG));
    let unrecorded = commit("fix: unrecorded", Some("c.rs"));
    let malformed = commit("Update stuff", None);
    let unscoped = commit("docs(wasm): a part named two ways", None);
    let mut runner = system(repo.path());
    let [messages, files, scoped] = crate::history_commands(&since, &scopes_since)
        .map(|cmd| runner.capture(&cmd).expect("git log"));
    let short = |hash: &str| hash.chars().take(7).collect::<String>();
    assert_eq!(
        crate::history_gate(&messages, &files, &scoped),
        failed(format!(
            "the history breaks the commit rules: 3 problems, listed above\n\
             commit {}: invalid commit message: unknown scope \"wasm\": name a crate, a \
             package or a scope of xtask's COMMIT_SCOPE_EXTRAS ({})\n\
             commit {}: invalid commit message: missing \"<type>: \" prefix\n\
             commit {} changes behavior but not CHANGELOG.md",
            short(&unscoped),
            crate::commit_scopes().join(", "),
            short(&malformed),
            short(&unrecorded)
        )),
        "every message checked; the changelog rule from the commit after its boundary on, \
         the scopes from the commit after theirs"
    );
}

#[test]
fn license_and_history_commands_are_exactly() {
    assert_eq!(
        crate::licenses::license_commands().map(|cmd| cmd.line()),
        LICENSE_LINES.map(str::to_owned),
        "every shipped package filtered, then everything"
    );
    assert_eq!(
        crate::history_commands(crate::CHANGELOG_SINCE, crate::SCOPES_SINCE).map(|cmd| cmd.line()),
        HISTORY_LINES.map(str::to_owned),
        "every message, the files after the boundary, the commits after the scopes' boundary"
    );
    assert!(
        crate::licenses::license_commands()
            .iter()
            .chain(&crate::history_commands(
                crate::CHANGELOG_SINCE,
                crate::SCOPES_SINCE
            ))
            .all(|cmd| cmd.dir == Dir::Root),
        "run at the root"
    );
}

#[test]
fn accepts_the_synthetic_merge_of_a_pull_request_only_as_the_commit_checked_out() {
    let (head, base) = ("a".repeat(40), "0123456789abcdef0123456789abcdef01234567");
    let merge = format!("Merge {head} into {base}\n");
    let record = |hash: &str, message: &str| format!("\u{1e}{hash}\u{1f}{message}");
    for message in [merge.clone(), format!("{merge}\n")] {
        let merged = format!("{}{HISTORY}", record("ffffffffff", &message));
        assert_eq!(
            crate::history_gate(&merged, HISTORY_FILES, ""),
            Ok(()),
            "the merge GitHub checks out for a pull request, at HEAD: {message:?}"
        );
    }
    let refused = |hash: &str| {
        found(
            "the history breaks the commit rules",
            &[format!(
                "commit {hash}: invalid commit message: missing \"<type>: \" prefix"
            )],
        )
    };
    let below = format!(
        "{}{}{HISTORY}",
        record("cccccccccc", "docs: x\n"),
        record("ffffffffff", &merge)
    );
    assert_eq!(
        crate::history_gate(&below, HISTORY_FILES, ""),
        refused("fffffff"),
        "anywhere but HEAD, the same message is refused"
    );
    for message in [
        "Merge branch 'x' into main\n".to_owned(),
        format!("Merge {head} into {}\n", base.get(..39).unwrap_or_default()),
        format!("Merge {} into {base}\n", head.to_uppercase()),
        format!("Merge {head} into {base}\n\nmore\n"),
        format!("Merge {head} into {base}g\n"),
        format!("Merge {head} into {base} \n"),
        format!("Merge {head}  into {base}\n"),
        format!("merge {head} into {base}\n"),
    ] {
        assert_eq!(
            crate::history_gate(&record("ffffffffff", &message), "", ""),
            refused("fffffff"),
            "{message:?}"
        );
    }
    assert!(
        commit_msg_gate(&merge).is_err(),
        "a commit written here never has that message"
    );
}

/// A record of the breaking rule's listing: a commit, its header, then its diff lines.
fn breaking_record(hash: &str, header: &str, diff: &[&str]) -> String {
    format!("\u{1e}{hash}\u{1f}{header}\n{}", diff.join("\n"))
}

#[test]
fn refuses_a_commit_adding_a_breaking_entry_without_its_bang() {
    let added = ["+- **Breaking:** `f` takes a second argument."];
    let refused = |hash: &str| {
        format!(
            "commit {hash}: it adds a breaking entry to CHANGELOG.md, so its header carries `!` \
             (`type(scope)!: summary`)"
        )
    };
    for (header, why) in [
        ("feat(audio-core)!: add", "a breaking feature"),
        ("refactor!: move", "breaking without a scope"),
    ] {
        assert_eq!(
            crate::breaking_bang_gate(&breaking_record("aaaaaaaaaa", header, &added)),
            Ok(()),
            "{why}"
        );
    }
    for (diff, why) in [
        (
            vec![
                "-- **Breaking:** `f` takes an argument.",
                "+- **Breaking:** `f` takes a second argument.",
            ],
            "an entry reworded",
        ),
        (
            vec!["-- **Breaking:** `f` takes an argument."],
            "an entry removed",
        ),
        (
            vec!["+- `f` is faster.", "+  - **Breaking** inside an entry"],
            "no breaking entry added",
        ),
        (
            vec!["--- a/CHANGELOG.md", "+++ b/CHANGELOG.md"],
            "the diff's own header lines",
        ),
        (vec![], "a commit with no changelog line"),
    ] {
        assert_eq!(
            crate::breaking_bang_gate(&breaking_record("aaaaaaaaaa", "docs: x", &diff)),
            Ok(()),
            "{why}"
        );
    }
    assert_eq!(
        crate::breaking_bang_gate(""),
        Ok(()),
        "no commit after the boundary"
    );
    for (header, why) in [
        ("feat(audio-core): add", "a feature not marked"),
        ("docs(changelog): move", "a docs commit"),
        ("feat: add!", "a `!` in the summary only"),
        ("Update stuff!", "a header with no prefix"),
    ] {
        assert_eq!(
            crate::breaking_bang_gate(&breaking_record("bbbbbbbbbb", header, &added)),
            failed(format!(
                "the history breaks the breaking rule: {}",
                refused("bbbbbbb")
            )),
            "{why}"
        );
    }
    let two = [
        "+- **Breaking:** `f` takes a second argument.",
        "+- **Breaking:** `g` is gone.",
        "-- **Breaking:** `h` takes an argument.",
    ];
    let log = format!(
        "{}{}{}",
        breaking_record("cccccccccc", "fix: x", &two),
        breaking_record("dddddddddd", "fix!: y", &two),
        breaking_record("eeeeeeeeee", "chore: z", &added)
    );
    assert_eq!(
        crate::breaking_bang_gate(&log),
        found(
            "the history breaks the breaking rule",
            &[refused("ccccccc"), refused("eeeeeee")]
        ),
        "more added than removed, each commit named"
    );
}

#[test]
fn checks_the_breaking_entries_git_really_lists() {
    let repo = scratch("breaking");
    let _init = hermetic_git(&repo, &["init", "--quiet"]);
    let commit = |message: &str, changelog: &str| {
        std::fs::write(repo.join(CHANGELOG), changelog).expect("changelog written");
        let _add = hermetic_git(&repo, &["add", CHANGELOG]);
        let _commit = hermetic_git(
            &repo,
            &["commit", "--quiet", "--allow-empty", "-m", message],
        );
        hermetic_git(&repo, &["rev-parse", "HEAD"])
            .trim()
            .to_owned()
    };
    let _before = commit(
        "docs: before the boundary\n\nWhy.",
        "# Changelog\n\n- **Breaking:** `a` before.\n",
    );
    let since = commit(
        "docs: the boundary itself",
        "# Changelog\n\n- **Breaking:** `a`.\n",
    );
    let _marked = commit(
        "feat!: break b",
        "# Changelog\n\n- **Breaking:** `a`.\n- **Breaking:** `b`.\n",
    );
    let _reworded = commit(
        "docs: reword a",
        "# Changelog\n\n- **Breaking:** `a`, reworded.\n- **Breaking:** `b`.\n",
    );
    let unmarked = commit(
        "docs: break c",
        "# Changelog\n\n- **Breaking:** `a`, reworded.\n- **Breaking:** `b`.\n- **Breaking:** `c`.\n",
    );
    let mut runner = system(repo.path());
    let log = runner
        .capture(&crate::breaking_log_command(&since))
        .expect("git log");
    assert_eq!(
        crate::breaking_bang_gate(&log),
        failed(format!(
            "the history breaks the breaking rule: commit {}: it adds a breaking entry to \
             CHANGELOG.md, so its header carries `!` (`type(scope)!: summary`)",
            unmarked.chars().take(7).collect::<String>()
        )),
        "the commits after the boundary only, a rewording accepted"
    );
    // Finding H11-1 of the fifth deep review of visual block 2 (#34): an attribute marking
    // the changelog binary turned each diff into one `Binary files … differ` line, which
    // the gate passed, finding no entry.
    std::fs::write(repo.join(".git/info/attributes"), "*.md binary\n").expect("attributes");
    assert_eq!(
        crate::breaking_bang_gate(
            &runner
                .capture(&crate::breaking_log_command(&since))
                .expect("git log")
        ),
        crate::breaking_bang_gate(&log),
        "read as text whatever an attribute says"
    );
    assert_eq!(
        crate::breaking_bang_gate(&format!(
            "\u{1e}{unmarked}\u{1f}docs: break c\n\nBinary files a/CHANGELOG.md and \
             b/CHANGELOG.md differ\n"
        )),
        failed(format!(
            "the history breaks the breaking rule: commit {}: its diff of CHANGELOG.md holds a \
             line the gate cannot read: `Binary files a/CHANGELOG.md and b/CHANGELOG.md differ`",
            unmarked.chars().take(7).collect::<String>()
        )),
        "a diff in another form refused, never passed"
    );
}

#[test]
fn runs_the_breaking_rule_with_the_history() {
    let step: Vec<Step> = check_steps(CHANGES_MODE)
        .into_iter()
        .filter(|step| step.action == Action::HistoryGate)
        .collect();
    assert_eq!(
        crate::breaking_log_command(crate::BREAKING_SINCE).line(),
        BREAKING_LISTED.0,
        "the commits after the boundary, with their changelog diffs"
    );
    let mut fake = passing();
    assert_eq!(run_steps(&step, &mut fake, &mut Vec::new()), Ok(()), "none");
    let _refusing = fake.files.insert(
        format!("capture {}", BREAKING_LISTED.0),
        breaking_record("ffffffffff", "docs: x", &["+- **Breaking:** `f`."]),
    );
    assert_eq!(
        run_steps(&step, &mut fake, &mut Vec::new()),
        failed(
            "Commit messages, and a changelog entry for each change of behavior: the history \
             breaks the breaking rule: commit fffffff: it adds a breaking entry to \
             CHANGELOG.md, so its header carries `!` (`type(scope)!: summary`)"
        ),
        "a commit refused"
    );
    let _unread = fake.files.remove(&format!("capture {}", BREAKING_LISTED.0));
    assert!(
        run_steps(&step, &mut fake, &mut Vec::new()).is_err_and(|failure| failure
            .summary
            .starts_with(
                "Commit messages, and a changelog entry for each change of behavior: cannot \
                 read the history: `git log --format=%x1e%H%x1f%s"
            )),
        "a listing git cannot give"
    );
}

#[test]
fn reads_the_breaking_mark_of_a_header_one_way() {
    for (header, read) in [
        ("feat(audio-core)!: summary", ("feat(audio-core)", true)),
        ("feat!: summary", ("feat", true)),
        ("feat(audio-core): summary!", ("feat(audio-core)", false)),
        ("fix: a: b!: c", ("fix", false)),
        ("no colon!", ("no colon!", false)),
        ("", ("", false)),
    ] {
        assert_eq!(crate::history::header_prefix(header), read, "{header}");
    }
}
