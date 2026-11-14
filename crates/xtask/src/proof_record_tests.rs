//! Tests of the mutation proof record: what a clean default check proved, reused only when every
//! input is unchanged.

use crate::tests::{
    COMMON_DIR, Fake, HIDDEN, HOST, MERGE_BASE, PROVEN, PUSH_STATUS, TRACKED, TREE_HEAD, UNTRACKED,
    diff_since, failed, fake_tools, hermetic_git, mutate_changes, passing, proof_record,
    record_path, rust_mutation_of_changes, scratch, since_base, system,
};
use crate::{Action, Cmd, Dir, Runner, Step, SystemRunner, run_steps};

#[test]
fn refuses_to_mutate_a_tree_where_a_filter_may_hide_changes() {
    let attributes = "git check-attr -z filter -- Cargo.toml crates/a/src/b.rs";
    let filtered = |answer: &str| {
        let mut fake = passing();
        for (command, text) in [
            (TRACKED.0, "Cargo.toml\0crates/a/src/b.rs\0"),
            (attributes, answer),
        ] {
            drop(
                fake.files
                    .insert(format!("capture {command}"), text.to_owned()),
            );
        }
        fake
    };
    let mut hiding = filtered("Cargo.toml\0filter\0unspecified\0crates/a/src/b.rs\0filter\0hide\0");
    assert_eq!(
        run_steps(&rust_mutation_of_changes(), &mut hiding, &mut Vec::new()),
        failed(
            "Rust mutation testing of the changes: `crates/a/src/b.rs` passes through git's \
             `hide` filter, whose clean driver may hide a change from git (status, diff): the \
             tree checked may not be the one committed: remove its `filter` attribute (from \
             `.gitattributes`, `info/attributes` in the directory `git rev-parse \
             --git-common-dir` names or the file `core.attributesFile` names)"
        ),
        "a filtered file"
    );
    assert!(
        !hiding
            .ran
            .iter()
            .any(|line| line.starts_with("cargo mutants")),
        "nothing mutated"
    );
    let mut plain = filtered("Cargo.toml\0filter\0unspecified\0crates/a/src/b.rs\0filter\0unset\0");
    assert_eq!(
        run_steps(&rust_mutation_of_changes(), &mut plain, &mut Vec::new()),
        Ok(()),
        "no filter"
    );
    for (broken, doing) in [
        (TRACKED.0, "list the tracked files"),
        (attributes, "read the filters of the tracked files"),
    ] {
        let mut unread = filtered("");
        unread.broken = Some(broken.to_owned());
        assert_eq!(
            run_steps(&rust_mutation_of_changes(), &mut unread, &mut Vec::new()),
            failed(format!(
                "Rust mutation testing of the changes: cannot {doing}: cannot run `{broken}`: \
                 not found"
            )),
            "{doing}"
        );
    }
}

#[test]
fn refuses_to_mutate_a_tree_where_git_hides_files() {
    for (listed, file, flag) in [
        (
            "H Cargo.toml\0h crates/audio-core/src/plan.rs\0",
            "crates/audio-core/src/plan.rs",
            "--no-assume-unchanged",
        ),
        (
            "S crates/a/src/b.rs\0",
            "crates/a/src/b.rs",
            "--no-skip-worktree",
        ),
    ] {
        let mut fake = passing();
        drop(
            fake.files
                .insert(format!("capture {}", HIDDEN.0), listed.to_owned()),
        );
        assert_eq!(
            run_steps(&rust_mutation_of_changes(), &mut fake, &mut Vec::new()),
            failed(format!(
                "Rust mutation testing of the changes: `{file}` is hidden from git status \
                 (assume-unchanged or skip-worktree), so no diff of git shows its changes: clear \
                 it with `git update-index {flag} -- '{file}'`"
            )),
            "{listed}"
        );
        assert!(
            !fake
                .ran
                .iter()
                .any(|line| line.starts_with("cargo mutants")),
            "{listed}: nothing mutated"
        );
    }
    let mut unlisted = passing();
    unlisted.broken = Some(HIDDEN.0.to_owned());
    assert_eq!(
        run_steps(&rust_mutation_of_changes(), &mut unlisted, &mut Vec::new()),
        failed(format!(
            "Rust mutation testing of the changes: cannot read the work tree: cannot run `{}`: \
             not found",
            HIDDEN.0
        )),
        "the hidden files cannot be listed"
    );
}

#[test]
fn refuses_a_tracked_file_git_passes_through_a_filter() {
    let tracked = "a.rs\0b c.rs\0";
    let answer =
        |first: &str, second: &str| format!("a.rs\0filter\0{first}\0b c.rs\0filter\0{second}\0");
    for (first, second) in [("unspecified", "unspecified"), ("unset", "unspecified")] {
        assert_eq!(
            crate::filtered(tracked, &answer(first, second)),
            None,
            "{first}, {second}: no filter"
        );
    }
    assert_eq!(crate::filtered("", ""), None, "no tracked file");
    for (value, path) in [("hide", "b c.rs"), ("set", "b c.rs"), ("lfs", "b c.rs")] {
        assert_eq!(
            crate::filtered(tracked, &answer("unspecified", value)),
            Some(format!(
                "`{path}` passes through git's `{value}` filter, whose clean driver may hide a \
                 change from git (status, diff): the tree checked may not be the one committed: \
                 remove its `filter` attribute (from `.gitattributes`, `info/attributes` in the \
                 directory `git rev-parse --git-common-dir` names or the file \
                 `core.attributesFile` names)"
            )),
            "{value}"
        );
    }
    for unread in [
        "a.rs\0filter\0unspecified\0".to_owned(),
        "a.rs\0filter\0unspecified\0b c.rs\0filter\0unspecified".to_owned(),
        "a.rs\0diff\0unspecified\0b c.rs\0filter\0unspecified\0".to_owned(),
        "b c.rs\0filter\0unspecified\0a.rs\0filter\0unspecified\0".to_owned(),
        format!(
            "{}x.rs\0filter\0unspecified\0",
            answer("unspecified", "unspecified")
        ),
        "a.rs: filter: unspecified\nb c.rs: filter: unspecified\n".to_owned(),
    ] {
        assert_eq!(
            crate::filtered(tracked, &unread),
            Some(
                "git's answer on the `filter` attribute of the tracked files is not one entry \
                 per file, in their order"
                    .to_owned()
            ),
            "{unread:?}"
        );
    }
}

#[test]
fn sees_a_change_a_clean_filter_hides_from_git() {
    let dir = scratch("clean-filter");
    let path = "crates/audio-hash/src/lib.rs";
    std::fs::create_dir_all(dir.join("crates/audio-hash/src")).expect("its directory");
    std::fs::write(dir.join(path), "fn a() {}\n").expect("a file");
    let _init = hermetic_git(&dir, &["init", "--quiet", "--initial-branch=main"]);
    let _add = hermetic_git(&dir, &["add", "."]);
    let _commit = hermetic_git(&dir, &["commit", "--quiet", "-m", "first"]);
    let _filter = hermetic_git(&dir, &["config", "filter.hide.clean", "grep -v HIDDEN"]);
    let unclean = || {
        let tree = crate::proof_record::tree_cleanliness(&mut system(dir.path())).expect("read");
        crate::unclean(&tree)
    };
    std::fs::write(dir.join(".git/info/attributes"), "").expect("no attribute");
    assert_eq!(unclean(), None, "a clean tree, no filter assigned");
    std::fs::write(dir.join(".git/info/attributes"), "*.rs filter=hide\n").expect("an attribute");
    std::fs::write(dir.join(path), "fn a() {}\nfn b() {} // HIDDEN\n").expect("a change");
    let _staged = hermetic_git(&dir, &["add", path]);
    assert_eq!(
        hermetic_git(&dir, &["status", "--porcelain"]),
        "",
        "git sees no change: the filter took effect"
    );
    assert_eq!(
        unclean(),
        Some(format!(
            "`{path}` passes through git's `hide` filter, whose clean driver may hide a change \
             from git (status, diff): the tree checked may not be the one committed: remove its \
             `filter` attribute (from `.gitattributes`, `info/attributes` in the directory \
             `git rev-parse --git-common-dir` names or the file `core.attributesFile` names)"
        )),
        "the file the filter hides a change of"
    );
}

#[test]
fn says_how_to_clear_a_file_real_git_hides() {
    for (name, path, quoted) in [
        (
            "hidden-file",
            "crates/audio-hash/src/lib.rs",
            "crates/audio-hash/src/lib.rs",
        ),
        (
            "hidden-quoted",
            "crates/it's \u{e9}.rs",
            "crates/it'\\''s \u{e9}.rs",
        ),
    ] {
        clears_a_file_real_git_hides(name, path, quoted);
    }
}

/// Hides `path` from real git by each mark, then checks that the reason names it and that
/// its commands, the name written `quoted` there, clear it when a POSIX shell runs them.
fn clears_a_file_real_git_hides(name: &str, path: &str, quoted: &str) {
    let dir = scratch(name);
    std::fs::create_dir_all(dir.join("crates/audio-hash/src")).expect("its directory");
    std::fs::write(dir.join(path), "a\n").expect("a file");
    let _init = hermetic_git(&dir, &["init", "--quiet", "--initial-branch=main"]);
    let _add = hermetic_git(&dir, &["add", "."]);
    let _commit = hermetic_git(&dir, &["commit", "--quiet", "-m", "first"]);
    let mut runner = system(dir.path());
    let clear = |flag: &str| format!("`git update-index {flag} -- '{quoted}'`");
    for (marks, commands) in [
        (&["--assume-unchanged"][..], clear("--no-assume-unchanged")),
        (&["--skip-worktree"][..], clear("--no-skip-worktree")),
        (
            &["--assume-unchanged", "--skip-worktree"][..],
            format!(
                "{} then {}",
                clear("--no-assume-unchanged"),
                clear("--no-skip-worktree")
            ),
        ),
    ] {
        for mark in marks {
            let _hide = hermetic_git(&dir, &["update-index", mark, path]);
        }
        let tree = crate::proof_record::tree_cleanliness(&mut runner).expect("the tree");
        assert_eq!(
            crate::unclean(&tree),
            Some(format!(
                "`{path}` is hidden from git status (assume-unchanged or skip-worktree), so no \
                 diff of git shows its changes: clear it with {commands}"
            )),
            "{marks:?}"
        );
        for command in commands.split(" then ") {
            let script = command.trim_matches('`');
            let ran = runner.run_for_stdout(&Cmd::new("sh", &["-c", script], Dir::Root));
            assert!(ran.ended.is_ok(), "{script}: {}", ran.stderr);
        }
        let cleared = crate::proof_record::tree_cleanliness(&mut runner).expect("the tree");
        assert_eq!(
            crate::unclean(&cleared),
            None,
            "{marks:?}: the commands clear it"
        );
    }
}

#[test]
fn compares_the_tools_of_two_records_field_by_field() {
    let tools = fake_tools();
    assert_eq!(tools, fake_tools(), "the same tools");
    let others = [
        crate::proof_record::ProofTools {
            mutants: "cargo-mutants 27.1.1".to_owned(),
            ..fake_tools()
        },
        crate::proof_record::ProofTools {
            git: "git version 2.39.3".to_owned(),
            ..fake_tools()
        },
    ];
    for other in others {
        assert_ne!(tools, other, "one field differs: {other:?}");
    }
}

#[test]
fn reads_exactly_the_proof_records_it_writes() {
    let record = proof_record(PROVEN[2]);
    let text = record.text();
    assert_eq!(
        text,
        format!(
            "poyetik rust-mutants-proven 5\ncommit {}\ntree {}\nbase {}\ncargo-mutants cargo-mutants 27.1.0\ntoolchain 1.99.1\nhost aarch64-apple-darwin\ncompiler 58cf8f9c35b355199727627d07ae34a4e2846d67 LLVM 23.1.1\nlinker Apple clang version 14.0.3 (clang-1403.0.22.14.1)\ngit git version 2.39.2 (Apple Git-143)\n",
            PROVEN[0], PROVEN[1], PROVEN[2]
        ),
        "one field per line, after the tag"
    );
    assert_eq!(
        crate::ProofRecord::parse(&text),
        Ok(record.clone()),
        "round trip"
    );
    let long = crate::ProofRecord {
        commit: "a".repeat(64),
        ..record
    };
    assert_eq!(
        crate::ProofRecord::parse(&long.text()),
        Ok(long),
        "SHA-256 object names"
    );
    let refused = |malformed: &str| crate::ProofRecord::parse(malformed).err();
    for (malformed, reason) in [
        (
            text.trim_end().to_owned(),
            "it does not end with a line break",
        ),
        (
            text.replacen("commit ", "comit ", 1),
            "it has no `commit` line where expected",
        ),
        (
            text.replacen("toolchain 1.99.1", "toolchain ", 1),
            "it has no `toolchain` line where expected",
        ),
        (
            text.replacen("toolchain 1.99.1", "toolchain  ", 1),
            "it has no `toolchain` line where expected",
        ),
        (
            text.replacen("host aarch64-apple-darwin\n", "", 1),
            "it has no `host` line where expected",
        ),
        (
            text.replacen("compiler 58cf", "compile 58cf", 1),
            "it has no `compiler` line where expected",
        ),
        (
            text.replacen(
                "linker Apple clang version 14.0.3 (clang-1403.0.22.14.1)",
                "linker ",
                1,
            ),
            "it has no `linker` line where expected",
        ),
        (
            text.replacen("tree ", "treex", 1),
            "it has no `tree` line where expected",
        ),
        (format!("{text}extra\n"), "it has a line more"),
        (
            text.replacen(PROVEN[0], &"1".repeat(39), 1),
            "its commit is not an object name",
        ),
        (
            text.replacen(PROVEN[1], &"G".repeat(40), 1),
            "its tree is not an object name",
        ),
        (
            text.replacen(PROVEN[2], &"A".repeat(40), 1),
            "its base is not an object name",
        ),
        (
            text.replacen(PROVEN[2], &"3".repeat(41), 1),
            "its base is not an object name",
        ),
        (
            "poyetik rust-mutants-proven 5\ncommit x\n".to_owned(),
            "it has no `tree` line where expected",
        ),
    ] {
        assert_eq!(
            refused(&malformed),
            Some(reason.to_owned()),
            "{malformed:?}"
        );
    }
    for name in ["g", "/", "`", "@"] {
        assert!(
            refused(&text.replacen('1', name, 1)).is_some(),
            "{name} is no hexadecimal digit"
        );
    }
}

#[test]
fn names_the_format_of_a_proof_record_of_another_format() {
    let text = proof_record(PROVEN[2]).text();
    for (malformed, reason) in [
        (
            text.replacen("proven 5", "proven 2", 1),
            "it was written in format 2; this xtask reads format 5",
        ),
        (
            text.replacen("proven 5", "proven 3", 1),
            "it was written in format 3; this xtask reads format 5",
        ),
        (
            text.replacen("proven 5", "proven 4", 1),
            "it was written in format 4; this xtask reads format 5",
        ),
        (
            text.replacen("proven 5", "proven 6", 1),
            "it was written in format 6; this xtask reads format 5",
        ),
        (
            text.replacen("proven 5", "proven 10", 1),
            "it was written in format 10; this xtask reads format 5",
        ),
        (
            text.replacen("proven 5", "proven 05", 1),
            "its first line is not `poyetik rust-mutants-proven 5`",
        ),
        (
            text.replacen("proven 5", "proven +4", 1),
            "its first line is not `poyetik rust-mutants-proven 5`",
        ),
        (
            text.replacen("proven 5", "proven 4x", 1),
            "its first line is not `poyetik rust-mutants-proven 5`",
        ),
        (
            text.replacen("proven 5", "proven ", 1),
            "its first line is not `poyetik rust-mutants-proven 5`",
        ),
        (
            text.replacen("proven 5", "proven -2", 1),
            "its first line is not `poyetik rust-mutants-proven 5`",
        ),
        (
            text.replacen("proven 5", "proven  4", 1),
            "its first line is not `poyetik rust-mutants-proven 5`",
        ),
        (
            text.replacen("proven 5", "proven4", 1),
            "its first line is not `poyetik rust-mutants-proven 5`",
        ),
        (
            text.replacen("poyetik rust-mutants-proven 5", "other 2", 1),
            "its first line is not `poyetik rust-mutants-proven 5`",
        ),
    ] {
        assert_eq!(
            crate::ProofRecord::parse(&malformed).err(),
            Some(reason.to_owned()),
            "{malformed:?}"
        );
    }
}

/// The steps that record the proof: the tree recorded, then the record.
fn proof_steps() -> Vec<Step> {
    vec![
        Step {
            name: "Working tree recorded".to_owned(),
            action: Action::TreeRecorded,
        },
        Step {
            name: "Rust mutation proof recorded for the next default check".to_owned(),
            action: Action::MutationProofRecorded,
        },
    ]
}

/// The commit and tree the fake checks out, full object names.
const CHECKED_OUT: [&str; 2] = [
    "abc1230000000000000000000000000000000000",
    "def4560000000000000000000000000000000000",
];

#[test]
fn records_the_proof_of_a_clean_tree() {
    let mut fake = measured();
    let mut out = Vec::new();
    assert_eq!(
        run_steps(&proof_steps(), &mut fake, &mut out),
        Ok(()),
        "recorded"
    );
    let record = crate::ProofRecord {
        commit: CHECKED_OUT[0].to_owned(),
        tree: CHECKED_OUT[1].to_owned(),
        base: PROVEN[2].to_owned(),
        tools: fake_tools(),
    };
    assert_eq!(
        fake.files.get(&record_path(PROVEN[2])).cloned(),
        Some(format!(
            "poyetik rust-mutants-proven 5\ncommit {}\ntree {}\nbase {}\ncargo-mutants cargo-mutants 27.1.0\ntoolchain 1.99.1\nhost aarch64-apple-darwin\ncompiler 58cf8f9c35b355199727627d07ae34a4e2846d67 LLVM 23.1.1\nlinker Apple clang version 14.0.3 (clang-1403.0.22.14.1)\ngit git version 2.39.2 (Apple Git-143)\n",
            CHECKED_OUT[0], CHECKED_OUT[1], PROVEN[2]
        )),
        "the commit and tree checked out, the base measured, the tools, the host"
    );
    assert_eq!(
        crate::ProofRecord::parse(
            &fake
                .files
                .get(&record_path(PROVEN[2]))
                .cloned()
                .unwrap_or_default()
        ),
        Ok(record),
        "readable as written"
    );
    assert!(
        String::from_utf8(out).expect("UTF-8").contains(&format!(
            "  recorded: the mutants of {} are proven against {}\n",
            CHECKED_OUT[0], PROVEN[2]
        )),
        "said"
    );
}

#[test]
fn uses_the_proof_record_it_wrote() {
    let mut fake = measured();
    assert_eq!(
        run_steps(&proof_steps(), &mut fake, &mut Vec::new()),
        Ok(()),
        "recorded"
    );
    for (key, value) in [
        (
            format!(
                "capture git rev-parse --verify --quiet {}^{{tree}}",
                CHECKED_OUT[0]
            ),
            format!("{}\n", CHECKED_OUT[1]),
        ),
        (diff_since(CHECKED_OUT[0]), String::new()),
        (diff_since(PROVEN[2]), since_base()),
        (UNTRACKED.to_owned(), String::new()),
        (
            format!("capture {}", MERGE_BASE.0),
            format!("{}\n", PROVEN[2]),
        ),
    ] {
        drop(fake.files.insert(key, value));
    }
    let said = mutate_changes(&mut fake);
    assert!(
        said.contains(&format!(
            "  audio-core: proven by an earlier check at {}, unchanged since\n",
            CHECKED_OUT[0]
        )) && said
            .contains("  no Rust file changed outside the crates proven: no mutant to test\n"),
        "the record written is the record read: {said}"
    );
}

#[test]
fn records_no_proof_of_a_tree_not_clean() {
    for (status, hidden, reason) in [
        (
            " M crates/a/src/b.rs\n",
            "",
            "` M crates/a/src/b.rs` is not committed",
        ),
        (
            "?? crates/a/src/new.rs\n",
            "",
            "`?? crates/a/src/new.rs` is not committed",
        ),
        (
            "?? conformance/x.json\n",
            "",
            "`?? conformance/x.json` is not committed",
        ),
        ("?? \"a b.rs\"\n", "", "`?? \"a b.rs\"` is not committed"),
        (
            "?? .cargo/mutants.toml\n",
            "",
            "`?? .cargo/mutants.toml` is not committed",
        ),
        (
            "",
            "H Cargo.toml\0h crates/audio-core/src/plan.rs\0",
            "`crates/audio-core/src/plan.rs` is hidden from git status (assume-unchanged or \
             skip-worktree), so no diff of git shows its changes: clear it with `git update-index \
             --no-assume-unchanged -- 'crates/audio-core/src/plan.rs'`",
        ),
        (
            "",
            "S crates/audio-core/src/plan.rs\0",
            "`crates/audio-core/src/plan.rs` is hidden from git status (assume-unchanged or \
             skip-worktree), so no diff of git shows its changes: clear it with `git update-index \
             --no-skip-worktree -- 'crates/audio-core/src/plan.rs'`",
        ),
    ] {
        let mut dirty = measured();
        for (command, answer) in [(PUSH_STATUS.0, status), (HIDDEN.0, hidden)] {
            drop(
                dirty
                    .files
                    .insert(format!("capture {command}"), answer.to_owned()),
            );
        }
        let mut out = Vec::new();
        assert_eq!(
            run_steps(&proof_steps(), &mut dirty, &mut out),
            Ok(()),
            "{status}{hidden}"
        );
        assert!(
            !dirty.ran.iter().any(|line| line.starts_with("write")),
            "{status}{hidden}: nothing recorded"
        );
        assert_eq!(
            String::from_utf8(out).expect("UTF-8"),
            format!(
                "▶ [1/2] Working tree recorded\n  done in 1.5 s\n▶ [2/2] Rust mutation proof recorded for the next default check\n  not recorded: the tree is not clean: {reason}\n  done in 1.5 s\n✔ all 2 checks passed in 3.0 s\n"
            ),
            "{status}{hidden}: said"
        );
    }
}

#[test]
fn records_no_proof_of_a_tree_that_changed() {
    let changed_while = "  not recorded: the tree changed while the checks ran, or was not \
                         recorded at their start\n";
    let mut unrecorded = measured();
    let mut out = Vec::new();
    assert_eq!(
        run_steps(&proof_steps().split_off(1), &mut unrecorded, &mut out),
        Ok(()),
        "no state of the tree"
    );
    assert!(
        !unrecorded.ran.iter().any(|line| line.starts_with("write"))
            && String::from_utf8(out)
                .expect("UTF-8")
                .contains(changed_while),
        "nothing recorded without the tree's state at the start"
    );
    let mut moved = measured();
    moved.rewrite = Some((
        "git checkout other".to_owned(),
        format!("capture {}", TREE_HEAD.0),
        "fed9870000000000000000000000000000000000\nrefs/heads/other\n".to_owned(),
    ));
    let mut steps = proof_steps();
    steps.insert(
        1,
        Step {
            name: "move".to_owned(),
            action: Action::Run(Cmd::new("git", &["checkout", "other"], Dir::Root)),
        },
    );
    let mut moved_out = Vec::new();
    assert_eq!(
        run_steps(&steps, &mut moved, &mut moved_out),
        Ok(()),
        "moved"
    );
    assert!(
        !moved.ran.iter().any(|line| line.starts_with("write"))
            && String::from_utf8(moved_out)
                .expect("UTF-8")
                .contains(changed_while),
        "nothing recorded when what is checked out changed"
    );
}

/// Runs the proof steps on `fake`, which must succeed without writing a record, and returns
/// what the record step said.
fn not_recorded(fake: &mut Fake) -> String {
    let mut out = Vec::new();
    assert_eq!(
        run_steps(&proof_steps(), fake, &mut out),
        Ok(()),
        "the record is optional"
    );
    assert!(
        !fake.files.contains_key(&record_path(PROVEN[2])),
        "nothing recorded"
    );
    let said = String::from_utf8(out).expect("UTF-8");
    said.lines()
        .find(|line| line.starts_with("  not recorded: "))
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn records_nothing_when_rustc_names_a_key_twice() {
    for (twice, key) in [
        (
            "commit-hash: 1\nhost: a b\nhost: c\nLLVM version: 2\n",
            "host",
        ),
        (
            "commit-hash: 1\ncommit-hash: 1\nhost: a\nLLVM version: 2\n",
            "commit-hash",
        ),
        (
            "commit-hash: 1\nhost: a\nLLVM version: 2\nLLVM version: 3\n",
            "LLVM version",
        ),
    ] {
        let mut repeated = measured();
        drop(
            repeated
                .files
                .insert(format!("capture {}", HOST.0), twice.to_owned()),
        );
        assert_eq!(
            not_recorded(&mut repeated),
            format!("  not recorded: `rustc -vV` names {key} twice"),
            "a line rustc never writes twice: {twice}"
        );
    }
}

#[test]
fn records_nothing_when_it_cannot_read_or_write() {
    for (broken, reason) in [
        (
            "git log -1 --format=%T HEAD".to_owned(),
            "cannot read the tree checked out: cannot run `git log -1 --format=%T HEAD`: not found"
                .to_owned(),
        ),
        (
            "cargo mutants --version".to_owned(),
            "cannot read the version of cargo-mutants: cannot run `cargo mutants --version`: not found"
                .to_owned(),
        ),
        (
            "rustc -vV".to_owned(),
            "cannot read the host of rustc: cannot run `rustc -vV`: not found".to_owned(),
        ),
        (
            "git ls-files -v -z".to_owned(),
            "cannot read the work tree: cannot run `git ls-files -v -z`: not found".to_owned(),
        ),
        (
            PUSH_STATUS.0.to_owned(),
            format!("cannot read the work tree: cannot run `{}`: not found", PUSH_STATUS.0),
        ),
        (
            format!("write {}", record_path(PROVEN[2])),
            format!("cannot write {}: read-only", record_path(PROVEN[2])),
        ),
    ] {
        let mut fake = measured();
        fake.broken = Some(broken.clone());
        assert_eq!(
            not_recorded(&mut fake),
            format!("  not recorded: {reason}"),
            "{broken}"
        );
    }
    let mut no_base = passing();
    assert_eq!(
        not_recorded(&mut no_base),
        format!(
            "  not recorded: cannot read {}: missing",
            crate::MEASURED_BASE
        ),
        "no base measured"
    );
    let mut no_host = measured();
    drop(
        no_host
            .files
            .insert(format!("capture {}", HOST.0), "release: 1\n".to_owned()),
    );
    assert_eq!(
        not_recorded(&mut no_host),
        "  not recorded: `rustc -vV` names no host",
        "no host"
    );
    let mut no_channel = measured();
    drop(no_channel.files.insert(
        crate::RUST_TOOLCHAIN_FILE.to_owned(),
        "[toolchain]\n".to_owned(),
    ));
    assert_eq!(
        not_recorded(&mut no_channel),
        "  not recorded: rust-toolchain.toml pins no channel",
        "no channel"
    );
    let mut no_common = measured();
    drop(no_common.files.remove(&format!("capture {}", COMMON_DIR.0)));
    assert_eq!(
        not_recorded(&mut no_common),
        format!(
            "  not recorded: cannot find git's common directory: `{}` exited with code 1: failed",
            COMMON_DIR.0
        ),
        "no common directory"
    );
}

#[test]
fn fails_to_record_what_it_would_not_read_back() {
    let step = "Rust mutation proof recorded for the next default check";
    let mut empty_base = measured();
    drop(
        empty_base
            .files
            .insert(crate::MEASURED_BASE.to_owned(), "\n".to_owned()),
    );
    assert_eq!(
        run_steps(&proof_steps(), &mut empty_base, &mut Vec::new()),
        failed(format!(
            "{step}: cannot record the mutation proof: it has no `base` line where expected"
        )),
        "an empty base is never written"
    );
    assert!(
        !empty_base
            .ran
            .iter()
            .any(|line| line.starts_with("write .git/poyetik/")),
        "nothing written"
    );
    let mut spaced = measured();
    drop(spaced.files.insert(
        format!("capture {}", HOST.0),
        "commit-hash: 1\nhost: a b\nLLVM version: 2\n".to_owned(),
    ));
    assert_eq!(
        run_steps(&proof_steps(), &mut spaced, &mut Vec::new()),
        Ok(()),
        "a host with a space is written as it is, and reads back"
    );
    assert!(
        spaced
            .files
            .get(&record_path(PROVEN[2]))
            .is_some_and(|text| text.contains("\nhost a b\n")),
        "recorded"
    );
}

#[test]
fn shares_proof_records_between_the_worktrees_of_real_git() {
    let dir = scratch("proof-record");
    let main = dir.join("main");
    std::fs::create_dir_all(&main).expect("main worktree");
    let _init = hermetic_git(&main, &["init", "--quiet", "--initial-branch=main"]);
    std::fs::write(main.join("a.rs"), "fn a() {}\n").expect("a source");
    let _add = hermetic_git(&main, &["add", "a.rs"]);
    let _commit = hermetic_git(&main, &["commit", "--quiet", "-m", "first"]);
    let commit = hermetic_git(&main, &["rev-parse", "HEAD"])
        .trim()
        .to_owned();
    let tree = hermetic_git(&main, &["log", "-1", "--format=%T", "HEAD"])
        .trim()
        .to_owned();
    let linked = dir.join("linked");
    let _worktree = hermetic_git(
        &main,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "other",
            linked.to_str().expect("UTF-8"),
        ],
    );
    let record = crate::ProofRecord {
        commit: commit.clone(),
        tree,
        base: commit.clone(),
        tools: fake_tools(),
    };
    // Written from the linked worktree, read from the main one: one common directory.
    let mut from_linked = system(linked);
    let path = crate::proof_record::proof_record_path(&mut from_linked, &commit)
        .expect("common directory");
    from_linked
        .write(&path, &record.text())
        .expect("the record is written");
    let mut from_main = system(main.clone());
    let main_path =
        crate::proof_record::proof_record_path(&mut from_main, &commit).expect("common directory");
    let read = from_main.read(&main_path).expect("the record is read");
    assert_eq!(
        crate::ProofRecord::parse(&read),
        Ok(record),
        "the same record"
    );
    // Real git names the proven commit's tree, and nothing for a commit it does not have.
    let peel = |runner: &mut SystemRunner, object: &str| {
        runner
            .capture(&Cmd::new(
                "git",
                &[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    &format!("{object}^{{tree}}"),
                ],
                Dir::Root,
            ))
            .map(|named| named.trim().to_owned())
            .ok()
    };
    assert_eq!(
        peel(&mut from_main, &commit),
        Some(
            hermetic_git(&main, &["log", "-1", "--format=%T", "HEAD"])
                .trim()
                .to_owned()
        ),
        "the tree of a commit git has"
    );
    assert_eq!(
        peel(&mut from_main, &"1".repeat(40)),
        None,
        "nothing for a commit git does not have"
    );
}

/// A passing fake that measured [`PROVEN`]'s base.
fn measured() -> Fake {
    let mut fake = passing();
    drop(
        fake.files
            .insert(crate::MEASURED_BASE.to_owned(), format!("{}\n", PROVEN[2])),
    );
    fake
}
