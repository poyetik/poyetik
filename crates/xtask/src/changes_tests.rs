//! Tests of what changed since the mutation base, and how far a change widens the mutants to test.

use alloc::collections::BTreeSet;
use core::fmt::Write as _;
use std::path::Path;

use crate::tests::{
    CHANGES, COMMON_DIR, FAKE_JOBS, Fake, HERMETIC, HOST, LINKER, LISTED, MAIN_BASE, MERGE_BASE,
    NEW_RUST_FILES, PROVEN, UNTRACKED, diff_since, failed, fake_revision, fixture_metadata,
    hermetic_git, mutate_changes, one_line_change, passing, proof_record, record_path,
    revision_answers, rust_mutation_of_changes, scratch, since_base, system,
};
use crate::{
    CHANGES_DIFF, MEASURED_BASE, PROOF_RECORDS, RustCrate, Step, SystemRunner, Target,
    UNVIABLE_REPORT, changes_mutants_command, mutants_steps, run_steps,
};

#[test]
fn mutates_the_changes_since_the_merge_base_with_the_upstream() {
    let mut fake = passing();
    let mut out = Vec::new();
    assert_eq!(
        run_steps(&rust_mutation_of_changes(), &mut fake, &mut out),
        Ok(()),
        "mutated"
    );
    assert_eq!(
        fake.ran,
        vec![
            "capture git ls-files -v -z",
            "capture git ls-files -z",
            "capture git ls-files -s -z",
            "capture git merge-base HEAD @{upstream}",
            "write target/xtask/mutants-base",
            "capture git rev-parse --git-common-dir",
            "read .git/poyetik/rust-mutants-proven/abc1230000000000000000000000000000000000",
            "capture git ls-files -z --others --exclude-per-directory=.gitignore -- *.rs",
            "capture git diff --no-color --name-only -z --no-renames abc1230000000000000000000000000000000000",
            "capture git -c diff.suppressBlankEmpty=false diff --no-color --no-ext-diff --no-textconv --text --no-renames --unified=3 --inter-hunk-context=0 --diff-algorithm=myers --indent-heuristic --src-prefix=a/ --dst-prefix=b/ abc1230000000000000000000000000000000000 -- *.rs :(exclude)crates/visual-gpu/",
            // The mutants of the file changed in part, listed with their functions' spans: the
            // changes widened to the functions holding them.
            "capture cargo mutants --no-config --output=. --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --cargo-arg=--config=target.'cfg(target_os = \"linux\")'.runner=[\"prlimit\", \"--as=4294967296\"] --test-tool=nextest --cargo-test-arg=--max-fail=1:immediate --cargo-test-arg=--no-tests=warn --workspace --list --json --file crates/a/src/b.rs -- -- --include-ignored",
            "write target/xtask/changes.diff",
            "cargo mutants --no-config --output=. --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --cargo-arg=--config=target.'cfg(target_os = \"linux\")'.runner=[\"prlimit\", \"--as=4294967296\"] --test-tool=nextest --cargo-test-arg=--max-fail=1:immediate --cargo-test-arg=--no-tests=warn --workspace --in-diff target/xtask/changes.diff --jobs 3 --timeout-multiplier 15 --minimum-test-timeout 60 -- -- --include-ignored",
            "read mutants.out/log/baseline.log",
            "read mutants.out/unviable.txt",
            "read mutants-unviable.txt",
        ],
        "the diff since the merge base is written, then mutated; a change to no crate's test \
         code or test data widens it to no crate, xtask included"
    );
    assert_eq!(
        CHANGES_DIFF, "target/xtask/changes.diff",
        "below the ignored target/"
    );
    assert_eq!(
        fake.files.get(CHANGES_DIFF).map(String::as_str),
        Some(CHANGES.1),
        "the diff as git wrote it"
    );
    assert_eq!(
        String::from_utf8(out).expect("UTF-8"),
        "▶ [1/2] Rust mutation testing of the changes\n  the changes since abc1230000000000000000000000000000000000 (merge base with @{upstream})\n  test order: no suggestion, cannot read mutants.out/log/baseline.log: missing\n  done in 1.5 s\n▶ [2/2] Rust unviable mutants reviewed\n  done in 1.5 s\n✔ all 2 checks passed in 3.0 s\n",
        "the base is said"
    );
}

#[test]
fn falls_back_to_origin_main_then_main() {
    for (resolved, against) in [
        (vec!["origin/main", "main"], "origin/main"),
        (vec!["main"], "main"),
    ] {
        let mut fake = passing();
        let _upstream = fake.files.remove(&format!("capture {}", MERGE_BASE.0));
        let _main = fake.files.remove(&format!("capture {}", MAIN_BASE.0));
        for base in &resolved {
            let _base = fake.files.insert(
                format!("capture git merge-base HEAD {base}"),
                "abc1230000000000000000000000000000000000\n".to_owned(),
            );
        }
        let mut out = Vec::new();
        assert_eq!(
            run_steps(&rust_mutation_of_changes(), &mut fake, &mut out),
            Ok(()),
            "{against}"
        );
        let tried: Vec<&str> = fake
            .ran
            .iter()
            .map(String::as_str)
            .filter(|line| line.starts_with("capture git merge-base"))
            .collect();
        let expected: Vec<String> = ["@{upstream}", "origin/main", "main"]
            .iter()
            .take_while(|base| **base != against)
            .chain(core::iter::once(&against))
            .map(|base| format!("capture git merge-base HEAD {base}"))
            .collect();
        assert_eq!(tried, expected, "tried in order until {against}");
        assert!(
            String::from_utf8(out)
                .expect("UTF-8")
                .contains(&format!("(merge base with {against})")),
            "{against} said"
        );
    }
    let mut none = passing();
    let _upstream = none.files.remove(&format!("capture {}", MERGE_BASE.0));
    let _main = none.files.remove(&format!("capture {}", MAIN_BASE.0));
    assert_eq!(
        run_steps(&rust_mutation_of_changes(), &mut none, &mut Vec::new()),
        failed(
            "Rust mutation testing of the changes: no base to measure the changes against: \
             HEAD shares no history with any of @{upstream}, origin/main, main (see: cargo xtask check --full)\n\
             `git merge-base HEAD @{upstream}` exited with code 1: failed\n\
             `git merge-base HEAD origin/main` exited with code 1: failed\n\
             `git merge-base HEAD main` exited with code 1: failed"
        ),
        "no base: the check fails, saying why each could not be used"
    );
    assert!(
        !none
            .ran
            .iter()
            .any(|line| line.starts_with("cargo mutants")),
        "nothing mutated"
    );
}

#[test]
fn mutates_a_whole_crate_whose_integration_tests_changed() {
    let test_code = "crates/audio-core/tests/laws.rs";
    let mut fake = passing();
    let _changes = fake
        .files
        .insert(format!("capture {}", CHANGES.0), one_line_change(test_code));
    for (path, text) in [
        ("crates/audio-core/src/lib.rs", "fn a() {}\n"),
        ("crates/audio-core/tests/laws.rs", "fn t() {}\n"),
        ("crates/audio-core/tests/laws.rs", "fn l() {}\n"),
        ("crates/audio-boxes/src/lib.rs", "fn o() {}\n"),
    ] {
        let _source = fake.files.insert(path.to_owned(), text.to_owned());
    }
    let mut out = Vec::new();
    assert_eq!(
        run_steps(&rust_mutation_of_changes(), &mut fake, &mut out),
        Ok(())
    );
    assert_eq!(
        fake.files.get(CHANGES_DIFF).cloned(),
        Some(crate::new_file_diff(
            "crates/audio-core/src/lib.rs",
            "fn a() {}\n"
        )),
        "every source of its crate added whole, nothing else of it, its test code and other \
         crates apart"
    );
    assert!(
        fake.ran.contains(&format!(
            "{} :(exclude)crates/audio-core/ :(exclude)crates/visual-gpu/",
            diff_since(MERGE_BASE.1.trim())
        )),
        "git leaves the crate out of the diff"
    );
    assert!(
        String::from_utf8(out).expect("UTF-8").contains(
            "  the integration tests of audio-core changed: every mutant of audio-core\n"
        ),
        "said"
    );
}

/// The declaration of the unit test file `file` beside its source.
fn declares(file: &str) -> String {
    format!("fn a() {{}}\n#[cfg(test)]\n#[path = \"{file}\"]\nmod tests;\n")
}

/// The diff git writes for a change of one line of `path`, as the fake's changes hold it.
fn tests_section(path: &str) -> String {
    format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -1 +1 @@\n-a\n+b\n")
}

/// Runs the default check's mutation of the changes on a fake whose changes are `changes`
/// (one line each) and whose files are `files`; returns the diff given to cargo-mutants,
/// what was said and what ran.
fn mutate_with(changes: &[&str], files: &[(&str, &str)]) -> (Option<String>, String, Vec<String>) {
    let mut fake = passing();
    let _changes = fake.files.insert(
        format!("capture {}", CHANGES.0),
        changes.iter().map(|path| one_line_change(path)).collect(),
    );
    for (path, text) in files {
        let _source = fake.files.insert((*path).to_owned(), (*text).to_owned());
    }
    let mut out = Vec::new();
    assert_eq!(
        run_steps(&rust_mutation_of_changes(), &mut fake, &mut out),
        Ok(()),
        "{changes:?}"
    );
    (
        fake.files.get(CHANGES_DIFF).cloned(),
        String::from_utf8(out).expect("UTF-8"),
        fake.ran,
    )
}

#[test]
fn mutates_the_source_declaring_the_unit_tests_changed_and_no_other() {
    let plan = declares("plan_tests.rs");
    let math = declares("reduction_tests.rs");
    let others = [
        ("crates/audio-core/src/lib.rs", "fn a() {}\n"),
        ("crates/audio-core/src/plan_tests.rs", "fn t() {}\n"),
        ("crates/audio-core/src/reduction_tests.rs", "fn r() {}\n"),
        ("crates/audio-core/tests/laws.rs", "fn l() {}\n"),
        ("crates/audio-boxes/src/lib.rs", "fn o() {}\n"),
    ];
    // A test file and the source of the same name declaring it; then a test file declared
    // by a source of another name (math.rs declares reduction_tests.rs).
    for (test, source, text) in [
        (
            "crates/audio-core/src/plan_tests.rs",
            "crates/audio-core/src/plan.rs",
            &plan,
        ),
        (
            "crates/audio-core/src/reduction_tests.rs",
            "crates/audio-core/src/math.rs",
            &math,
        ),
    ] {
        let mut files = others.to_vec();
        files.push((source, text.as_str()));
        let (diff, said, ran) = mutate_with(&[test], &files);
        assert_eq!(
            diff,
            Some(format!(
                "{}{}",
                tests_section(test),
                crate::new_file_diff(source, text)
            )),
            "{test}: the lines of the unit tests (cargo-mutants mutates no test code), then \
             the source declaring them whole; nothing else of its crate, nor of others"
        );
        assert!(
            ran.contains(&format!(
                "{} :(exclude)crates/visual-gpu/ :(exclude,literal){source}",
                diff_since(MERGE_BASE.1.trim())
            )),
            "{test}: git leaves the source out of the diff, literally: {ran:?}"
        );
        assert!(
            said.contains(&format!(
                "  {test} changed: every mutant of {source}, which declares it\n"
            )),
            "{test}: said: {said}"
        );
    }
}

#[test]
fn mutates_the_whole_crate_of_a_unit_test_file_no_source_declares() {
    // A test file deleted with its declaration, or whose declaration was removed; its name
    // said escaped (finding PB-9 of the eighth deep review of visual block 2, #34).
    let (diff, said, _) = mutate_with(
        &["crates/audio-core/src/plan\u{1b}_tests.rs"],
        &[
            ("crates/audio-core/src/lib.rs", "fn a() {}\n"),
            ("crates/audio-core/src/plan.rs", "fn p() {}\n"),
        ],
    );
    assert_eq!(
        diff,
        Some(format!(
            "{}{}",
            crate::new_file_diff("crates/audio-core/src/lib.rs", "fn a() {}\n"),
            crate::new_file_diff("crates/audio-core/src/plan.rs", "fn p() {}\n")
        )),
        "every source of the crate whole, as for its integration tests"
    );
    assert!(
        said.contains(
            "  crates/audio-core/src/plan\\u{1b}_tests.rs changed, which no source of \
             audio-core declares: every mutant of audio-core\n"
        ),
        "said: {said}"
    );
}

#[test]
fn gives_a_source_tested_whole_one_section_however_it_changed() {
    let plan = declares("plan_tests.rs");
    let both = declares("both_tests.rs").replace(
        "mod tests;",
        "mod tests;\n#[cfg(test)]\n#[path = \"also_tests.rs\"]\nmod also;",
    );
    // The source and its unit tests changed together, tracked; two test files of one source.
    let (diff, said, _) = mutate_with(
        &[
            "crates/audio-core/src/plan.rs",
            "crates/audio-core/src/plan_tests.rs",
            "crates/audio-core/src/both_tests.rs",
            "crates/audio-core/src/also_tests.rs",
        ],
        &[
            ("crates/audio-core/src/plan.rs", &plan),
            ("crates/audio-core/src/plan_tests.rs", "fn t() {}\n"),
            ("crates/audio-core/src/both.rs", &both),
            ("crates/audio-core/src/both_tests.rs", "fn t() {}\n"),
            ("crates/audio-core/src/also_tests.rs", "fn t() {}\n"),
        ],
    );
    assert_eq!(
        diff,
        Some(format!(
            "{}{}{}{}{}",
            tests_section("crates/audio-core/src/plan_tests.rs"),
            tests_section("crates/audio-core/src/both_tests.rs"),
            tests_section("crates/audio-core/src/also_tests.rs"),
            crate::new_file_diff("crates/audio-core/src/plan.rs", &plan),
            crate::new_file_diff("crates/audio-core/src/both.rs", &both),
        )),
        "each source whole once, not also as git wrote its change"
    );
    assert_eq!(
        said.matches("every mutant of crates/audio-core/src/both.rs")
            .count(),
        1,
        "said once per source: {said}"
    );
}

#[test]
fn gives_a_new_source_tested_whole_one_section() {
    let plan = declares("plan_tests.rs");
    let mut fake = passing();
    let _changes = fake
        .files
        .insert(format!("capture {}", CHANGES.0), String::new());
    let _listed = fake.files.insert(
        format!("capture {}", NEW_RUST_FILES.0),
        "crates/audio-core/src/plan.rs\0crates/audio-core/src/plan_tests.rs\0".to_owned(),
    );
    for (path, text) in [
        ("crates/audio-core/src/plan.rs", plan.as_str()),
        ("crates/audio-core/src/plan_tests.rs", "fn t() {}\n"),
    ] {
        let _source = fake.files.insert(path.to_owned(), text.to_owned());
    }
    assert_eq!(
        run_steps(&rust_mutation_of_changes(), &mut fake, &mut Vec::new()),
        Ok(())
    );
    assert_eq!(
        fake.files.get(CHANGES_DIFF).cloned(),
        Some(format!(
            "{}{}",
            crate::new_file_diff("crates/audio-core/src/plan_tests.rs", "fn t() {}\n"),
            crate::new_file_diff("crates/audio-core/src/plan.rs", &plan)
        )),
        "the new source once, whole"
    );
}

#[test]
fn refuses_a_source_it_cannot_lex_when_it_looks_for_the_declarer() {
    let mut fake = passing();
    let _changes = fake.files.insert(
        format!("capture {}", CHANGES.0),
        one_line_change("crates/audio-core/src/plan_tests.rs"),
    );
    let _source = fake.files.insert(
        "crates/audio-core/src/plan.rs".to_owned(),
        "#!shebang\n".to_owned(),
    );
    assert_eq!(
        run_steps(&rust_mutation_of_changes(), &mut fake, &mut Vec::new())
            .map_err(|failure| failure.summary),
        Err("Rust mutation testing of the changes: crates/audio-core/src/plan.rs:1: starts with `#!` not directly followed by `[`, which rustc may strip as a shebang before it lexes: an inner attribute is written `#![`, and no Rust source of the repository holds a shebang".to_owned()),
        "refused, never a silent skip"
    );
}

#[test]
fn mutates_a_whole_crate_whose_test_data_changed() {
    for (data, hash_reads, xtask_reads) in [
        ("spec/render-hash-vectors.json", true, false),
        (".githooks/pre-push", false, true),
        ("apps/roadmap/board.json", false, false),
        ("spec/render-hash-vectors.json.orig", false, false),
        (".githooks-old/pre-push", false, false),
    ] {
        let mut fake = passing();
        let _changes = fake
            .files
            .insert(format!("capture {}", CHANGES.0), one_line_change(data));
        for (path, text) in [
            ("crates/audio-hash/src/lib.rs", "fn h() {}\n"),
            ("crates/audio-hash/src/lib_tests.rs", "fn t() {}\n"),
            ("crates/audio-core/src/lib.rs", "fn a() {}\n"),
            ("crates/xtask/src/lib.rs", "fn x() {}\n"),
        ] {
            let _source = fake.files.insert(path.to_owned(), text.to_owned());
        }
        let mut out = Vec::new();
        assert_eq!(
            run_steps(&rust_mutation_of_changes(), &mut fake, &mut out),
            Ok(()),
            "{data}"
        );
        let hash = crate::new_file_diff("crates/audio-hash/src/lib.rs", "fn h() {}\n");
        let xtask = crate::new_file_diff("crates/xtask/src/lib.rs", "fn x() {}\n");
        let expected = format!(
            "{}{}",
            if hash_reads { hash.as_str() } else { "" },
            if xtask_reads { xtask.as_str() } else { "" }
        );
        assert_eq!(
            fake.files.get(CHANGES_DIFF).cloned(),
            Some(expected).filter(|diff| !diff.is_empty()),
            "{data}: the crates that read it, whole, and no other (xtask's tests read its \
             fixtures and the hooks)"
        );
        let said = String::from_utf8(out).expect("UTF-8");
        assert_eq!(
            said.contains("  the test data of audio-hash changed: every mutant of audio-hash\n"),
            hash_reads,
            "{data}: said"
        );
        assert_eq!(
            said.contains("  the test data of xtask changed: every mutant of xtask\n"),
            xtask_reads,
            "{data}: said of xtask"
        );
    }
}

#[test]
fn mutates_everything_when_what_every_test_uses_changed() {
    // The fake's git cannot list the lock file at the base: every crate may build differently.
    for path in [
        "Cargo.toml",
        "Cargo.lock",
        "rust-toolchain.toml",
        ".cargo/config.toml",
        "tooling/x/Cargo.toml",
        "doc/Cargo.lock",
        "apps/a/rust-toolchain.toml",
        // Its name said escaped (finding R-2 of the mini review of the eighth deep review of
        // visual block 2, #34).
        "apps/a\u{1b}/rust-toolchain.toml",
    ] {
        let mut fake = passing();
        let _changes = fake
            .files
            .insert(format!("capture {}", CHANGES.0), one_line_change(path));
        let mut out = Vec::new();
        assert_eq!(
            run_steps(&rust_mutation_of_changes(), &mut fake, &mut out),
            Ok(()),
            "{path}"
        );
        assert!(
            fake.ran.contains(
                &"cargo mutants --no-config --output=. --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --cargo-arg=--config=target.'cfg(target_os = \"linux\")'.runner=[\"prlimit\", \"--as=4294967296\"] --test-tool=nextest --cargo-test-arg=--max-fail=1:immediate --cargo-test-arg=--no-tests=warn --workspace --jobs 3 --timeout-multiplier 15 --minimum-test-timeout 60 -- -- --include-ignored".to_owned()
            ) && !fake.ran.iter().any(|line| line.starts_with("write target/xtask/changes")),
            "{path}: every mutant of the workspace, no diff: {:?}",
            fake.ran
        );
        let unnarrowed = if ["Cargo.lock", "Cargo.toml"].contains(&path) {
            format!(
                " and cannot be narrowed (cannot read {path} at the merge base \
                 abc1230000000000000000000000000000000000: `git ls-tree --name-only \
                 abc1230000000000000000000000000000000000 -- {path}` exited with code 1: failed)"
            )
        } else {
            String::new()
        };
        assert!(
            String::from_utf8(out).expect("UTF-8").contains(&format!(
                "  {} changed{unnarrowed}: every Rust mutant\n",
                path.replace('\u{1b}', "\\u{1b}")
            )),
            "{path}: said, with why a lock file or the workspace's manifest is not narrowed"
        );
    }
    for path in [
        "Cargo.toml.orig",
        "crates/audio-core/Cargo.toml.orig",
        "crates/audio-testkits/a.rs",
        "doc/Cargo.lock.md",
        "doc/crates/audio-testkit/a.rs",
        "crates/audio-core/src/Cargo.toml.rs",
    ] {
        let mut fake = passing();
        let _changes = fake
            .files
            .insert(format!("capture {}", CHANGES.0), one_line_change(path));
        let mut out = Vec::new();
        assert_eq!(
            run_steps(&rust_mutation_of_changes(), &mut fake, &mut out),
            Ok(()),
            "{path}"
        );
        let rust = Path::new(path)
            .extension()
            .is_some_and(|extension| extension == "rs");
        assert_eq!(
            fake.ran
                .iter()
                .any(|line| line.contains("--in-diff target/xtask/changes.diff ")),
            rust,
            "{path}: only the changes, of Rust files"
        );
        assert_eq!(
            String::from_utf8(out)
                .expect("UTF-8")
                .contains("  no Rust file changed: no mutant to test\n"),
            !rust,
            "{path}: nothing to mutate, said"
        );
    }
    let mut broken = passing();
    let _changes = broken.files.insert(
        format!("capture {}", CHANGES.0),
        one_line_change("Cargo.lock"),
    );
    broken.fail =
        Some("cargo mutants --no-config --output=. --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --cargo-arg=--config=target.'cfg(target_os = \"linux\")'.runner=[\"prlimit\", \"--as=4294967296\"] --test-tool=nextest --cargo-test-arg=--max-fail=1:immediate --cargo-test-arg=--no-tests=warn --workspace --jobs 3 --timeout-multiplier 15 --minimum-test-timeout 60 -- -- --include-ignored".to_owned());
    assert_eq!(
        run_steps(&rust_mutation_of_changes(), &mut broken, &mut Vec::new()),
        failed(
            "Rust mutation testing of the changes: `cargo mutants --no-config --output=. --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --cargo-arg=--config=target.'cfg(target_os = \"linux\")'.runner=[\"prlimit\", \"--as=4294967296\"] --test-tool=nextest --cargo-test-arg=--max-fail=1:immediate --cargo-test-arg=--no-tests=warn --workspace --jobs 3 --timeout-multiplier 15 --minimum-test-timeout 60 -- -- --include-ignored` exited with code 3"
        ),
        "a surviving mutant of the workspace fails"
    );
}

#[test]
fn gives_each_changed_file_one_section_when_its_crate_is_tested_whole() {
    let mut fake = passing();
    let _changes = fake.files.insert(
        format!("capture {}", CHANGES.0),
        format!(
            "{}{}{}",
            one_line_change("crates/audio-core/src/lib.rs"),
            one_line_change("crates/audio-core/tests/laws.rs"),
            one_line_change("crates/audio-hash/src/lib.rs")
        ),
    );
    for (path, text) in [
        ("crates/audio-core/src/lib.rs", "fn a() {}\nfn b() {}\n"),
        ("crates/audio-core/tests/laws.rs", "fn t() {}\n"),
        ("crates/audio-hash/src/lib.rs", "fn h() {}\n"),
    ] {
        let _source = fake.files.insert(path.to_owned(), text.to_owned());
    }
    let _new = fake.files.insert(
        "crates/audio-core/src/new.rs".to_owned(),
        "fn n() {}\n".to_owned(),
    );
    let _listed = fake.files.insert(
        format!("capture {}", NEW_RUST_FILES.0),
        "crates/audio-core/src/new.rs\0".to_owned(),
    );
    let said = mutate_changes(&mut fake);
    assert_eq!(
        fake.files.get(CHANGES_DIFF).cloned(),
        Some(format!(
            "{}{}{}",
            one_line_change("crates/audio-hash/src/lib.rs"),
            crate::new_file_diff("crates/audio-core/src/lib.rs", "fn a() {}\nfn b() {}\n"),
            crate::new_file_diff("crates/audio-core/src/new.rs", "fn n() {}\n")
        )),
        "the source changed and widened, and the new file, once each and whole; the other \
         crate's change as git wrote it"
    );
    assert!(
        said.contains(
            "  the integration tests of audio-core changed: every mutant of audio-core\n"
        ),
        "said: {said}"
    );
    // A git that wrote the excluded crate's change in place of another's, as many sections
    // as changed files outside it: the diff would hold the crate's source twice, and the
    // other's change not at all.
    let mut careless = passing();
    let _careless_changes = careless.files.insert(
        format!("capture {}", CHANGES.0),
        format!(
            "{}{}{}",
            one_line_change("crates/audio-core/src/lib.rs"),
            one_line_change("crates/audio-core/tests/laws.rs"),
            one_line_change("crates/audio-hash/src/lib.rs")
        ),
    );
    let _ignored = careless.files.insert(
        format!(
            "{} :(exclude)crates/audio-core/ :(exclude)crates/visual-gpu/",
            diff_since(MERGE_BASE.1.trim())
        ),
        one_line_change("crates/audio-core/src/lib.rs"),
    );
    let _source = careless.files.insert(
        "crates/audio-core/src/lib.rs".to_owned(),
        "fn a() {}\n".to_owned(),
    );
    assert_eq!(
        run_steps(&rust_mutation_of_changes(), &mut careless, &mut Vec::new()),
        failed(
            "Rust mutation testing of the changes: git's diff holds no section for \
             crates/audio-hash/src/lib.rs, a changed Rust file: it left the change out, whose \
             mutants would go untested"
        ),
        "refused before cargo-mutants reads it"
    );
    assert!(
        !careless
            .ran
            .iter()
            .any(|line| line.starts_with("cargo mutants")
                || line == "write target/xtask/changes.diff"),
        "nothing written, nothing mutated"
    );
}

#[test]
fn refuses_a_diff_git_left_a_change_out_of() {
    // A git that left a change out (a pathspec variable, a configuration no gate reads).
    let mut lossy = passing();
    let _lossy_changes = lossy.files.insert(
        format!("capture {}", CHANGES.0),
        format!(
            "{}{}",
            one_line_change("crates/audio-core/tests/laws.rs"),
            one_line_change("crates/audio-hash/src/lib.rs")
        ),
    );
    let _lost = lossy.files.insert(
        format!(
            "{} :(exclude)crates/audio-core/ :(exclude)crates/visual-gpu/",
            diff_since(MERGE_BASE.1.trim())
        ),
        String::new(),
    );
    assert_eq!(
        run_steps(&rust_mutation_of_changes(), &mut lossy, &mut Vec::new()),
        failed(
            "Rust mutation testing of the changes: git's diff holds no section for \
             crates/audio-hash/src/lib.rs, a changed Rust file: it left the change out, whose \
             mutants would go untested"
        ),
        "refused before cargo-mutants reads it"
    );
    assert!(
        !lossy
            .ran
            .iter()
            .any(|line| line.starts_with("cargo mutants")),
        "nothing mutated"
    );
}

#[test]
fn reads_the_paths_real_git_changes_whatever_they_hold() {
    let dir = scratch("changed-paths");
    let quoted = "crates/audio-core/tests/caf\u{e9}.json";
    let spaced = "crates/audio-core/tests/x b/y.rs";
    let hash = "crates/audio-hash/src/lib.rs";
    for path in [quoted, spaced, hash] {
        let file = dir.join(path);
        std::fs::create_dir_all(file.parent().expect("a parent")).expect("its directory");
        std::fs::write(&file, "a\n").expect("a file");
    }
    let _init = hermetic_git(&dir, &["init", "--quiet", "--initial-branch=main"]);
    let _add = hermetic_git(&dir, &["add", "."]);
    let _commit = hermetic_git(&dir, &["commit", "--quiet", "-m", "first"]);
    for path in [quoted, spaced, hash] {
        std::fs::write(dir.join(path), "b\n").expect("a change");
    }
    std::fs::write(dir.join("crates/audio-core/tests/new.rs"), "fn n() {}\n").expect("a new file");
    let mut runner = system(dir.path());
    assert_eq!(
        crate::changes::changed_since(&mut runner, "HEAD").map_err(|failure| failure.summary),
        Ok(vec![quoted.to_owned(), spaced.to_owned(), hash.to_owned()]),
        "a path git would quote, and one holding ` b/`, as they are"
    );
    assert_eq!(
        crate::changes::new_rust_files(&mut runner).map_err(|failure| failure.summary),
        Ok(vec!["crates/audio-core/tests/new.rs".to_owned()]),
        "the new Rust file"
    );
    let core = crate::RUST_CRATES
        .iter()
        .find(|krate| krate.name == "audio-core")
        .expect("the engine core");
    let diff = crate::changes::diff_since(&mut runner, "HEAD", &[core], &[]).expect("a diff");
    assert_eq!(
        diff,
        format!(
            "diff --git a/{hash} b/{hash}\nindex 7898192..6178079 100644\n--- a/{hash}\n+++ b/{hash}\n@@ -1 +1 @@\n-a\n+b\n"
        ),
        "git leaves out the crate excluded"
    );
}

#[test]
fn leaves_out_exactly_the_files_tested_whole_with_real_git() {
    let dir = scratch("literal-exclusion");
    // A name git's pathspec glob would read as a pattern matching its neighbor.
    let (odd, neighbor) = ("crates/a/src/b[1].rs", "crates/a/src/b1.rs");
    for path in [odd, neighbor] {
        let file = dir.join(path);
        std::fs::create_dir_all(file.parent().expect("a parent")).expect("its directory");
        std::fs::write(&file, "a\n").expect("a file");
    }
    let _init = hermetic_git(&dir, &["init", "--quiet", "--initial-branch=main"]);
    let _add = hermetic_git(&dir, &["add", "."]);
    let _commit = hermetic_git(&dir, &["commit", "--quiet", "-m", "first"]);
    for path in [odd, neighbor] {
        std::fs::write(dir.join(path), "b\n").expect("a change");
    }
    let mut runner = system(dir.path());
    let diff = crate::changes::diff_since(&mut runner, "HEAD", &[], &[odd]).expect("a diff");
    assert!(
        diff.starts_with(&format!("diff --git a/{neighbor} b/{neighbor}\n")) && !diff.contains(odd),
        "the file tested whole left out, its neighbor kept: {diff}"
    );
}

#[test]
fn takes_every_file_git_s_rust_pathspec_matches_for_rust() {
    let dir = scratch("dot-rs");
    let path = "crates/audio-hash/src/.rs";
    std::fs::create_dir_all(dir.join("crates/audio-hash/src")).expect("its directory");
    std::fs::write(dir.join(path), "a\n").expect("a file named `.rs`");
    let _init = hermetic_git(&dir, &["init", "--quiet", "--initial-branch=main"]);
    let _add = hermetic_git(&dir, &["add", "."]);
    let _commit = hermetic_git(&dir, &["commit", "--quiet", "-m", "first"]);
    std::fs::write(dir.join(path), "b\n").expect("a change");
    let mut runner = system(dir.path());
    let changed = crate::changes::changed_since(&mut runner, "HEAD").expect("the changes");
    assert_eq!(
        crate::changes::mutated_diff(
            &mut runner,
            &mut Vec::new(),
            "HEAD",
            (&changed, &[]),
            Vec::new(),
            Vec::new(),
            &[],
        )
        .map_err(|failure| failure.summary),
        Ok(format!(
            "diff --git a/{path} b/{path}\nindex 7898192..6178079 100644\n--- a/{path}\n+++ \
             b/{path}\n@@ -1 +1 @@\n-a\n+b\n"
        )),
        "`.rs`, which git's `*.rs` matches, is a changed Rust file"
    );
}

#[test]
fn shows_cargo_mutants_the_rust_changes_alone() {
    let dir = scratch("binary-change");
    let (sound, hash) = ("conformance/x.wav", "crates/audio-hash/src/lib.rs");
    for path in [sound, hash] {
        std::fs::create_dir_all(dir.join(path).parent().expect("a parent")).expect("its directory");
    }
    std::fs::write(dir.join(sound), [0xff_u8, 0xfe, 0x80, 0x81]).expect("a binary file");
    std::fs::write(dir.join(hash), "a\n").expect("a file");
    let _init = hermetic_git(&dir, &["init", "--quiet", "--initial-branch=main"]);
    let _add = hermetic_git(&dir, &["add", "."]);
    let _commit = hermetic_git(&dir, &["commit", "--quiet", "-m", "first"]);
    std::fs::write(dir.join(sound), [0xff_u8, 0xfe, 0x80, 0x82, 0x00]).expect("a change");
    std::fs::write(dir.join(hash), "b\n").expect("a change");
    let mut runner = system(dir.path());
    assert_eq!(
        crate::changes::diff_since(&mut runner, "HEAD", &[], &[])
            .map_err(|failure| failure.summary),
        Ok(format!(
            "diff --git a/{hash} b/{hash}\nindex 7898192..6178079 100644\n--- a/{hash}\n+++ b/{hash}\n@@ -1 +1 @@\n-a\n+b\n"
        )),
        "the Rust file's section, never the bytes of a file no mutant is made from"
    );
}

#[test]
fn tells_a_file_turned_into_a_link_from_a_change_git_left_out() {
    let dir = scratch("type-change");
    let (markdown, hash, linked) = (
        "docs/a.md",
        "crates/audio-hash/src/lib.rs",
        "crates/audio-hash/src/b.rs",
    );
    for path in [markdown, hash, linked] {
        std::fs::create_dir_all(dir.join(path).parent().expect("a parent")).expect("its directory");
        std::fs::write(dir.join(path), "a\n").expect("a file");
    }
    let _init = hermetic_git(&dir, &["init", "--quiet", "--initial-branch=main"]);
    let _add = hermetic_git(&dir, &["add", "."]);
    let _commit = hermetic_git(&dir, &["commit", "--quiet", "-m", "first"]);
    let link = |path: &str| {
        std::fs::remove_file(dir.join(path)).expect("removed");
        std::os::unix::fs::symlink("elsewhere", dir.join(path)).expect("a link");
    };
    link(markdown);
    std::fs::write(dir.join(hash), "b\n").expect("a change");
    let mut runner = system(dir.path());
    let mutated = |git: &mut SystemRunner| {
        let changed = crate::changes::changed_since(git, "HEAD").expect("the changes");
        crate::changes::mutated_diff(
            git,
            &mut Vec::new(),
            "HEAD",
            (&changed, &[]),
            Vec::new(),
            Vec::new(),
            &[],
        )
        .map_err(|failure| failure.summary)
    };
    assert_eq!(
        mutated(&mut runner),
        Ok(format!(
            "diff --git a/{hash} b/{hash}\nindex 7898192..6178079 100644\n--- a/{hash}\n+++ b/{hash}\n@@ -1 +1 @@\n-a\n+b\n"
        )),
        "a document turned into a link is no Rust change"
    );
    link(linked);
    assert_eq!(
        mutated(&mut runner),
        Err(format!(
            "git's diff shows {linked} as two sections, a file deleted then one created: its type \
             changed (a file turned into a symbolic link, or the reverse), which cargo-mutants \
             would read as one change"
        )),
        "a Rust file turned into a link, said as such"
    );
}

#[test]
fn mutates_an_honest_change_staged_or_not() {
    let dir = scratch("honest-change");
    let (edited, deleted) = (
        "crates/audio-hash/src/lib.rs",
        "crates/audio-hash/src/b c.rs",
    );
    std::fs::create_dir_all(dir.join("crates/audio-hash/src")).expect("its directory");
    for path in [edited, deleted] {
        std::fs::write(dir.join(path), "a\n").expect("a file");
    }
    let _init = hermetic_git(&dir, &["init", "--quiet", "--initial-branch=main"]);
    let _add = hermetic_git(&dir, &["add", "."]);
    let _commit = hermetic_git(&dir, &["commit", "--quiet", "-m", "first"]);
    let mut runner = system(dir.path());
    for (state, expected) in [
        ("edited", " M crates/audio-hash/src/lib.rs\n"),
        ("staged", "M  crates/audio-hash/src/lib.rs\n"),
        ("edited again", "MM crates/audio-hash/src/lib.rs\n"),
        (
            "deleted",
            " D \"crates/audio-hash/src/b c.rs\"\nMM crates/audio-hash/src/lib.rs\n",
        ),
        (
            "every file deleted",
            " D \"crates/audio-hash/src/b c.rs\"\nMD crates/audio-hash/src/lib.rs\n",
        ),
    ] {
        match state {
            "staged" => drop(hermetic_git(&dir, &["add", "."])),
            "deleted" => std::fs::remove_file(dir.join(deleted)).expect("removed"),
            "every file deleted" => std::fs::remove_file(dir.join(edited)).expect("removed"),
            text => std::fs::write(dir.join(edited), text).expect("a change"),
        }
        assert_eq!(
            hermetic_git(&dir, &["status", "--porcelain"]),
            expected,
            "{state}: the state git shows"
        );
        assert_eq!(
            crate::changes::shown_whole(&mut runner),
            Ok(()),
            "{state}: a change git shows whole"
        );
    }
}

/// What the git-fed functions of the mutation step and of the tree gates answer in a
/// repository at `dir` holding two Rust files and a change, after `configure` ran there,
/// their git started with the variables of `env`.
fn git_answers(
    dir: &Path,
    env: &'static [(&'static str, &'static str)],
    configure: &dyn Fn(&Path),
) -> Vec<String> {
    let (first, second) = (
        "crates/audio-hash/src/lib.rs",
        "crates/audio-hash/src/b c.rs",
    );
    std::fs::create_dir_all(dir.join("crates/audio-hash/src")).expect("its directory");
    std::fs::write(dir.join(first), "a\n\nb\nc\n").expect("a file");
    std::fs::write(dir.join(second), "x\n").expect("a file");
    let _init = hermetic_git(dir, &["init", "--quiet", "--initial-branch=main"]);
    let _add = hermetic_git(dir, &["add", "."]);
    let _commit = hermetic_git(dir, &["commit", "--quiet", "-m", "first"]);
    configure(dir);
    std::fs::write(dir.join(first), "a\n\nb\nd\n").expect("a change");
    std::fs::write(dir.join(second), "y\n").expect("a change");
    std::fs::write(dir.join("crates/audio-hash/src/new.rs"), "n\n").expect("a new file");
    let mut runner = SystemRunner {
        root: dir.to_path_buf(),
        env,
    };
    let changed = crate::changes::changed_since(&mut runner, "HEAD").expect("the changes");
    let new = crate::changes::new_rust_files(&mut runner).expect("the new files");
    let tree = crate::proof_record::tree_cleanliness(&mut runner).expect("the tree");
    vec![
        changed.join("|"),
        new.join("|"),
        crate::changes::mutated_diff(
            &mut runner,
            &mut Vec::new(),
            "HEAD",
            (&changed, &[]),
            Vec::new(),
            Vec::new(),
            &[],
        )
        .map_err(|failure| failure.summary)
        .unwrap_or_else(|reason| reason),
        crate::unclean(&tree).unwrap_or_default(),
    ]
}

#[test]
fn gives_the_same_answers_under_a_hostile_git_configuration() {
    let plain = scratch("plain-git");
    let hostile = scratch("hostile-git");
    let expected = git_answers(&plain, &HERMETIC, &|_| {});
    assert_eq!(
        expected.get(..3),
        Some(PLAIN_GIT_ANSWERS.map(str::to_owned).as_slice()),
        "the plain run's answers, as known"
    );
    let answers = git_answers(&hostile, &HERMETIC, &|dir| {
        for (key, value) in [
            ("diff.suppressBlankEmpty", "true"),
            ("diff.noprefix", "true"),
            ("diff.mnemonicPrefix", "true"),
            ("diff.renames", "copies"),
            ("diff.external", "false"),
            ("diff.orderFile", "order.txt"),
            ("diff.context", "0"),
            ("diff.interHunkContext", "9"),
            ("diff.algorithm", "histogram"),
            ("diff.indentHeuristic", "false"),
            ("diff.relative", "true"),
            ("color.ui", "always"),
            ("core.quotePath", "true"),
            ("status.showUntrackedFiles", "no"),
            ("status.relativePaths", "true"),
        ] {
            let _set = hermetic_git(dir, &["config", key, value]);
        }
        std::fs::write(dir.join("order.txt"), "*b*\n").expect("an order file");
        std::fs::write(
            dir.join(".git/info/attributes"),
            "*.rs -diff binary diff=nothing\n",
        )
        .expect("attributes no gate reads");
        std::fs::write(dir.join(".git/info/exclude"), "*.txt\n").expect("excludes");
    });
    assert_eq!(
        answers.get(..2),
        expected.get(..2),
        "the changed paths and the new Rust files"
    );
    assert_eq!(
        answers.get(2),
        expected.get(2),
        "the diff cargo-mutants reads"
    );
    assert_eq!(
        (
            expected.get(3).map(String::as_str),
            answers.get(3).map(String::as_str)
        ),
        (
            Some("` M \"crates/audio-hash/src/b c.rs\"` is not committed"),
            Some("` M \"crates/audio-hash/src/b c.rs\"` is not committed")
        ),
        "the tree is not clean, whatever the configuration"
    );
}

/// The changed paths, the new Rust files and the diff [`git_answers`] gives with no
/// configuration of git's: the reference every other configuration's answers must equal.
const PLAIN_GIT_ANSWERS: [&str; 3] = [
    "crates/audio-hash/src/b c.rs|crates/audio-hash/src/lib.rs",
    "crates/audio-hash/src/new.rs",
    "diff --git a/crates/audio-hash/src/b c.rs b/crates/audio-hash/src/b c.rs\n\
     index 587be6b..975fbec 100644\n\
     --- a/crates/audio-hash/src/b c.rs\t\n\
     +++ b/crates/audio-hash/src/b c.rs\t\n\
     @@ -1 +1 @@\n\
     -x\n\
     +y\n\
     diff --git a/crates/audio-hash/src/lib.rs b/crates/audio-hash/src/lib.rs\n\
     index 068025d..6b37b6d 100644\n\
     --- a/crates/audio-hash/src/lib.rs\n\
     +++ b/crates/audio-hash/src/lib.rs\n\
     @@ -1,4 +1,4 @@\n\
     \x20a\n\
     \x20\n\
     \x20b\n\
     -c\n\
     +d\n",
];

/// The variables of a test's git, and a user's `GIT_DIFF_OPTS` asking for diffs without
/// context, which git applies after the options of its command line.
static TERSE_DIFFS: [(&str, &str); 5] = [
    HERMETIC[0],
    HERMETIC[1],
    HERMETIC[2],
    HERMETIC[3],
    ("GIT_DIFF_OPTS", "--unified=0"),
];

#[test]
fn gives_the_same_diff_under_a_user_s_diff_options() {
    let dir = scratch("terse-git");
    assert_eq!(
        git_answers(&dir, &TERSE_DIFFS, &|_| {}).get(..3),
        Some(PLAIN_GIT_ANSWERS.map(str::to_owned).as_slice()),
        "the changed paths, the new Rust files and the diff with its context"
    );
}

#[test]
fn reads_the_changed_lines_whatever_git_attributes_say() {
    let dir = scratch("binary-attributes");
    let path = "crates/audio-hash/src/lib.rs";
    std::fs::create_dir_all(dir.join("crates/audio-hash/src")).expect("its directory");
    std::fs::write(dir.join(path), "a\n").expect("a file");
    let _init = hermetic_git(&dir, &["init", "--quiet", "--initial-branch=main"]);
    let _add = hermetic_git(&dir, &["add", "."]);
    let _commit = hermetic_git(&dir, &["commit", "--quiet", "-m", "first"]);
    std::fs::write(dir.join(path), "b\n").expect("a change");
    for attribute in ["-diff", "binary"] {
        std::fs::write(
            dir.join(".git/info/attributes"),
            format!("*.rs {attribute}\n"),
        )
        .expect("an attribute no gate sees");
        let mut runner = system(dir.path());
        let diff = crate::changes::diff_since(&mut runner, "HEAD", &[], &[]).expect("a diff");
        assert!(
            diff.ends_with("@@ -1 +1 @@\n-a\n+b\n"),
            "{attribute}: the changed line, never `Binary files … differ`: {diff}"
        );
        assert_eq!(
            crate::changes::incomplete_diff(&diff, &BTreeSet::from([path])),
            None,
            "{attribute}"
        );
    }
}

#[test]
fn refuses_a_diff_that_is_not_every_change_in_text() {
    let (first, second) = ("crates/a/src/b.rs", "crates/a/src/c.rs");
    let one = one_line_change(first);
    let two = format!("{one}{}", one_line_change(second));
    let incomplete = |diff: &str, expected: &[&str]| {
        crate::changes::incomplete_diff(diff, &expected.iter().copied().collect())
    };
    assert_eq!(incomplete(&one, &[first]), None, "one section, one change");
    assert_eq!(
        incomplete(&two, &[second, first]),
        None,
        "two of each, in any order"
    );
    assert_eq!(incomplete("", &[]), None, "none of either");
    assert_eq!(
        incomplete(&one, &[first, second]),
        Some(format!(
            "git's diff holds no section for {second}, a changed Rust file: it left the change out, \
             whose mutants would go untested"
        )),
        "a change left out"
    );
    assert_eq!(
        incomplete(&two, &[first]),
        Some(format!(
            "git's diff holds a section for {second}, which is no changed Rust file outside the \
             crates left out"
        )),
        "a section of a file not changed"
    );
    assert_eq!(
        incomplete(&one, &[second]),
        Some(format!(
            "git's diff holds no section for {second}, a changed Rust file: it left the change out, \
             whose mutants would go untested"
        )),
        "as many sections as changes, but of another file"
    );
    let header = format!("diff --git a/{first} b/{first}");
    for binary in [
        format!("Binary files a/{first} and b/{first} differ\n"),
        "GIT binary patch\nliteral 2\n".to_owned(),
    ] {
        assert_eq!(
            incomplete(&format!("{header}\nindex 1..2 100644\n{binary}"), &[first]),
            Some(format!(
                "git wrote `{header}` as binary: cargo-mutants would see none of its changed \
                 lines"
            )),
            "{binary}"
        );
    }
    let content = format!(
        "{header}\n--- a/{first}\n+++ b/{first}\n@@ -1,3 +1,3 @@\n-Binary files\n \
         GIT binary patch\n\n+deleted file mode 100644\n+diff --git x\n"
    );
    assert_eq!(
        incomplete(&content, &[first]),
        None,
        "lines of a file, an empty one included, are no binary section, nor a section"
    );
    let deleted = format!("{header}\ndeleted file mode 100644\n--- a/{first}\n+++ /dev/null\n");
    let created = format!("{header}\nnew file mode 120000\n--- /dev/null\n+++ b/{first}\n");
    assert_eq!(
        incomplete(&format!("{deleted}{created}"), &[first]),
        Some(format!(
            "git's diff shows {first} as two sections, a file deleted then one created: its type \
             changed (a file turned into a symbolic link, or the reverse), which cargo-mutants \
             would read as one change"
        )),
        "a type change"
    );
    for twice in [
        format!("{one}{one}"),
        format!("{deleted}{}{created}", one_line_change(second)),
        format!("{one}{deleted}{created}"),
        format!("{one}diff --git a/{second} b/{second}\ndeleted file mode 100644\n{one}"),
    ] {
        assert_eq!(
            incomplete(&twice, &[first, second]),
            Some(format!("git's diff shows {first} in two sections")),
            "not a deletion followed by its creation: {twice}"
        );
    }
}

#[test]
fn refuses_a_section_header_whose_path_it_cannot_read() {
    let first = "crates/a/src/b.rs";
    let incomplete = |diff: &str, expected: &[&str]| {
        crate::changes::incomplete_diff(diff, &expected.iter().copied().collect())
    };
    let spaced = "crates/a/src/x b/y.rs";
    assert_eq!(
        incomplete(&one_line_change(spaced), &[spaced]),
        None,
        "a path holding ` b/`, read by the rule without renames"
    );
    for unread in [
        "diff --git \"a/crates/a/src/caf\\303\\251.rs\" \"b/crates/a/src/caf\\303\\251.rs\"",
        "diff --git a/crates/a/src/b.rs b/crates/a/src/c.rs",
        "diff --git a/crates/a/src/b.rs",
        "diff --git a/x",
        "diff --git a/",
        "diff --git a/b.rs c/b.rs",
        "diff --git a/\u{e9}.rs b/\u{e9}.rs\u{e9}",
    ] {
        assert_eq!(
            incomplete(&format!("{unread}\n"), &[first]),
            Some(format!(
                "git wrote `{unread}`, a header whose path cannot be read (quoted, or not \
                 `a/<path> b/<path>`)"
            )),
            "{unread}"
        );
    }
}

#[test]
fn reads_the_commits_objects_whatever_a_replace_ref_says() {
    let dir = scratch("replace-ref");
    let path = "crates/audio-hash/src/lib.rs";
    std::fs::create_dir_all(dir.join("crates/audio-hash/src")).expect("its directory");
    std::fs::write(dir.join(path), "a\n").expect("a file");
    let _init = hermetic_git(&dir, &["init", "--quiet", "--initial-branch=main"]);
    let _add = hermetic_git(&dir, &["add", "."]);
    let _base = hermetic_git(&dir, &["commit", "--quiet", "-m", "base"]);
    let base = hermetic_git(&dir, &["rev-parse", "HEAD"]);
    std::fs::write(dir.join(path), "b\n").expect("a change");
    let _change = hermetic_git(&dir, &["commit", "--quiet", "-am", "change"]);
    let tree = hermetic_git(&dir, &["rev-parse", "HEAD:"]);
    let lookalike = hermetic_git(&dir, &["commit-tree", "-m", "lookalike", tree.trim()]);
    let _replace = hermetic_git(&dir, &["replace", base.trim(), lookalike.trim()]);
    assert_eq!(
        hermetic_git(&dir, &["diff", "--name-only", base.trim()]),
        "",
        "the replace ref is in force for git"
    );
    assert_eq!(
        crate::changes::changed_since(&mut system(dir.path()), base.trim()).expect("the changes"),
        vec![path.to_owned()],
        "xtask's git reads the base's own tree"
    );
}

#[test]
fn mutates_nothing_without_a_change() {
    let mut fake = passing();
    let _changes = fake
        .files
        .insert(format!("capture {}", CHANGES.0), String::new());
    let _report = fake.files.remove(UNVIABLE_REPORT);
    let mut out = Vec::new();
    assert_eq!(
        run_steps(&rust_mutation_of_changes(), &mut fake, &mut out),
        Ok(()),
        "an empty diff passes"
    );
    assert!(
        !fake.ran.iter().any(|line| line.starts_with("cargo mutants")
            || line.starts_with("write target/xtask/changes")),
        "no diff written, nothing mutated: {:?}",
        fake.ran
    );
    assert!(
        String::from_utf8(out).expect("UTF-8").contains(
            "  no change since abc1230000000000000000000000000000000000 (merge base with @{upstream}): no mutant to test\n"
        ),
        "said"
    );
}

#[test]
fn mutates_new_rust_files_whole() {
    let mut fake = passing();
    for (path, text) in [
        ("crates/a/src/new.rs", "fn g() {}\nfn h() {}\n"),
        ("crates/a/src/open.rs", "fn k() {}"),
        ("crates/a/src/empty.rs", ""),
    ] {
        let _new = fake.files.insert(path.to_owned(), text.to_owned());
    }
    let _listed = fake.files.insert(
        format!("capture {}", NEW_RUST_FILES.0),
        "crates/a/src/new.rs\0crates/a/src/open.rs\0crates/a/src/empty.rs\0".to_owned(),
    );
    assert_eq!(
        run_steps(&rust_mutation_of_changes(), &mut fake, &mut Vec::new()),
        Ok(()),
        "mutated"
    );
    assert_eq!(
        fake.files.get(CHANGES_DIFF).map(String::as_str),
        Some(
            format!(
                "{}diff --git a/crates/a/src/new.rs b/crates/a/src/new.rs\nnew file mode 100644\n--- /dev/null\n+++ b/crates/a/src/new.rs\n@@ -0,0 +1,2 @@\n+fn g() {{}}\n+fn h() {{}}\n\
                 diff --git a/crates/a/src/open.rs b/crates/a/src/open.rs\nnew file mode 100644\n--- /dev/null\n+++ b/crates/a/src/open.rs\n@@ -0,0 +1,1 @@\n+fn k() {{}}\n\\ No newline at end of file\n",
                CHANGES.1
            )
            .as_str()
        ),
        "each new file added whole, the empty one apart"
    );
    let mut only_new = passing();
    let _changes = only_new
        .files
        .insert(format!("capture {}", CHANGES.0), String::new());
    let _new = only_new
        .files
        .insert("crates/a/src/new.rs".to_owned(), "fn g() {}\n".to_owned());
    let _only = only_new.files.insert(
        format!("capture {}", NEW_RUST_FILES.0),
        "crates/a/src/new.rs\0".to_owned(),
    );
    assert_eq!(
        run_steps(&rust_mutation_of_changes(), &mut only_new, &mut Vec::new()),
        Ok(()),
        "mutated"
    );
    assert!(
        only_new
            .ran
            .iter()
            .any(|line| line.starts_with("cargo mutants")),
        "a new file alone is a change"
    );
    let mut unreadable = passing();
    let _gone = unreadable.files.insert(
        format!("capture {}", NEW_RUST_FILES.0),
        "crates/a/src/gone.rs\0".to_owned(),
    );
    assert_eq!(
        run_steps(
            &rust_mutation_of_changes(),
            &mut unreadable,
            &mut Vec::new()
        ),
        failed("Rust mutation testing of the changes: cannot read crates/a/src/gone.rs: missing"),
        "a new file that cannot be read"
    );
}

#[test]
fn reports_why_the_changes_cannot_be_mutated() {
    // The changes of the root's workspace: git leaves the GPU workspace out.
    let root_changes = format!("{} :(exclude)crates/visual-gpu/", CHANGES.0);
    for (broken, expected) in [
        (
            root_changes.as_str(),
            format!("cannot read the changes: cannot run `{root_changes}`: not found"),
        ),
        (
            NEW_RUST_FILES.0,
            format!(
                "cannot list the new Rust files: cannot run `{}`: not found",
                NEW_RUST_FILES.0
            ),
        ),
        (
            "write target/xtask/changes.diff",
            "cannot write target/xtask/changes.diff: read-only".to_owned(),
        ),
        (
            "cargo mutants --no-config --output=. --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --cargo-arg=--config=target.'cfg(target_os = \"linux\")'.runner=[\"prlimit\", \"--as=4294967296\"] --test-tool=nextest --cargo-test-arg=--max-fail=1:immediate --cargo-test-arg=--no-tests=warn --workspace --in-diff target/xtask/changes.diff --jobs 3 --timeout-multiplier 15 --minimum-test-timeout 60 -- -- --include-ignored",
            "cannot run `cargo mutants --no-config --output=. --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --cargo-arg=--config=target.'cfg(target_os = \"linux\")'.runner=[\"prlimit\", \"--as=4294967296\"] --test-tool=nextest --cargo-test-arg=--max-fail=1:immediate --cargo-test-arg=--no-tests=warn --workspace --in-diff target/xtask/changes.diff --jobs 3 --timeout-multiplier 15 --minimum-test-timeout 60 -- -- --include-ignored`: not found".to_owned(),
        ),
    ] {
        let mut fake = passing();
        fake.broken = Some(broken.to_owned());
        assert_eq!(
            run_steps(&rust_mutation_of_changes(), &mut fake, &mut Vec::new()),
            failed(format!("Rust mutation testing of the changes: {expected}")),
            "{broken}"
        );
    }
}

#[test]
fn reads_the_versions_spent_at_origin_main_then_main() {
    let mut fallen_back = passing();
    let _origin = fallen_back
        .files
        .remove(&format!("capture {}", MAIN_BASE.0));
    let _main = fallen_back.files.insert(
        "capture git merge-base HEAD main".to_owned(),
        "fed4560000000000000000000000000000000000\n".to_owned(),
    );
    assert_eq!(
        crate::changes::spent_base(&mut fallen_back).map_err(|failure| failure.summary),
        Ok((
            "fed4560000000000000000000000000000000000".to_owned(),
            "main"
        )),
        "main, when origin/main does not resolve"
    );
    assert_eq!(
        fallen_back.ran,
        [
            "capture git merge-base HEAD origin/main",
            "capture git merge-base HEAD main"
        ],
        "origin/main first, never the upstream"
    );
    let mut neither = passing();
    let _unresolved = neither.files.remove(&format!("capture {}", MAIN_BASE.0));
    assert_eq!(
        crate::changes::spent_base(&mut neither).map(|_base| ()),
        failed(
            "no merge base with origin/main or main to read the algorithm versions spent from \
             (fetch or create one of them)\n\
             `git merge-base HEAD origin/main` exited with code 1: failed\n\
             `git merge-base HEAD main` exited with code 1: failed"
        ),
        "neither: why each could not be used"
    );
}

#[test]
fn reads_the_versions_spent_where_real_git_says_main_is() {
    let dir = scratch("spent-base");
    let (remote, clone) = (dir.join("remote.git"), dir.join("clone"));
    std::fs::create_dir_all(&clone).expect("the clone's directory");
    let _bare = hermetic_git(&dir, &["init", "--quiet", "--bare", "remote.git"]);
    let _init = hermetic_git(&clone, &["init", "--quiet", "--initial-branch=main"]);
    let commit = |message: &str| {
        let _commit = hermetic_git(
            &clone,
            &["commit", "--quiet", "--allow-empty", "-m", message],
        );
        hermetic_git(&clone, &["rev-parse", "HEAD"])
            .trim()
            .to_owned()
    };
    let on_main = commit("on main");
    let _remote = hermetic_git(
        &clone,
        &[
            "remote",
            "add",
            "origin",
            remote.to_str().expect("UTF-8 path"),
        ],
    );
    let _pushed = hermetic_git(&clone, &["push", "--quiet", "origin", "main"]);
    let _branch = hermetic_git(&clone, &["switch", "--quiet", "-c", "work"]);
    let pushed = commit("pushed on the branch");
    let _upstream = hermetic_git(&clone, &["push", "--quiet", "-u", "origin", "work"]);
    let _local = commit("not pushed");
    let mut runner = system(clone.clone());
    assert_eq!(
        crate::changes::mutation_base(&mut runner).map_err(|failure| failure.summary),
        Ok((pushed, "@{upstream}")),
        "the changes are measured from the upstream, ahead of main"
    );
    assert_eq!(
        crate::changes::spent_base(&mut runner).map_err(|failure| failure.summary),
        Ok((on_main.clone(), "origin/main")),
        "the versions spent are those on main, never the upstream's"
    );
    let _forgotten = hermetic_git(&clone, &["remote", "remove", "origin"]);
    assert_eq!(
        crate::changes::spent_base(&mut runner).map_err(|failure| failure.summary),
        Ok((on_main, "main")),
        "without origin/main, main"
    );
}

/// The command reading the tree of the proven commit.
fn proven_tree_command() -> String {
    format!(
        "capture git rev-parse --verify --quiet {}^{{tree}}",
        PROVEN[0]
    )
}

/// Cargo metadata where the render hash depends on the engine core when `linked`, and on
/// no crate of the workspace otherwise; the boxes always depend on the engine core.
fn closure_metadata(linked: bool) -> String {
    let hash = if linked {
        r#"[{"name":"audio-core","kind":null,"path":"/r/crates/audio-core"},{"name":"sha2","kind":null}]"#
    } else {
        r#"[{"name":"sha2","kind":null}]"#
    };
    format!(
        r#"{{"workspace_root":"/r","packages":[{{"name":"audio-hash","dependencies":{hash}}},{{"name":"audio-boxes","dependencies":[{{"name":"audio-core","kind":null,"path":"/r/crates/audio-core"}},{{"name":"audio-testkit","kind":"dev","path":"/r/crates/audio-testkit"}}]}},{{"name":"audio-core","dependencies":[]}},{{"name":"serde","dependencies":[]}}]}}"#
    )
}

/// A passing fake whose base is [`PROVEN`]'s, holding `record` as its proof record, the
/// tree of the proven commit, `since_commit` as the changes since it, no untracked file,
/// [`since_base`] as the changes since the base, and [`closure_metadata`] unlinked.
fn proven_fake(record: &crate::ProofRecord, since_commit: &str) -> Fake {
    let mut fake = passing();
    for (key, value) in [
        (record_path(PROVEN[2]), record.text()),
        (proven_tree_command(), format!("{}\n", PROVEN[1])),
        (diff_since(PROVEN[0]), since_commit.to_owned()),
        (diff_since(PROVEN[2]), since_base()),
        (UNTRACKED.to_owned(), String::new()),
        (
            format!("capture {}", MERGE_BASE.0),
            format!("{}\n", PROVEN[2]),
        ),
        (
            format!(
                "capture {}",
                crate::metadata_command(crate::CargoWorkspace::Root).line()
            ),
            closure_metadata(false),
        ),
    ] {
        drop(fake.files.insert(key, value));
    }
    fake
}

/// What the default check says of a crate a record still proves.
fn proven_line(name: &str) -> String {
    format!(
        "  {name}: proven by an earlier check at {}, unchanged since\n",
        PROVEN[0]
    )
}

#[test]
fn skips_only_the_crates_a_valid_proof_record_still_proves() {
    let plan = one_line_change("crates/audio-core/src/plan.rs");
    let mut fake = proven_fake(&proof_record(PROVEN[2]), &plan);
    let said = mutate_changes(&mut fake);
    assert_eq!(
        fake.files.get(CHANGES_DIFF).cloned(),
        Some(plan.clone()),
        "the changes since the base of the crate changed since the record; the render hash, \
         unchanged since and depending on nothing changed, left out"
    );
    assert!(
        said.contains(&format!(
            "  the changes since {} (merge base with @{{upstream}})\n",
            PROVEN[2]
        )) && said.contains(&proven_line("audio-hash"))
            && !said.contains(&proven_line("audio-core")),
        "said: {said}"
    );
    assert_eq!(
        fake.files.get(MEASURED_BASE).cloned(),
        Some(format!("{}\n", PROVEN[2])),
        "the base noted for the record of this run"
    );
    let mut linked = proven_fake(&proof_record(PROVEN[2]), &plan);
    drop(linked.files.insert(
        format!(
            "capture {}",
            crate::metadata_command(crate::CargoWorkspace::Root).line()
        ),
        closure_metadata(true),
    ));
    let linked_said = mutate_changes(&mut linked);
    assert_eq!(
        linked.files.get(CHANGES_DIFF).cloned(),
        Some(since_base()),
        "a crate whose dependency changed since the record is mutated as without it"
    );
    assert!(
        !linked_said.contains("proven by an earlier check"),
        "linked_said: {linked_said}"
    );
    // A line changed since the base, its neighbor since the record: the crate's changes
    // since the base are mutated whole, not only what changed since the record.
    let mut neighbor = proven_fake(&proof_record(PROVEN[2]), &plan);
    let two = "diff --git a/crates/audio-core/src/plan.rs b/crates/audio-core/src/plan.rs\n--- a/crates/audio-core/src/plan.rs\n+++ b/crates/audio-core/src/plan.rs\n@@ -1 +1 @@\n-a\n+b\n@@ -9 +9 @@\n-c\n+d\n";
    drop(neighbor.files.insert(diff_since(PROVEN[2]), two.to_owned()));
    let _said = mutate_changes(&mut neighbor);
    assert_eq!(
        neighbor.files.get(CHANGES_DIFF).map(String::as_str),
        Some(two),
        "every change of the crate since the base"
    );
}

#[test]
fn mutates_nothing_when_every_change_is_proven() {
    for since_commit in [
        String::new(),
        one_line_change("apps/roadmap/board.json"),
        one_line_change("crates/audio-cli/src/lib.rs"),
    ] {
        let mut fake = proven_fake(&proof_record(PROVEN[2]), &since_commit);
        drop(fake.files.remove(UNVIABLE_REPORT));
        let said = mutate_changes(&mut fake);
        assert!(
            said.contains(&proven_line("audio-core"))
                && said.contains(&proven_line("audio-hash"))
                && said.contains(
                    "  no Rust file changed outside the crates proven: no mutant to test\n"
                ),
            "{since_commit}: said: {said}"
        );
        assert!(
            !fake
                .ran
                .iter()
                .any(|line| line.starts_with("cargo mutants")),
            "{since_commit}: nothing mutated"
        );
    }
}

#[test]
fn widens_a_crate_that_changed_since_the_record_as_without_it() {
    // Test code changed since the base, before the record: proven, not widened again.
    let mut before = proven_fake(&proof_record(PROVEN[2]), "");
    drop(before.files.insert(
        diff_since(PROVEN[2]),
        format!(
            "{}{}",
            since_base(),
            one_line_change("crates/audio-hash/tests/vectors.rs")
        ),
    ));
    let said = mutate_changes(&mut before);
    assert!(
        !said.contains("the integration tests of audio-hash changed"),
        "said: {said}"
    );
    // The same, with the render hash changed since the record: widened whole.
    let mut after = proven_fake(
        &proof_record(PROVEN[2]),
        &one_line_change("crates/audio-hash/src/lib.rs"),
    );
    drop(after.files.insert(
        diff_since(PROVEN[2]),
        format!(
            "{}{}",
            since_base(),
            one_line_change("crates/audio-hash/tests/vectors.rs")
        ),
    ));
    drop(after.files.insert(
        "crates/audio-hash/src/lib.rs".to_owned(),
        "fn h() {}\n".to_owned(),
    ));
    let after_said = mutate_changes(&mut after);
    assert!(
        after_said.contains(
            "  the integration tests of audio-hash changed: every mutant of audio-hash\n"
        ) && after_said.contains(&proven_line("audio-core")),
        "after_said: {after_said}"
    );
    assert_eq!(
        after.files.get(CHANGES_DIFF).cloned(),
        Some(crate::new_file_diff(
            "crates/audio-hash/src/lib.rs",
            "fn h() {}\n"
        )),
        "the render hash widened, its source once and whole, the engine core left out"
    );
    // Its test data changed since the record, or an untracked file under it appeared.
    let mut data = proven_fake(
        &proof_record(PROVEN[2]),
        &one_line_change("spec/render-hash-vectors.json"),
    );
    assert!(
        !mutate_changes(&mut data).contains(&proven_line("audio-hash")),
        "test data changed since the record"
    );
    let mut untracked = proven_fake(&proof_record(PROVEN[2]), "");
    drop(untracked.files.insert(
        UNTRACKED.to_owned(),
        "conformance/new.json\0crates/audio-hash/notes.txt\0".to_owned(),
    ));
    let untracked_said = mutate_changes(&mut untracked);
    assert!(
        !untracked_said.contains(&proven_line("audio-hash"))
            && untracked_said.contains(&proven_line("audio-core")),
        "an untracked file in a crate: untracked_said: {untracked_said}"
    );
}

#[test]
fn mutates_every_crate_not_proven_whole_when_what_every_test_uses_changed() {
    // What every test depends on changed since the base, before the record: every crate
    // not proven, whole; the proven ones left out.
    let mut everything = proven_fake(
        &proof_record(PROVEN[2]),
        &one_line_change("crates/audio-hash/src/lib.rs"),
    );
    drop(everything.files.insert(
        diff_since(PROVEN[2]),
        format!("{}{}", since_base(), one_line_change("Cargo.lock")),
    ));
    drop(everything.files.insert(
        "crates/audio-hash/src/lib.rs".to_owned(),
        "fn h() {}\n".to_owned(),
    ));
    drop(everything.files.insert(
        "crates/audio-core/src/plan.rs".to_owned(),
        "fn p() {}\n".to_owned(),
    ));
    let everything_said = mutate_changes(&mut everything);
    let diff = everything
        .files
        .get(CHANGES_DIFF)
        .cloned()
        .unwrap_or_default();
    let proven: String = [
        "audio-core",
        "audio-boxes",
        "audio-patch",
        "audio-host",
        "visual-core",
        "visual-boxes",
        "visual-hash",
        "visual-testkit",
        "visual-cli",
        "visual-host",
        "visual-gpu",
        "visual-wasm",
        "alloc-counter",
        "fpenv",
        "audio-battery",
        "audio-testkit",
        "audio-cli",
        "audio-wasm",
        "coordinator",
        "web-page",
        "xtask",
    ]
    .iter()
    .map(|name| proven_line(name))
    .collect();
    assert_eq!(
        everything_said,
        format!(
            "▶ [1/2] Rust mutation testing of the changes\n  the changes since {} (merge base \
             with @{{upstream}})\n  Cargo.lock changed and cannot be narrowed (cannot read Cargo.lock at \
             the merge base {}: `git ls-tree --name-only {} -- Cargo.lock` exited with code 1: \
             failed): every Rust mutant but the crates proven below\n{proven}  test order: no \
             suggestion, cannot read mutants.out/log/baseline.log: missing\n  done in 1.5 s\n▶ [2/2] Rust unviable mutants reviewed\n  done in \
             1.5 s\n✔ all 2 checks passed in 3.0 s\n",
            PROVEN[2], PROVEN[2], PROVEN[2]
        ),
        "every crate proven named, the render hash mutated whole"
    );
    assert_eq!(
        diff,
        crate::new_file_diff("crates/audio-hash/src/lib.rs", "fn h() {}\n"),
        "every crate is tested whole or proven: git keeps only the Rust files no crate holds"
    );
}

#[test]
fn names_each_proven_crate_the_check_would_otherwise_have_tested() {
    // The shared test crate changed since the base, before the record: the boxes, which
    // only it widens, are proven, and said so; the render hash, out of scope, is not named.
    let mut fake = proven_fake(&proof_record(PROVEN[2]), "");
    drop(fake.files.insert(
        diff_since(PROVEN[2]),
        one_line_change("crates/audio-testkit/src/lib.rs"),
    ));
    drop(fake.files.remove(UNVIABLE_REPORT));
    let said = mutate_changes(&mut fake);
    assert_eq!(
        said,
        format!(
            "▶ [1/2] Rust mutation testing of the changes\n  the changes since {} (merge base \
             with @{{upstream}})\n{}{}  no Rust file changed outside the crates proven: no mutant to test\n  done in \
             1.5 s\n▶ [2/2] Rust unviable mutants reviewed\n  done in 1.5 s\n✔ all 2 checks \
             passed in 3.0 s\n",
            PROVEN[2],
            proven_line("audio-boxes"),
            proven_line("audio-testkit")
        ),
        "the crates in scope only through the widening are named"
    );
}

#[test]
fn reuses_no_record_of_a_crate_whose_quoted_path_changed() {
    let mut fake = proven_fake(&proof_record(PROVEN[2]), "");
    drop(fake.files.insert(
        format!(
            "capture git diff --no-color --name-only -z --no-renames {}",
            PROVEN[0]
        ),
        "crates/audio-hash/tests/caf\u{e9}.json\0".to_owned(),
    ));
    let said = mutate_changes(&mut fake);
    assert!(
        !said.contains(&proven_line("audio-hash")) && said.contains(&proven_line("audio-core")),
        "a path git would quote is a change of its crate: {said}"
    );
}

#[test]
fn ignores_a_proof_record_it_cannot_trust() {
    let other = |edit: &dyn Fn(&mut crate::ProofRecord)| {
        let mut record = proof_record(PROVEN[2]);
        edit(&mut record);
        proven_fake(&record, "")
    };
    ignored(
        &mut other(&|record| record.base = "4".repeat(40)),
        &format!("it is of the base {}", "4".repeat(40)),
    );
    ignored(
        &mut other(&|record| record.tools.mutants = "cargo-mutants 27.0.0".to_owned()),
        "it was made with the cargo-mutants cargo-mutants 27.0.0, not cargo-mutants 27.1.0",
    );
    ignored(
        &mut other(&|record| record.tools.toolchain = "1.98.0".to_owned()),
        "it was made with the toolchain 1.98.0, not 1.99.1",
    );
    ignored(
        &mut other(&|record| record.tools.host = "x86_64-apple-darwin".to_owned()),
        "it was made with the host x86_64-apple-darwin, not aarch64-apple-darwin",
    );
    let mut gone = proven_fake(&proof_record(PROVEN[2]), "");
    drop(gone.files.remove(&proven_tree_command()));
    ignored(
        &mut gone,
        &format!(
            "git cannot find the tree of its commit {0}: `git rev-parse --verify --quiet {0}^{{tree}}` exited with code 1: failed",
            PROVEN[0]
        ),
    );
    let mut retreed = proven_fake(&proof_record(PROVEN[2]), "");
    drop(
        retreed
            .files
            .insert(proven_tree_command(), format!("{}\n", PROVEN[2])),
    );
    ignored(
        &mut retreed,
        &format!("git has no commit {} of tree {}", PROVEN[0], PROVEN[1]),
    );
    let mut malformed = proven_fake(&proof_record(PROVEN[2]), "");
    drop(
        malformed
            .files
            .insert(record_path(PROVEN[2]), "nonsense\n".to_owned()),
    );
    ignored(
        &mut malformed,
        "its first line is not `poyetik rust-mutants-proven 6`",
    );
    let mut unreadable = proven_fake(&proof_record(PROVEN[2]), "");
    unreadable.broken = Some(format!("read {}", record_path(PROVEN[2])));
    ignored(&mut unreadable, "cannot read it: not UTF-8");
    let mut no_tools = proven_fake(&proof_record(PROVEN[2]), "");
    no_tools.broken = Some("cargo mutants --version".to_owned());
    ignored(
        &mut no_tools,
        "cannot read the version of cargo-mutants: cannot run `cargo mutants --version`: not found",
    );
    let mut no_host = proven_fake(&proof_record(PROVEN[2]), "");
    no_host.broken = Some("rustc -vV".to_owned());
    ignored(
        &mut no_host,
        "cannot read the host of rustc: cannot run `rustc -vV`: not found",
    );
    let mut hostless = proven_fake(&proof_record(PROVEN[2]), "");
    drop(hostless.files.insert(
        format!("capture {}", HOST.0),
        "rustc 1.99.1\nhost: \n".to_owned(),
    ));
    ignored(&mut hostless, "`rustc -vV` names no host");
    let mut absent = proven_fake(&proof_record(PROVEN[2]), "");
    drop(absent.files.remove(&record_path(PROVEN[2])));
    assert!(
        !mutate_changes(&mut absent).contains("proof record"),
        "no record: nothing to say"
    );
}

#[test]
fn ignores_a_proof_record_when_its_rules_or_shared_inputs_changed() {
    for (path, what, tracked) in [
        ("crates/xtask/src/lib.rs", "the mutation rules", true),
        ("mutants-unviable.txt", "the mutation rules", true),
        (".cargo/config.toml", "the mutation rules", true),
        (".cargo/mutants.toml", "the mutation rules", false),
        (
            "Cargo.lock",
            "and cannot be narrowed (cannot read Cargo.lock at the merge base \
             1111111111111111111111111111111111111111: `git ls-tree --name-only \
             1111111111111111111111111111111111111111 -- Cargo.lock` exited with code 1: failed), what \
             every test depends on",
            true,
        ),
        ("rust-toolchain.toml", "what every test depends on", true),
        // Finding R-2 of the mini review of the eighth: its name said escaped.
        (
            "a\u{1b}/rust-toolchain.toml",
            "what every test depends on",
            true,
        ),
        (
            "Cargo.toml",
            "and cannot be narrowed (cannot read Cargo.toml at the merge base \
             1111111111111111111111111111111111111111: `git ls-tree --name-only \
             1111111111111111111111111111111111111111 -- Cargo.toml` exited with code 1: failed), what \
             every test depends on",
            true,
        ),
    ] {
        let mut fake = if tracked {
            proven_fake(&proof_record(PROVEN[2]), &one_line_change(path))
        } else {
            let mut untracked = proven_fake(&proof_record(PROVEN[2]), "");
            drop(
                untracked
                    .files
                    .insert(UNTRACKED.to_owned(), format!("{path}\0")),
            );
            untracked
        };
        let said = mutate_changes(&mut fake);
        let comma = if what.starts_with("and ") { " " } else { ", " };
        assert!(
            said.contains(&format!(
                "  ignored the mutation proof record of {}: {} changed since{comma}{what}\n",
                PROVEN[0],
                path.replace('\u{1b}', "\\u{1b}")
            )) && !said.contains("proven by an earlier check"),
            "{path}: said: {said}"
        );
        assert_eq!(
            fake.files.get(CHANGES_DIFF).cloned(),
            Some(since_base()),
            "{path}: as without a record"
        );
    }
    let mut tests = proven_fake(
        &proof_record(PROVEN[2]),
        &one_line_change("crates/xtask/src/lib_tests.rs"),
    );
    let said = mutate_changes(&mut tests);
    assert!(
        !said.contains("ignored the mutation proof record")
            && said.contains(&proven_line("audio-core")),
        "xtask's tests are not its rules: said: {said}"
    );
}

#[test]
fn falls_back_without_a_common_directory() {
    let mut fake = proven_fake(&proof_record(PROVEN[2]), "");
    drop(fake.files.remove(&format!("capture {}", COMMON_DIR.0)));
    let said = mutate_changes(&mut fake);
    assert!(
        said.contains(
            "  no mutation proof record: cannot find git's common directory: `git rev-parse --git-common-dir` exited with code 1: failed\n"
        ),
        "said: {said}"
    );
    assert_eq!(
        fake.files.get(CHANGES_DIFF).cloned(),
        Some(since_base()),
        "the diff since the base"
    );
    let mut unwritable = passing();
    unwritable.broken = Some(format!("write {MEASURED_BASE}"));
    assert_eq!(
        run_steps(
            &rust_mutation_of_changes(),
            &mut unwritable,
            &mut Vec::new()
        ),
        failed(format!(
            "Rust mutation testing of the changes: cannot write {MEASURED_BASE}: read-only"
        )),
        "the base measured cannot be noted"
    );
    for (broken, expected) in [
        (
            UNTRACKED
                .strip_prefix("capture ")
                .unwrap_or_default()
                .to_owned(),
            "cannot list the untracked files: cannot run `git ls-files -z --others --exclude-standard`: not found",
        ),
        (
            crate::metadata_command(crate::CargoWorkspace::Root).line(),
            "cannot list the crates: cannot run `cargo metadata --format-version 1 --no-deps \
             --locked`: not found",
        ),
    ] {
        let mut broken_fake = proven_fake(&proof_record(PROVEN[2]), "");
        broken_fake.broken = Some(broken.clone());
        assert_eq!(
            run_steps(
                &rust_mutation_of_changes(),
                &mut broken_fake,
                &mut Vec::new()
            ),
            failed(format!("Rust mutation testing of the changes: {expected}")),
            "{broken}: the record cannot be weighed"
        );
    }
    let mut invalid = proven_fake(&proof_record(PROVEN[2]), "");
    drop(invalid.files.insert(
        format!(
            "capture {}",
            crate::metadata_command(crate::CargoWorkspace::Root).line()
        ),
        "{}".to_owned(),
    ));
    assert_eq!(
        run_steps(&rust_mutation_of_changes(), &mut invalid, &mut Vec::new()),
        failed("Rust mutation testing of the changes: cargo metadata lists no packages"),
        "metadata without packages"
    );
}

/// The names of `crates`, sorted.
fn sorted_names(crates: &[&RustCrate]) -> Vec<&'static str> {
    let mut names: Vec<&'static str> = crates.iter().map(|krate| krate.name).collect();
    names.sort_unstable();
    names
}

#[test]
fn reads_the_crates_each_crate_compiles_for_its_tests() {
    let closures = crate::changes::crate_closures(&closure_metadata(true)).expect("valid metadata");
    let names = |name: &str| -> Vec<&str> {
        closures
            .get(name)
            .map(|closure| sorted_names(closure))
            .unwrap_or_default()
    };
    assert_eq!(
        names("audio-boxes"),
        ["audio-boxes", "audio-core", "audio-testkit"],
        "its dependencies of every kind, and itself"
    );
    assert_eq!(names("audio-hash"), ["audio-core", "audio-hash"], "linked");
    assert_eq!(
        names("xtask"),
        ["xtask"],
        "a crate the metadata does not list"
    );
    let deep = r#"{"workspace_root":"/r","packages":[{"name":"audio-cli","dependencies":[{"name":"audio-hash","path":"/r/crates/audio-hash"}]},{"name":"audio-hash","dependencies":[{"name":"audio-core","path":"/r/crates/audio-core"},{"name":"other","path":"/r/crates/other"},{"name":"audio-patch","path":"/r/vendor/crates/audio-patch"}]}]}"#;
    let deep_closures = crate::changes::crate_closures(deep).expect("valid metadata");
    assert_eq!(
        deep_closures
            .get("audio-cli")
            .map(|closure| sorted_names(closure)),
        Some(vec!["audio-cli", "audio-core", "audio-hash"]),
        "at any depth, crates outside the workspace apart, one named like a crate elsewhere \
         included"
    );
    assert_eq!(
        crate::changes::crate_closures(r#"{"workspace_root":"/r","packages":[{"name":"audio-hash","dependencies":[{"path":"/r/crates/audio-core"}]}]}"#).map_err(|failure| failure.summary), Err("cargo metadata lists a dependency without a name".to_owned()), "a dependency without a name");
    assert_eq!(
        crate::changes::crate_closures("not json").map_err(|failure| failure.summary),
        Err("invalid cargo metadata: expected ident at line 1 column 2".to_owned()),
        "not JSON"
    );
}

#[test]
fn reads_the_crates_real_cargo_metadata_lists() {
    let (metadata, _) = fixture_metadata();
    let closures = crate::changes::crate_closures(&metadata).expect("valid metadata");
    let of = |name: &str| closures.get(name).map(|closure| sorted_names(closure));
    assert_eq!(
        of("audio-boxes"),
        Some(vec![
            "alloc-counter",
            "audio-battery",
            "audio-boxes",
            "audio-cli",
            "audio-core",
            "audio-testkit",
            "visual-core"
        ]),
        "normal and development dependencies, at any depth"
    );
    assert_eq!(
        of("audio-core"),
        Some(vec![
            "alloc-counter",
            "audio-battery",
            "audio-core",
            "audio-testkit",
            "visual-core"
        ]),
        "a development dependency's own"
    );
    assert_eq!(
        of("fpenv"),
        Some(vec!["fpenv"]),
        "a crate with no dependency"
    );
    assert_eq!(
        of("xtask"),
        Some(vec!["xtask"]),
        "a crate the metadata does not list"
    );
}

#[test]
fn widens_each_kind_of_shared_input_as_far_as_it_reaches() {
    use crate::changes::Widening;
    for (path, expected) in [
        ("Cargo.toml", Some(Widening::Workspace)),
        ("rust-toolchain.toml", Some(Widening::Everything)),
        (".cargo/config.toml", Some(Widening::Everything)),
        ("crates/audio-core/.cargo/config.toml", None),
        ("Cargo.lock", Some(Widening::Lock)),
        (
            "crates/audio-core/Cargo.toml",
            Some(Widening::Manifest("audio-core")),
        ),
        (
            "bindings/audio-wasm/Cargo.toml",
            Some(Widening::Manifest("audio-wasm")),
        ),
        (
            "crates/audio-testkit/Cargo.toml",
            Some(Widening::Manifest("audio-testkit")),
        ),
        (
            "crates/audio-testkit/src/lib.rs",
            Some(Widening::SharedTests("audio-testkit")),
        ),
        (
            "crates/alloc-counter/src/lib.rs",
            Some(Widening::SharedTests("alloc-counter")),
        ),
        ("tooling/x/Cargo.toml", Some(Widening::Everything)),
        ("doc/Cargo.lock", Some(Widening::Everything)),
        ("apps/a/rust-toolchain.toml", Some(Widening::Everything)),
        ("crates/xtask/tests/fixtures/workspace/Cargo.toml", None),
        (
            "crates/xtask/tests/fixtures/workspace/crates/audio-core/Cargo.toml",
            None,
        ),
        ("crates/audio-core/src/Cargo.lock", None),
        ("crates/audio-core/src/lib.rs", None),
        ("Cargo.toml.orig", None),
        ("crates/audio-testkits/a.rs", None),
        ("doc/crates/audio-testkit/a.rs", None),
    ] {
        assert_eq!(crate::changes::widening(path), expected, "{path}");
    }
}

/// A registry package of a lock file: name, version, checksum, dependencies.
type Registered<'text> = (&'text str, &'text str, &'text str, &'text [&'text str]);

/// A lock file as Cargo writes it: every crate of the workspace at 0.1.0, with the
/// dependencies `workspace` gives it (none for the others), then the `registry` packages.
fn lock_text(workspace: &[(&str, &[&str])], registry: &[Registered<'_>]) -> String {
    let dependencies = |names: &[&str]| {
        if names.is_empty() {
            String::new()
        } else {
            let mut list = "dependencies = [\n".to_owned();
            for name in names {
                list.push_str(" \"");
                list.push_str(name);
                list.push_str("\",\n");
            }
            list.push_str("]\n");
            list
        }
    };
    let mut text = "# This file is automatically @generated by Cargo.\n# It is not intended for manual editing.\nversion = 4\n".to_owned();
    for krate in crate::RUST_CRATES {
        let names = workspace
            .iter()
            .find(|(name, _)| *name == krate.name)
            .map_or(&[][..], |(_, names)| *names);
        let _written = write!(
            text,
            "\n[[package]]\nname = \"{}\"\nversion = \"0.1.0\"\n{}",
            krate.name,
            dependencies(names)
        );
    }
    for (name, version, checksum, names) in registry {
        let _written = write!(
            text,
            "\n[[package]]\nname = \"{name}\"\nversion = \"{version}\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"{checksum}\"\n{}",
            dependencies(names)
        );
    }
    text
}

/// What the workspace of the lock tests depends on.
const LOCKED_WORKSPACE: [(&str, &[&str]); 4] = [
    ("audio-core", &["libm"]),
    ("audio-boxes", &["audio-core"]),
    ("audio-testkit", &["audio-core", "proptest"]),
    ("xtask", &["serde_json"]),
];

/// The registry packages of the lock tests.
const LOCKED_REGISTRY: [Registered<'static>; 5] = [
    ("libm", "0.2.16", "c1", &[]),
    ("proptest", "1.0.0", "c2", &["rand"]),
    ("rand", "0.8.0", "c3", &[]),
    ("serde_json", "1.0.0", "c4", &["itoa"]),
    ("itoa", "1.0.0", "c5", &[]),
];

/// The crates [`crate::changes::lock_widened`] finds between the lock of the tests and the one with
/// `registry` instead.
fn widened_by(registry: &[Registered<'_>]) -> Result<Vec<&'static str>, String> {
    crate::changes::lock_widened(
        &lock_text(&LOCKED_WORKSPACE, &LOCKED_REGISTRY),
        &lock_text(&LOCKED_WORKSPACE, registry),
    )
    .map(|crates| crates.into_iter().collect())
}

#[test]
fn widens_to_the_crates_whose_build_reaches_a_changed_lock_entry() {
    let mut registry = LOCKED_REGISTRY.to_vec();
    assert_eq!(widened_by(&registry), Ok(Vec::new()), "nothing changed");
    registry[0] = ("libm", "0.2.16", "c9", &[]);
    assert_eq!(
        widened_by(&registry),
        Ok(vec!["audio-boxes", "audio-core", "audio-testkit"]),
        "a checksum: every crate reaching it, however deep"
    );
    let mut registry2 = LOCKED_REGISTRY.to_vec();
    registry2[4] = ("itoa", "1.0.1", "c5", &[]);
    assert_eq!(
        widened_by(&registry2),
        Ok(vec!["xtask"]),
        "a version, reached through another package"
    );
    let mut registry3 = LOCKED_REGISTRY.to_vec();
    registry3[1] = ("proptest", "1.0.0", "c2", &[]);
    let _removed = registry3.remove(2);
    assert_eq!(
        widened_by(&registry3),
        Ok(vec!["audio-testkit"]),
        "a package removed: the one that reached it changed"
    );
    let mut registry4 = LOCKED_REGISTRY.to_vec();
    registry4.push(("unused", "1.0.0", "c6", &[]));
    assert_eq!(
        widened_by(&registry4),
        Ok(Vec::new()),
        "a package no crate reaches"
    );
    let workspace = [
        ("audio-core", &["libm"][..]),
        ("audio-boxes", &["audio-core"]),
        ("audio-testkit", &["audio-core", "proptest"]),
        ("xtask", &["serde_json", "itoa"]),
    ];
    assert_eq!(
        crate::changes::lock_widened(
            &lock_text(&LOCKED_WORKSPACE, &LOCKED_REGISTRY),
            &lock_text(&workspace, &LOCKED_REGISTRY)
        )
        .map(|crates| crates.into_iter().collect::<Vec<_>>()),
        Ok(vec!["xtask"]),
        "a crate's own dependencies"
    );
}

#[test]
fn follows_a_lock_dependency_by_name_version_and_source() {
    let two = [
        ("serde_json", "1.0.0", "c4", &["itoa 1.0.0"][..]),
        ("itoa", "1.0.0", "c5", &[]),
        ("itoa", "2.0.0", "c7", &[]),
    ];
    let before = lock_text(&[("xtask", &["serde_json"])], &two);
    let mut changed = two;
    changed[2] = ("itoa", "2.0.0", "c8", &[]);
    assert_eq!(
        crate::changes::lock_widened(&before, &lock_text(&[("xtask", &["serde_json"])], &changed))
            .map(|crates| crates.into_iter().collect::<Vec<_>>()),
        Ok(Vec::new()),
        "the version named, not the other"
    );
    changed[1] = ("itoa", "1.0.0", "c9", &[]);
    assert_eq!(
        crate::changes::lock_widened(&before, &lock_text(&[("xtask", &["serde_json"])], &changed))
            .map(|crates| crates.into_iter().collect::<Vec<_>>()),
        Ok(vec!["xtask"]),
        "the version named"
    );
    let sourced = [
        (
            "serde_json",
            "1.0.0",
            "c4",
            &["itoa 1.0.0 (registry+https://github.com/rust-lang/crates.io-index)"][..],
        ),
        ("itoa", "1.0.0", "c5", &[]),
    ];
    let lock = lock_text(&[("xtask", &["serde_json"])], &sourced);
    assert_eq!(
        crate::changes::lock_widened(&lock, &lock).map(|crates| crates.len()),
        Ok(0),
        "a source named"
    );
    for dependencies in [
        &["itoa"][..],
        &["itoa 3.0.0"],
        &["itoa 1.0.0 (git+https://example.invalid)"],
        &["itoa 1.0.0 registry"],
        &["missing"],
    ] {
        let unresolved = lock_text(
            &[("xtask", &["serde_json"])],
            &[
                ("serde_json", "1.0.0", "c4", dependencies),
                ("itoa", "1.0.0", "c5", &[]),
                ("itoa", "2.0.0", "c7", &[]),
            ],
        );
        assert_eq!(
            crate::changes::lock_widened(&unresolved, &unresolved),
            Err(format!(
                "a dependency, `{}`, names no one entry",
                dependencies.join("")
            )),
            "{dependencies:?}: ambiguous, missing or malformed"
        );
    }
}

/// The reasons the lock reader gives for the malformed forms of `lock` the test below
/// writes around `libm`'s entry and after the last one. The fixture lists the registry's
/// packages after every crate of the registry, so these lines are counted from the first
/// registry entry, `libm`'s, and from the end, never retyped.
fn malformed_lock_reasons(lock: &str) -> [String; 8] {
    let libm = lock
        .lines()
        .position(|line| line == "name = \"libm\"")
        .map_or(0, |index| index.saturating_add(1));
    let after = lock.lines().count().saturating_add(2);
    let at = |line: usize, what: &str| format!("line {line} {what}");
    [
        at(libm, "is not `name = \"…\"`"),
        at(libm.saturating_add(1), "is not `version = \"…\"`"),
        at(libm.saturating_add(3), "is not `checksum = \"…\"`"),
        at(
            libm.saturating_add(4),
            "is not the blank line ending an entry",
        ),
        at(
            libm.saturating_sub(2),
            "is not the blank line ending an entry",
        ),
        at(after, "is not `[[package]]`"),
        format!("the entry of line {after} repeats an earlier one"),
        at(libm.saturating_add(2), "is not `source = \"…\"`"),
    ]
}

#[test]
fn reads_a_lock_file_only_as_cargo_writes_it() {
    let lock = lock_text(&LOCKED_WORKSPACE, &LOCKED_REGISTRY);
    assert!(crate::changes::lock_entries(&lock).is_ok(), "as written");
    assert_eq!(
        crate::changes::lock_entries(&lock).map(|entries| entries.len()),
        Ok(crate::RUST_CRATES.len() + LOCKED_REGISTRY.len()),
        "every crate of the workspace and every package"
    );
    let around = malformed_lock_reasons(&lock);
    let first = "it does not start with `version = 4` after its comments";
    for (malformed, reason) in [
        (lock.replacen("version = 4", "version = 3", 1), first),
        (lock.replacen("version = 4\n", "", 1), first),
        (
            lock.replacen(
                "\n[[package]]\nname = \"libm\"",
                "\n[[package]]\nnamee = \"libm\"",
                1,
            ),
            &around[0],
        ),
        (
            lock.replacen("version = \"0.2.16\"", "version = 0.2.16", 1),
            &around[1],
        ),
        (
            lock.replacen("checksum = \"c1\"", "checksum = c1", 1),
            &around[2],
        ),
        (
            lock.replacen("checksum = \"c1\"", "checksum = \"c1\"\nyanked = true", 1),
            &around[3],
        ),
        (
            lock.replacen(" \"libm\",", "  \"libm\",", 1),
            "line 9 is not ` \"<dependency>\",`",
        ),
        (
            lock.replacen(" \"libm\",", " \"libm\"", 1),
            "line 9 is not ` \"<dependency>\",`",
        ),
        (
            lock.replacen(" \"libm\",\n]", " \"libm\",", 1),
            "line 10 is not ` \"<dependency>\",`",
        ),
        (
            format!(
                "{lock}\n[[package]]\nname = \"a\"\nversion = \"1\"\ndependencies = [\n \"b\",\n"
            ),
            "it ends in a list of dependencies",
        ),
        (
            lock.replacen(
                "\n\n[[package]]\nname = \"libm\"",
                "\n[[package]]\nname = \"libm\"",
                1,
            ),
            &around[4],
        ),
        (format!("{lock}\n[metadata]\n"), &around[5]),
        (
            format!(
                "{lock}\n[[package]]\nname = \"libm\"\nversion = \"0.2.16\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"c1\"\n"
            ),
            &around[6],
        ),
        (
            format!("{lock}\n[[package]]\nname = \"a\"\n"),
            "it ends where `version = \"…\"` is expected",
        ),
        (
            format!("{lock}\n[[package]]\n"),
            "it ends where `name = \"…\"` is expected",
        ),
        (
            lock.replacen(
                "source = \"registry+https://github.com/rust-lang/crates.io-index\"",
                "source = registry",
                1,
            ),
            &around[7],
        ),
    ] {
        assert_eq!(
            crate::changes::lock_entries(&malformed).err(),
            Some(reason.to_owned()),
            "{reason}"
        );
        assert_eq!(
            crate::changes::lock_widened(&lock, &malformed),
            Err(reason.to_owned()),
            "{reason}: after"
        );
        assert_eq!(
            crate::changes::lock_widened(&malformed, &lock),
            Err(format!("its earlier version: {reason}")),
            "{reason}: before"
        );
    }
}

#[test]
fn needs_one_entry_of_each_crate_of_the_workspace() {
    let lock = lock_text(&LOCKED_WORKSPACE, &LOCKED_REGISTRY);
    let without = lock.replacen(
        "\n[[package]]\nname = \"audio-cli\"\nversion = \"0.1.0\"\n",
        "",
        1,
    );
    assert!(
        crate::changes::lock_entries(&without).is_ok(),
        "still a lock file"
    );
    assert_eq!(
        crate::changes::lock_widened(&lock, &without),
        Err("it does not hold exactly one entry of audio-cli".to_owned()),
        "a crate of the workspace missing"
    );
}

/// A passing fake whose base holds [`LOCKED_REGISTRY`]'s lock file and whose tree holds
/// `registry`'s, every crate of the workspace with one source file, and `changes` as the
/// changes since the base.
fn locked_fake(registry: &[Registered<'_>], changes: &str) -> Fake {
    let mut fake = passing();
    for (key, value) in [
        (format!("capture {}", CHANGES.0), changes.to_owned()),
        (
            format!(
                "capture git ls-tree --name-only {} -- Cargo.lock",
                MERGE_BASE.1.trim()
            ),
            "Cargo.lock\n".to_owned(),
        ),
        (
            format!("capture git show {}:Cargo.lock", MERGE_BASE.1.trim()),
            lock_text(&LOCKED_WORKSPACE, &LOCKED_REGISTRY),
        ),
        (
            "Cargo.lock".to_owned(),
            lock_text(&LOCKED_WORKSPACE, registry),
        ),
        (
            format!(
                "capture {}",
                crate::metadata_command(crate::CargoWorkspace::Root).line()
            ),
            closure_metadata(false),
        ),
    ] {
        drop(fake.files.insert(key, value));
    }
    for krate in crate::RUST_CRATES {
        drop(fake.files.insert(
            format!("{}/src/lib.rs", krate.dir),
            "fn a() {}\n".to_owned(),
        ));
    }
    fake
}

/// The diff of [`locked_fake`]'s change, unless it is not of a Rust file or of a file of a
/// crate of `whole`, then the source of each crate of `whole`, added whole, in the order of
/// the workspace.
fn whole_crates(change: &str, whole: &[&str]) -> String {
    let mut diff = if change
        .lines()
        .next()
        .and_then(|header| Path::new(header).extension())
        .is_none_or(|extension| extension != "rs")
        || crate::RUST_CRATES.iter().any(|krate| {
            whole.contains(&krate.name) && change.contains(&format!(" b/{}/", krate.dir))
        }) {
        String::new()
    } else {
        change.to_owned()
    };
    for krate in crate::RUST_CRATES {
        if whole.contains(&krate.name) {
            diff.push_str(&crate::new_file_diff(
                &format!("{}/src/lib.rs", krate.dir),
                "fn a() {}\n",
            ));
        }
    }
    diff
}

#[test]
fn mutates_the_crates_a_change_of_the_lock_file_reaches() {
    let mut registry = LOCKED_REGISTRY.to_vec();
    registry[4] = ("itoa", "1.0.1", "c5", &[]);
    let change = one_line_change("Cargo.lock");
    let mut fake = locked_fake(&registry, &change);
    let said = mutate_changes(&mut fake);
    assert_eq!(
        fake.files.get(CHANGES_DIFF).cloned(),
        Some(whole_crates(&change, &["xtask"])),
        "xtask alone, whole"
    );
    assert!(
        said.contains("  Cargo.lock changed: every mutant of xtask\n")
            && !said.contains("every Rust mutant"),
        "said: {said}"
    );
    let mut same = locked_fake(&LOCKED_REGISTRY, &change);
    let _quiet = mutate_changes(&mut same);
    assert_eq!(
        same.files.get(CHANGES_DIFF).cloned(),
        None,
        "an entry-for-entry identical lock widens nothing, and is no Rust file to mutate"
    );
    let mut unreadable = locked_fake(&registry, &change);
    drop(unreadable.files.remove("Cargo.lock"));
    assert!(
        mutate_changes(&mut unreadable).contains(
            "  Cargo.lock changed and cannot be narrowed (cannot read it: missing): every Rust \
             mutant\n"
        ),
        "a lock file that cannot be read here widens to everything, saying why"
    );
    let mut malformed = locked_fake(&registry, &change);
    drop(
        malformed
            .files
            .insert("Cargo.lock".to_owned(), "nonsense\n".to_owned()),
    );
    assert!(
        mutate_changes(&mut malformed).contains(
            "  Cargo.lock changed and cannot be narrowed (it does not start with `version = 4` \
             after its comments): every Rust mutant\n"
        ),
        "a lock file not as Cargo writes it widens to everything, saying where"
    );
}

#[test]
fn mutates_the_crates_that_build_a_changed_manifest_or_the_shared_test_crate() {
    for (path, whole) in [
        (
            "crates/audio-core/Cargo.toml",
            &["audio-core", "audio-boxes"][..],
        ),
        ("crates/audio-hash/Cargo.toml", &["audio-hash"]),
        (
            "crates/audio-testkit/src/runner.rs",
            &["audio-boxes", "audio-testkit"],
        ),
    ] {
        let change = one_line_change(path);
        let mut fake = locked_fake(&LOCKED_REGISTRY, &change);
        let said = mutate_changes(&mut fake);
        assert_eq!(
            fake.files.get(CHANGES_DIFF).cloned(),
            Some(whole_crates(&change, whole)),
            "{path}"
        );
        for name in whole {
            assert!(
                said.contains(&format!("  {path} changed: every mutant of {name}\n")),
                "{path}: said of {name}: {said}"
            );
        }
    }
    let fixture = one_line_change("crates/xtask/tests/fixtures/workspace/Cargo.toml");
    let mut fake = locked_fake(&LOCKED_REGISTRY, &fixture);
    let said = mutate_changes(&mut fake);
    assert_eq!(
        fake.files.get(CHANGES_DIFF).cloned(),
        Some(whole_crates(&fixture, &["xtask"])),
        "a fixture manifest is xtask's test code"
    );
    assert!(
        said.contains("  the integration tests of xtask changed: every mutant of xtask\n"),
        "said: {said}"
    );
}

#[test]
fn keeps_the_crates_a_record_proves_that_a_lock_change_does_not_reach() {
    let mut registry = LOCKED_REGISTRY.to_vec();
    registry[4] = ("itoa", "1.0.1", "c5", &[]);
    let mut fake = proven_fake(&proof_record(PROVEN[2]), &one_line_change("Cargo.lock"));
    for (key, value) in [
        (
            format!(
                "capture git ls-tree --name-only {} -- Cargo.lock",
                PROVEN[0]
            ),
            "Cargo.lock\n".to_owned(),
        ),
        (
            format!("capture git show {}:Cargo.lock", PROVEN[0]),
            lock_text(&LOCKED_WORKSPACE, &LOCKED_REGISTRY),
        ),
        (
            "Cargo.lock".to_owned(),
            lock_text(&LOCKED_WORKSPACE, &registry),
        ),
    ] {
        drop(fake.files.insert(key, value));
    }
    let said = mutate_changes(&mut fake);
    assert!(
        said.contains(&proven_line("audio-core"))
            && said.contains(&proven_line("audio-hash"))
            && !said.contains("ignored the mutation proof record"),
        "the record still proves what the change does not reach: {said}"
    );
    let mut xtask = proven_fake(&proof_record(PROVEN[2]), &one_line_change("Cargo.lock"));
    let mut reaching = LOCKED_REGISTRY.to_vec();
    reaching[0] = ("libm", "0.2.16", "c9", &[]);
    for (key, value) in [
        (
            format!(
                "capture git ls-tree --name-only {} -- Cargo.lock",
                PROVEN[0]
            ),
            "Cargo.lock\n".to_owned(),
        ),
        (
            format!("capture git show {}:Cargo.lock", PROVEN[0]),
            lock_text(&LOCKED_WORKSPACE, &LOCKED_REGISTRY),
        ),
        (
            "Cargo.lock".to_owned(),
            lock_text(&LOCKED_WORKSPACE, &reaching),
        ),
    ] {
        drop(xtask.files.insert(key, value));
    }
    let reached = mutate_changes(&mut xtask);
    assert!(
        !reached.contains(&proven_line("audio-core"))
            && reached.contains(&proven_line("audio-hash")),
        "the engine core reaches the changed entry, the render hash does not: {reached}"
    );
}

#[test]
fn ignores_a_proof_record_of_another_compiler_or_linker() {
    let other = |edit: &dyn Fn(&mut crate::ProofRecord)| {
        let mut record = proof_record(PROVEN[2]);
        edit(&mut record);
        proven_fake(&record, "")
    };
    ignored(
        &mut other(&|record| record.tools.compiler = "0000 LLVM 23.1.1".to_owned()),
        "it was made with the compiler 0000 LLVM 23.1.1, not \
         58cf8f9c35b355199727627d07ae34a4e2846d67 LLVM 23.1.1",
    );
    ignored(
        &mut other(&|record| record.tools.linker = "Apple clang version 17.0.0".to_owned()),
        "it was made with the linker Apple clang version 17.0.0, not Apple clang version 14.0.3 \
         (clang-1403.0.22.14.1)",
    );
    for (missing, key) in [
        (
            "commit-hash: 58cf8f9c35b355199727627d07ae34a4e2846d67\n",
            "commit-hash",
        ),
        ("LLVM version: 23.1.1\n", "LLVM version"),
    ] {
        let mut unbuilt = proven_fake(&proof_record(PROVEN[2]), "");
        drop(unbuilt.files.insert(
            format!("capture {}", HOST.0),
            HOST.1.replacen(missing, "", 1),
        ));
        ignored(&mut unbuilt, &format!("`rustc -vV` names no {key}"));
    }
    let mut no_linker = proven_fake(&proof_record(PROVEN[2]), "");
    no_linker.broken = Some(LINKER.0.to_owned());
    ignored(
        &mut no_linker,
        "cannot read the version of the linker driver: cannot run `cc --version`: not found",
    );
    ignored(
        &mut other(&|record| record.tools.git = "git version 2.30.0".to_owned()),
        "it was made with the git git version 2.30.0, not git version 2.39.2 (Apple Git-143)",
    );
    let mut no_git = proven_fake(&proof_record(PROVEN[2]), "");
    no_git.broken = Some("git --version".to_owned());
    ignored(
        &mut no_git,
        "cannot read the version of git: cannot run `git --version`: not found",
    );
    for silent in ["", "\nclang\n", "  \n"] {
        let mut unnamed = proven_fake(&proof_record(PROVEN[2]), "");
        drop(
            unnamed
                .files
                .insert(format!("capture {}", LINKER.0), silent.to_owned()),
        );
        ignored(&mut unnamed, "`cc --version` names no version");
    }
}

/// A workspace manifest shaped as the project's: members, package fields, shared
/// dependencies with a comment, lints and a profile.
const MANIFEST: &str = "[workspace]\nresolver = \"3\"\nmembers = [\"crates/*\"]\n\n[workspace.package]\nversion = \"0.1.0\"\n\n[workspace.dependencies]\n# Pinned.\nlibm = { version = \"=0.2.16\", default-features = false }\nsha2 = { version = \"=0.11.1\", default-features = false }\n\n[workspace.lints.rust]\nunsafe_code = \"deny\"\n\n[profile.release]\nlto = \"fat\"\n";

/// [`MANIFEST`] with `from` replaced by `to`, which it holds.
fn manifest_with(from: &str, to: &str) -> String {
    assert!(MANIFEST.contains(from), "{from}");
    MANIFEST.replacen(from, to, 1)
}

/// What [`crate::changes::manifest_widened`] finds from [`MANIFEST`] to `manifest`, names sorted.
fn widened_entries(manifest: &str) -> Result<Vec<String>, String> {
    crate::changes::manifest_widened(MANIFEST, manifest).map(|names| names.into_iter().collect())
}

#[test]
fn finds_the_shared_dependencies_a_workspace_manifest_changes() {
    let libm = "libm = { version = \"=0.2.16\", default-features = false }";
    let sha2 = "sha2 = { version = \"=0.11.1\", default-features = false }";
    for (manifest, expected, what) in [
        (MANIFEST.to_owned(), &[][..], "unchanged"),
        (
            manifest_with("# Pinned.\n", "# Pinned exactly.\n\n  # Twice.\n"),
            &[],
            "a comment or a blank line of the table",
        ),
        (
            manifest_with(&format!("{libm}\n{sha2}"), &format!("{sha2}\n{libm}")),
            &[],
            "the same entries in another order",
        ),
        (
            manifest_with("=0.2.16", "=0.2.17"),
            &["libm"],
            "an entry changed",
        ),
        (
            manifest_with(
                "default-features = false }\nsha2",
                "default-features = true }\nsha2",
            ),
            &["libm"],
            "its features",
        ),
        (
            manifest_with(libm, &format!("\t {libm} # {{ it's \"")),
            &["libm"],
            "indented, with a comment holding anything",
        ),
        (
            manifest_with(
                "# Pinned.\n",
                "audio-core = { path = \"crates/audio-core\" }\na_b  =\t[\"x\", { y = \"#{[\" }]\n",
            ),
            &["a_b", "audio-core"],
            "entries added, names with a dash or an underscore, any value on one line",
        ),
        (
            manifest_with(
                "sha2 = { version = \"=0.11.1\", default-features = false }\n",
                "",
            ),
            &["sha2"],
            "an entry removed",
        ),
    ] {
        assert_eq!(
            widened_entries(&manifest),
            Ok(expected.iter().map(|name| (*name).to_owned()).collect()),
            "{what}"
        );
    }
}

#[test]
fn leaves_out_a_change_of_the_workspace_lints_alone() {
    let lint =
        |manifest: &str| manifest.replacen("unsafe_code = \"deny\"", "unsafe_code = \"forbid\"", 1);
    for (manifest, expected, what) in [
        (lint(MANIFEST), &[][..], "a lint level changed"),
        (
            manifest_with(
                "[profile.release]",
                "[workspace.lints.rustdoc]\nbroken_intra_doc_links = \"deny\"\n\n[profile.release]",
            ),
            &[],
            "a lint table added",
        ),
        (
            manifest_with(
                "[workspace.lints.rust]\nunsafe_code = \"deny\"\n",
                "[workspace.lints.rust]\nunsafe_code = \"deny\"\ncheck = [\n  \"x\",\n]\n",
            ),
            &[],
            "a lint written on several lines",
        ),
        (
            lint(&manifest_with("=0.2.16", "=0.2.17")),
            &["libm"],
            "a lint and an entry changed: the entry",
        ),
    ] {
        assert_eq!(
            widened_entries(&manifest),
            Ok(expected.iter().map(|name| (*name).to_owned()).collect()),
            "{what}"
        );
    }
    let outside = "it changed outside the entries of `[workspace.dependencies]`".to_owned();
    let under = |header: &str| {
        let before = manifest_with("[workspace.lints.rust]", header);
        let after = lint(&before);
        crate::changes::manifest_widened(&before, &after)
    };
    for header in [
        "[workspace.lints.rust]",
        "[workspace.lints]",
        "[workspace.lints.a-b_9]",
    ] {
        assert_eq!(under(header), Ok(BTreeSet::new()), "{header}");
    }
    for header in [
        "[workspace.lints.rust] # x",
        "[workspace.lints.rust]\r",
        "[workspace.lintsx]",
        "[workspace.lints.]",
        "[workspace.lints.rust.x]",
        "[workspace.lints.\"rust\"]",
        "[ workspace.lints.rust ]",
        " [workspace.lints.rust]",
        "[[workspace.lints.rust]]",
        "[workspace.lints.r\u{fc}st]",
    ] {
        assert_eq!(under(header), Err(outside.clone()), "{header:?}");
    }
    let continued = manifest_with(
        "unsafe_code = \"deny\"\n",
        "unsafe_code = \"deny\"\nx = [\n[\"a\"],\n]\ny = 1\n",
    );
    assert_eq!(
        crate::changes::manifest_widened(&continued, &continued.replacen("y = 1", "y = 2", 1)),
        Err(outside.clone()),
        "a line of a lint's value beginning with `[` ends what is left out"
    );
    assert_eq!(
        widened_entries(&lint(&manifest_with("lto = \"fat\"", "lto = \"thin\""))),
        Err(outside),
        "a lint and a profile changed: the profile"
    );
}

#[test]
fn widens_to_everything_a_workspace_manifest_changed_elsewhere_or_unread() {
    let outside = "it changed outside the entries of `[workspace.dependencies]`".to_owned();
    for (manifest, what) in [
        (
            manifest_with("lto = \"fat\"", "lto = \"thin\""),
            "a profile",
        ),
        (
            manifest_with("[\"crates/*\"]", "[\"crates/*\", \"x\"]"),
            "the members",
        ),
        (
            manifest_with("resolver = \"3\"", "resolver = \"2\""),
            "the resolver",
        ),
        (manifest_with("0.1.0", "0.2.0"), "a package field"),
        (
            format!("{MANIFEST}\n[patch.crates-io]\nlibm = {{ path = \"x\" }}\n"),
            "a patch",
        ),
        (
            format!("{MANIFEST}\n[replace]\n\"libm:0.2.16\" = {{ path = \"x\" }}\n"),
            "a replacement",
        ),
        (
            manifest_with(
                "\n[workspace.lints.rust]",
                "[workspace.dependencies.serde]\nversion = \"1\"\n\n[workspace.lints.rust]",
            ),
            "an entry written as a table of its own",
        ),
        (
            manifest_with("[workspace]\n", "[workspace]\nx = 1\n"),
            "a key not understood",
        ),
        (format!("{MANIFEST}\n"), "the last line"),
    ] {
        assert_eq!(widened_entries(&manifest), Err(outside.clone()), "{what}");
    }
    let one_header = "it does not hold exactly one line `[workspace.dependencies]`".to_owned();
    let not_entry =
        |line: usize| format!("line {line} is not one entry `<name> = <value>` on one line");
    for (manifest, expected, what) in [
        (
            "not a manifest {{{".to_owned(),
            one_header.clone(),
            "unparsable",
        ),
        (
            manifest_with("[workspace.dependencies]", "[workspace.dependencies] # x"),
            one_header.clone(),
            "a header written otherwise",
        ),
        (
            format!("{MANIFEST}[workspace.dependencies]\n"),
            one_header,
            "the table twice",
        ),
        (
            manifest_with("version = \"0.1.0\"", "description = \"\"\"\nx\"\"\""),
            "it holds a multi-line string".to_owned(),
            "a multi-line basic string",
        ),
        (
            manifest_with("version = \"0.1.0\"", "description = '''\nx'''"),
            "it holds a multi-line string".to_owned(),
            "a multi-line literal string",
        ),
        (
            manifest_with("sha2", "libm"),
            "line 11 repeats an entry".to_owned(),
            "an entry twice",
        ),
    ] {
        assert_eq!(widened_entries(&manifest), Err(expected), "{what}");
    }
    for refused in [
        "libm.version = \"1\"",
        "\"libm\" = \"1\"",
        "= \"1\"",
        "libm \"1\"",
        "libm = {",
        "libm = { version = \"1\", features = [",
        "libm = { version = \"1\" # }",
        "libm = \"1",
        "libm = \"\\\\\"",
        "libm = 'x'",
        "libm = { a = [1 }",
        "libm = { a = [1 } ]",
        "libm = 1 }",
        "libm = ]",
        "lïbm = \"1\"",
    ] {
        assert_eq!(
            widened_entries(&manifest_with("# Pinned.", refused)),
            Err(not_entry(9)),
            "{refused}"
        );
    }
    assert_eq!(
        crate::changes::manifest_widened("[workspace]\n", MANIFEST),
        Err(
            "its earlier version: it does not hold exactly one line `[workspace.dependencies]`"
                .to_owned()
        ),
        "the earlier version, said as such"
    );
}

/// What the crates of the manifest tests declare: the engine core `libm`, the render hash
/// `sha2`, xtask `serde_json` under the name `json`; the boxes build the engine core.
const DECLARING_METADATA: &str = r#"{"workspace_root":"/r","packages":[{"name":"audio-core","dependencies":[{"name":"libm","kind":null,"rename":null}]},{"name":"audio-hash","dependencies":[{"name":"sha2","kind":null}]},{"name":"audio-boxes","dependencies":[{"name":"audio-core","kind":null,"path":"/r/crates/audio-core"}]},{"name":"xtask","dependencies":[{"name":"serde_json","kind":null,"rename":"json"}]},{"name":"other","dependencies":[{"name":"libm","kind":null}]}]}"#;

#[test]
fn reads_the_name_each_crate_gives_its_dependencies() {
    let builds = crate::changes::crate_builds(DECLARING_METADATA).expect("valid metadata");
    let declaring: Vec<(&str, Vec<&str>)> = builds
        .declaring
        .iter()
        .map(|(name, crates)| (name.as_str(), crates.iter().copied().collect()))
        .collect();
    assert_eq!(
        declaring,
        [
            ("audio-core", vec!["audio-boxes"]),
            ("json", vec!["xtask"]),
            ("libm", vec!["audio-core"]),
            ("sha2", vec!["audio-hash"]),
        ],
        "by its name in the manifest, a crate outside the workspace apart"
    );
    assert_eq!(
        builds.closures,
        crate::changes::crate_closures(DECLARING_METADATA).expect("valid metadata"),
        "and the closures"
    );
    assert_eq!(
        crate::changes::crate_builds(
            r#"{"workspace_root":"/r","packages":[{"name":"audio-hash","dependencies":[{"rename":null}]}]}"#
        ).map_err(|failure| failure.summary), Err("cargo metadata lists a dependency without a name".to_owned()), "a dependency without a name");
    assert_eq!(
        crate::changes::crate_builds("not json").map_err(|failure| failure.summary),
        Err("invalid cargo metadata: expected ident at line 1 column 2".to_owned()),
        "not JSON"
    );
}

#[test]
fn narrows_a_change_of_the_workspace_manifest_real_git_shows() {
    let dir = scratch("workspace-manifest");
    std::fs::write(dir.join("Cargo.toml"), MANIFEST).expect("a manifest");
    let _init = hermetic_git(&dir, &["init", "--quiet", "--initial-branch=main"]);
    let _add = hermetic_git(&dir, &["add", "."]);
    let _commit = hermetic_git(&dir, &["commit", "--quiet", "-m", "base"]);
    let base = hermetic_git(&dir, &["rev-parse", "HEAD"]);
    let builds = crate::changes::crate_builds(DECLARING_METADATA).expect("valid metadata");
    let reach = |manifest: Option<&str>| {
        match manifest {
            Some(text) => std::fs::write(dir.join("Cargo.toml"), text).expect("written"),
            None => std::fs::remove_file(dir.join("Cargo.toml")).expect("removed"),
        }
        crate::changes::reach(
            &mut system(dir.path()),
            base.trim(),
            ["Cargo.toml"],
            &builds,
        )
    };
    let crates = |names: &[&'static str]| {
        crate::changes::Reach::Crates(
            names
                .iter()
                .map(|name| (*name, "Cargo.toml".to_owned()))
                .collect(),
        )
    };
    let everything = |why: &str| crate::changes::Reach::Everything {
        path: "Cargo.toml".to_owned(),
        unnarrowed: Some(why.to_owned()),
    };
    assert_eq!(
        reach(Some(&manifest_with("=0.2.16", "=0.2.17"))),
        crates(&["audio-core", "audio-boxes"]),
        "an entry changed: the crate declaring it, and every crate building that one"
    );
    assert_eq!(
        reach(Some(&manifest_with(
            "sha2 = { version = \"=0.11.1\", default-features = false }\n",
            ""
        ))),
        crates(&["audio-hash"]),
        "an entry removed"
    );
    assert_eq!(
        reach(Some(&manifest_with(
            "# Pinned.\n",
            "json = { package = \"serde_json\", version = \"1\" }\nserde = \"1\"\n"
        ))),
        crates(&["xtask"]),
        "entries added: one declared under its name, one by no crate"
    );
    assert_eq!(
        reach(Some(&manifest_with("# Pinned.", "# Pinned exactly."))),
        crates(&[]),
        "a comment: nothing"
    );
    let outside = "it changed outside the entries of `[workspace.dependencies]`";
    assert_eq!(
        reach(Some(&manifest_with("lto = \"fat\"", "lto = \"thin\""))),
        everything(outside),
        "a profile: every Rust mutant, said why"
    );
    assert_eq!(
        reach(Some(&manifest_with(
            "unsafe_code = \"deny\"",
            "unsafe_code = \"forbid\""
        ))),
        crates(&[]),
        "a lint: nothing"
    );
    assert_eq!(
        reach(Some(&manifest_with("=0.2.16\"", "=0.2.16"))),
        everything("line 10 is not one entry `<name> = <value>` on one line"),
        "unparsable"
    );
    assert!(
        matches!(
            reach(None),
            crate::changes::Reach::Everything { unnarrowed: Some(why), .. }
                if why.starts_with("cannot read it: ")
        ),
        "a manifest that cannot be read"
    );
    std::fs::write(dir.join("Cargo.toml"), MANIFEST).expect("restored");
    let unrooted = crate::changes::reach(
        &mut system(dir.path()),
        "0000000000000000000000000000000000000000",
        ["Cargo.toml"],
        &builds,
    );
    assert!(
        matches!(
            &unrooted,
            crate::changes::Reach::Everything { unnarrowed: Some(why), .. }
                if why.starts_with("cannot read Cargo.toml at the merge base 0000000000000000000000000000000000000000: `git ls-tree --name-only 0000000000000000000000000000000000000000 -- Cargo.toml` exited with code ")
        ),
        "a commit git cannot show it at: {unrooted:?}"
    );
}

#[test]
fn says_a_shared_input_the_base_lacks_is_missing_there() {
    let dir = scratch("missing-at-base");
    std::fs::write(dir.join("Cargo.toml"), MANIFEST).expect("a manifest");
    let _init = hermetic_git(&dir, &["init", "--quiet", "--initial-branch=main"]);
    let _add = hermetic_git(&dir, &["add", "."]);
    let _commit = hermetic_git(&dir, &["commit", "--quiet", "-m", "base"]);
    let base = hermetic_git(&dir, &["rev-parse", "HEAD"]);
    std::fs::write(dir.join("Cargo.lock"), "version = 4\n").expect("a lock file");
    let builds = crate::changes::crate_builds(DECLARING_METADATA).expect("valid metadata");
    assert_eq!(
        crate::changes::reach(
            &mut system(dir.path()),
            base.trim(),
            ["Cargo.lock"],
            &builds
        ),
        crate::changes::Reach::Everything {
            path: "Cargo.lock".to_owned(),
            unnarrowed: Some(format!("missing at {}", base.trim())),
        },
        "a file the base lacks, said as such"
    );
}

#[test]
fn mutates_the_crates_a_change_of_the_workspace_manifest_reaches() {
    let change = one_line_change("Cargo.toml");
    let removed = manifest_with(
        "sha2 = { version = \"=0.11.1\", default-features = false }\n",
        "",
    );
    let manifest_fake = |base: &str, manifest: &str| {
        let mut fake = locked_fake(&LOCKED_REGISTRY, &change);
        for (key, value) in [
            (
                format!("capture git ls-tree --name-only {base} -- Cargo.toml"),
                "Cargo.toml\n".to_owned(),
            ),
            (
                format!("capture git show {base}:Cargo.toml"),
                MANIFEST.to_owned(),
            ),
            ("Cargo.toml".to_owned(), manifest.to_owned()),
        ] {
            drop(fake.files.insert(key, value));
        }
        fake
    };
    let mut fake = manifest_fake(MERGE_BASE.1.trim(), &removed);
    let said = mutate_changes(&mut fake);
    assert_eq!(
        fake.files.get(CHANGES_DIFF).cloned(),
        Some(whole_crates(&change, &["audio-hash"])),
        "the render hash alone, whole"
    );
    assert!(
        said.contains("  Cargo.toml changed: every mutant of audio-hash\n")
            && !said.contains("every Rust mutant"),
        "said: {said}"
    );
    let mut profile = manifest_fake(
        MERGE_BASE.1.trim(),
        &manifest_with("lto = \"fat\"", "lto = \"thin\""),
    );
    assert!(
        mutate_changes(&mut profile).contains(
            "  Cargo.toml changed and cannot be narrowed (it changed outside the entries of \
             `[workspace.dependencies]`): every Rust mutant\n"
        ),
        "a profile widens to everything, saying why"
    );
    let mut lints = manifest_fake(
        MERGE_BASE.1.trim(),
        &manifest_with("unsafe_code = \"deny\"", "unsafe_code = \"forbid\""),
    );
    let linted = mutate_changes(&mut lints);
    assert!(
        linted.contains("  no Rust file changed: no mutant to test\n")
            && !linted.contains("Cargo.toml changed")
            && !linted.contains("every Rust mutant"),
        "a lint widens to nothing: {linted}"
    );
    assert_eq!(lints.files.get(CHANGES_DIFF), None, "nothing mutated");
    let mut record = proven_fake(&proof_record(PROVEN[2]), &change);
    for (key, value) in [
        (
            format!(
                "capture git ls-tree --name-only {} -- Cargo.toml",
                PROVEN[0]
            ),
            "Cargo.toml\n".to_owned(),
        ),
        (
            format!("capture git show {}:Cargo.toml", PROVEN[0]),
            MANIFEST.to_owned(),
        ),
        ("Cargo.toml".to_owned(), removed),
    ] {
        drop(record.files.insert(key, value));
    }
    let kept = mutate_changes(&mut record);
    assert!(
        kept.contains(&proven_line("audio-core"))
            && !kept.contains(&proven_line("audio-hash"))
            && !kept.contains("ignored the mutation proof record"),
        "a record still proves what the change does not reach: {kept}"
    );
}

/// Checks that `fake` ignores its proof record for `reason`, saying so, and mutates the
/// changes since the base as without it.
fn ignored(fake: &mut Fake, reason: &str) {
    let said = mutate_changes(fake);
    assert!(
        said.contains(&format!(
            "  ignored the mutation proof record {}: {reason}\n",
            record_path(PROVEN[2])
        )) && !said.contains("proven by an earlier check"),
        "{reason}: said, then nothing proven: {said}"
    );
    assert_eq!(
        fake.files.get(CHANGES_DIFF).cloned(),
        Some(since_base()),
        "{reason}: the diff since the base"
    );
}

/// The steps of `cargo xtask mutants --changed dsp/block-2`, with the fake's jobs.
fn changed_lines_steps() -> Vec<Step> {
    mutants_steps(&Target::ChangedLines(fake_revision()), FAKE_JOBS)
}

/// A passing fake that answers `cargo xtask mutants --changed dsp/block-2`.
fn changed_lines_fake() -> Fake {
    let mut fake = passing();
    revision_answers(&mut fake);
    fake
}

#[test]
fn mutates_the_lines_changed_and_never_widens() {
    // Each change the default check widens: unit tests (to their source), integration tests
    // and test data (to the crate), a lock entry (to the crates reaching it).
    let rust = [
        "crates/audio-core/src/lib.rs",
        "crates/audio-core/src/plan_tests.rs",
        "crates/audio-core/tests/laws.rs",
    ];
    let changes: String = rust
        .iter()
        .chain(&["spec/render-hash-vectors.json", "Cargo.lock"])
        .map(|path| one_line_change(path))
        .collect();
    let rust_changes: String = rust.iter().map(|path| one_line_change(path)).collect();
    let files = [
        ("crates/audio-core/src/lib.rs", "fn a() {}\n".to_owned()),
        ("crates/audio-core/src/plan.rs", declares("plan_tests.rs")),
        (
            "crates/audio-core/src/plan_tests.rs",
            "fn t() {}\n".to_owned(),
        ),
        ("crates/audio-hash/src/lib.rs", "fn h() {}\n".to_owned()),
    ];
    let fake_with = || {
        let mut fake = changed_lines_fake();
        let _changes = fake
            .files
            .insert(format!("capture {}", CHANGES.0), changes.clone());
        for (path, text) in &files {
            let _source = fake.files.insert((*path).to_owned(), text.clone());
        }
        fake
    };
    let mut fake = fake_with();
    let mut out = Vec::new();
    assert_eq!(
        run_steps(&changed_lines_steps(), &mut fake, &mut out),
        Ok(())
    );
    assert_eq!(
        fake.files.get(CHANGES_DIFF).cloned(),
        Some(rust_changes),
        "the Rust lines git wrote, nothing added whole"
    );
    assert!(
        fake.ran
            .contains(&changes_mutants_command(FAKE_JOBS).line()),
        "mutated: {:?}",
        fake.ran
    );
    let record = |line: &str| line.contains(PROOF_RECORDS) || line.contains(MEASURED_BASE);
    assert!(
        !fake.ran.iter().any(|line| record(line)),
        "no proof record read or written: {:?}",
        fake.ran
    );
    assert!(
        fake.ran.contains(&format!("capture {}", LISTED.0))
            && !fake.ran.contains(&format!("capture {}", MERGE_BASE.0)),
        "listed, measured from the revision, never from the upstream: {:?}",
        fake.ran
    );
    let said = String::from_utf8(out).expect("UTF-8");
    assert!(
        said.contains(
            "  the Rust lines changed since abc1230 feat: the base (merge base of HEAD with \
             dsp/block-2), not widened to their crates: 2 mutants to run\n"
        ),
        "said: {said}"
    );
    // The default check on the same changes widens, and reads and writes its record.
    let mut widened = fake_with();
    let _said = mutate_changes(&mut widened);
    assert_ne!(
        widened.files.get(CHANGES_DIFF),
        fake.files.get(CHANGES_DIFF),
        "the default check widens the same changes"
    );
    assert!(
        widened.ran.iter().any(|line| record(line)),
        "the default check reads or writes its record: {:?}",
        widened.ran
    );
}

#[test]
fn mutates_nothing_when_no_rust_line_changed() {
    let mut fake = changed_lines_fake();
    let _changes = fake
        .files
        .insert(format!("capture {}", CHANGES.0), String::new());
    let mut out = Vec::new();
    assert_eq!(
        run_steps(&changed_lines_steps(), &mut fake, &mut out),
        Ok(())
    );
    assert!(
        !fake
            .ran
            .iter()
            .any(|line| line.starts_with("cargo mutants")
                || line == "write target/xtask/changes.diff"),
        "nothing written, nothing mutated: {:?}",
        fake.ran
    );
    assert!(
        String::from_utf8(out).expect("UTF-8").contains(
            "  no Rust line changed since abc1230 feat: the base (merge base of HEAD with \
             dsp/block-2): no mutant to test\n"
        ),
        "said"
    );
}

#[test]
fn reports_why_the_lines_changed_cannot_be_mutated() {
    let changes = format!("{} :(exclude)crates/visual-gpu/", CHANGES.0);
    for (broken, expected) in [
        (
            "git ls-files -v -z",
            "cannot read the work tree: cannot run `git ls-files -v -z`: not found".to_owned(),
        ),
        (
            // The root's diff, the GPU workspace left out.
            changes.as_str(),
            format!("cannot read the changes: cannot run `{changes}`: not found"),
        ),
        (
            NEW_RUST_FILES.0,
            format!(
                "cannot list the new Rust files: cannot run `{}`: not found",
                NEW_RUST_FILES.0
            ),
        ),
        (
            "git merge-base HEAD fed4560000000000000000000000000000000000",
            "cannot find the merge base of HEAD with dsp/block-2: cannot run `git merge-base \
             HEAD fed4560000000000000000000000000000000000`: not found"
                .to_owned(),
        ),
        (
            "git show -s --format=%h %s abc1230000000000000000000000000000000000",
            "cannot describe the merge base: cannot run `git show -s --format=%h %s \
             abc1230000000000000000000000000000000000`: not found"
                .to_owned(),
        ),
        (
            "write target/xtask/changes.diff",
            "cannot write target/xtask/changes.diff: read-only".to_owned(),
        ),
        (
            LISTED.0,
            format!("cannot list the mutants: cannot run `{}`: not found", LISTED.0),
        ),
        (
            "cargo mutants --no-config --output=. --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --cargo-arg=--config=target.'cfg(target_os = \"linux\")'.runner=[\"prlimit\", \"--as=4294967296\"] --test-tool=nextest --cargo-test-arg=--max-fail=1:immediate --cargo-test-arg=--no-tests=warn --workspace --in-diff target/xtask/changes.diff --jobs 3 --timeout-multiplier 15 --minimum-test-timeout 60 -- -- --include-ignored",
            "cannot run `cargo mutants --no-config --output=. --cap-lints=true --gitignore=true --cargo-arg=--locked --cargo-arg=--config=profile.test.debug=false --cargo-arg=--config=target.'cfg(target_os = \"linux\")'.runner=[\"prlimit\", \"--as=4294967296\"] --test-tool=nextest --cargo-test-arg=--max-fail=1:immediate --cargo-test-arg=--no-tests=warn --workspace --in-diff target/xtask/changes.diff --jobs 3 --timeout-multiplier 15 --minimum-test-timeout 60 -- -- --include-ignored`: not found".to_owned(),
        ),
    ] {
        let mut fake = changed_lines_fake();
        fake.broken = Some(broken.to_owned());
        assert_eq!(
            run_steps(&changed_lines_steps(), &mut fake, &mut Vec::new()),
            failed(format!(
                "Rust mutation testing of the lines changed: {expected}"
            )),
            "{broken}"
        );
    }
}

#[test]
fn counts_one_mutant_in_the_singular() {
    let mut fake = changed_lines_fake();
    let _one = fake.files.insert(
        format!("capture {}", LISTED.0),
        "crates/a/src/b.rs:1:16: replace f -> u8 with 0\n".to_owned(),
    );
    let mut out = Vec::new();
    assert_eq!(
        run_steps(&changed_lines_steps(), &mut fake, &mut out),
        Ok(())
    );
    let said = String::from_utf8(out).expect("UTF-8");
    assert!(
        said.contains(
            "  the Rust lines changed since abc1230 feat: the base (merge base of HEAD with \
             dsp/block-2), not widened to their crates: 1 mutant to run\n"
        ),
        "said: {said}"
    );
}

#[test]
fn mutates_the_gpu_workspace_whole_when_its_lines_changed() {
    let mut gpu_only = changed_lines_fake();
    let _gpu_change = gpu_only.files.insert(
        format!("capture {}", CHANGES.0),
        one_line_change("crates/visual-gpu/src/lib.rs"),
    );
    let mut out = Vec::new();
    assert_eq!(
        run_steps(&changed_lines_steps(), &mut gpu_only, &mut out),
        Ok(())
    );
    let said = String::from_utf8(out).expect("UTF-8");
    assert!(
        said.contains(
            "  no Rust line of the root's workspace changed since abc1230 feat: the base \
             (merge base of HEAD with dsp/block-2)\n  crates/visual-gpu changed: every mutant \
             of the GPU workspace, its lines not mutated alone\n"
        ),
        "the root's diff leaves the GPU workspace out, which is mutated whole: {said}"
    );
    assert!(
        gpu_only.ran.contains(&crate::gpu_mutants_command().line())
            && !gpu_only
                .ran
                .contains(&changes_mutants_command(FAKE_JOBS).line()),
        "the GPU workspace's run, and none of the root's: {:?}",
        gpu_only.ran
    );
    let mut both = changed_lines_fake();
    let _changes = both.files.insert(
        format!("capture {}", CHANGES.0),
        format!(
            "{}{}",
            one_line_change("crates/audio-core/src/lib.rs"),
            one_line_change("crates/visual-gpu/src/lib.rs")
        ),
    );
    let _source = both.files.insert(
        "crates/audio-core/src/lib.rs".to_owned(),
        "fn a() {}\n".to_owned(),
    );
    assert_eq!(
        run_steps(&changed_lines_steps(), &mut both, &mut Vec::new()),
        Ok(())
    );
    assert_eq!(
        both.files.get(CHANGES_DIFF).cloned(),
        Some(one_line_change("crates/audio-core/src/lib.rs")),
        "the root's lines alone in the root's diff"
    );
    let root = both
        .ran
        .iter()
        .position(|line| *line == changes_mutants_command(FAKE_JOBS).line());
    let gpu = both
        .ran
        .iter()
        .position(|line| *line == crate::gpu_mutants_command().line());
    assert!(
        matches!((root, gpu), (Some(root), Some(gpu)) if root < gpu),
        "the root's lines, then the GPU workspace: {:?}",
        both.ran
    );
}

#[test]
fn every_mutants_target_reaching_another_workspace_reviews_its_unviable_mutants() {
    // The law over every target of `cargo xtask mutants` that reaches a crate outside the
    // root's workspace (the crate, one of its files, its lines changed): that workspace's
    // run reports one unviable mutant, which the target's review gate must then read, from
    // the root, and refuse as unreviewed. The root's cargo-mutants runs nothing and writes
    // no report. `mutants --changed` once ran the GPU workspace without that merge.
    let others: Vec<RustCrate> = crate::RUST_CRATES
        .into_iter()
        .filter(|krate| krate.workspace != crate::CargoWorkspace::Root)
        .collect();
    assert!(!others.is_empty(), "the GPU workspace at least");
    for krate in others {
        let source = format!("{}/src/lib.rs", krate.dir);
        for target in [
            Target::Crate(krate),
            Target::RustFile {
                krate,
                path: source.clone(),
            },
            Target::ChangedLines(fake_revision()),
        ] {
            let mut fake = changed_lines_fake();
            let _change = fake
                .files
                .insert(format!("capture {}", CHANGES.0), one_line_change(&source));
            let _report = fake.files.insert(
                format!("{}/unviable.txt", krate.workspace.mutants_report()),
                "src/lib.rs:1:1: replace f -> T with Default::default()\n".to_owned(),
            );
            let _none = fake.files.remove(UNVIABLE_REPORT);
            let refused = run_steps(
                &mutants_steps(&target, FAKE_JOBS),
                &mut fake,
                &mut Vec::new(),
            );
            assert_eq!(
                fake.files.get(UNVIABLE_REPORT).cloned(),
                Some(format!(
                    "{}/src/lib.rs:1:1: replace f -> T with Default::default()\n",
                    krate.dir
                )),
                "{target:?}: its workspace's unviable mutants in the report, from the root"
            );
            assert_eq!(
                refused,
                Err(crate::Failure::from(format!(
                    "Rust unviable mutants reviewed: unviable mutants must be reviewed in \
                     mutants-unviable.txt: not reviewed: {source}: replace f -> T with \
                     Default::default()"
                ))),
                "{target:?}: the review gate refuses it"
            );
        }
    }
}

#[test]
fn mutates_the_gpu_workspace_whole_when_the_changes_reach_it() {
    let mut fake = passing();
    let _gpu_change = fake.files.insert(
        format!("capture {}", CHANGES.0),
        one_line_change("crates/visual-gpu/src/lib.rs"),
    );
    let _gpu_report = fake.files.insert(
        format!("{}/mutants.out/unviable.txt", crate::GPU_MUTANTS_OUT),
        "src/lib.rs:1:1: replace f -> T with Default::default()\n".to_owned(),
    );
    // The root's run tests no mutant here, so it writes no report.
    let _none = fake.files.remove(UNVIABLE_REPORT);
    let mut out = Vec::new();
    let mutation: Vec<Step> = rust_mutation_of_changes().into_iter().take(1).collect();
    assert_eq!(run_steps(&mutation, &mut fake, &mut out), Ok(()), "mutated");
    let said = String::from_utf8(out).expect("UTF-8");
    assert!(
        said.contains("  no Rust file changed: no mutant to test\n  crates/visual-gpu reached: every mutant of the GPU workspace\n"),
        "the root's diff leaves the GPU workspace out; that workspace is tested whole: {said}"
    );
    assert!(
        fake.ran.contains(&crate::gpu_mutants_command().line())
            && !fake.ran.iter().any(|line| line.contains("--in-diff")),
        "the GPU workspace's run, and none of the root's: {:?}",
        fake.ran
    );
    assert_eq!(
        fake.files.get(UNVIABLE_REPORT).map(String::as_str),
        Some("crates/visual-gpu/src/lib.rs:1:1: replace f -> T with Default::default()\n"),
        "its unviable mutants in the report it creates, from the root"
    );
    // Unchanged, it is not tested.
    let mut root_only = passing();
    let mut quiet = Vec::new();
    assert_eq!(run_steps(&mutation, &mut root_only, &mut quiet), Ok(()));
    assert!(
        !root_only.ran.contains(&crate::gpu_mutants_command().line()),
        "a change of the root's crates only leaves it untested"
    );
}

#[test]
fn every_mutation_run_of_the_changes_is_followed_by_its_doc_test_pass() {
    let name = "crates/audio-core/src/block.rs:61:5: replace engine_block with ()";
    // A run that misses `name`, then the doc-test pass that catches it.
    let check = |mut fake: Fake, steps: &[Step], first: &crate::Cmd, report: &str, what: &str| {
        fake.refuse = Some(first.line());
        let _missed = fake
            .files
            .insert(format!("{report}/missed.txt"), format!("{name}\n"));
        let _caught = fake.files.insert(
            format!("{report}/doc-tests/mutants.out/caught.txt"),
            format!("{name}\n"),
        );
        let mut out = Vec::new();
        assert_eq!(run_steps(steps, &mut fake, &mut out), Ok(()), "{what}");
        let pass = crate::steps::doc_test_pass_command(first, report, &[name]);
        assert!(
            fake.ran.contains(&pass.line()),
            "{what}: the doc-test pass ran: {:?}",
            fake.ran
        );
        let said = String::from_utf8(out).expect("UTF-8");
        assert!(
            said.contains(&format!(
                "    {name}\n  caught with the doc tests: 1 mutant\n"
            )),
            "{what}: said: {said}"
        );
    };
    let root = crate::MUTANTS_OUT;
    check(
        passing(),
        &rust_mutation_of_changes(),
        &changes_mutants_command(FAKE_JOBS),
        root,
        "the changes",
    );
    let mut everything = passing();
    let _lock = everything.files.insert(
        format!("capture {}", CHANGES.0),
        one_line_change("Cargo.lock"),
    );
    check(
        everything,
        &rust_mutation_of_changes(),
        &crate::steps::tested_mutants_command(&["--workspace"], FAKE_JOBS),
        root,
        "every mutant of the workspace",
    );
    let mut gpu = passing();
    let _gpu_change = gpu.files.insert(
        format!("capture {}", CHANGES.0),
        one_line_change("crates/visual-gpu/src/lib.rs"),
    );
    let _gpu_unviable = gpu.files.insert(
        format!("{}/mutants.out/unviable.txt", crate::GPU_MUTANTS_OUT),
        String::new(),
    );
    check(
        gpu,
        &rust_mutation_of_changes(),
        &crate::gpu_mutants_command(),
        crate::GPU_MUTANTS_REPORT,
        "the GPU workspace",
    );
    let mut lines = changed_lines_fake();
    let _changes = lines.files.insert(
        format!("capture {}", CHANGES.0),
        one_line_change("crates/audio-core/src/lib.rs"),
    );
    let _source = lines.files.insert(
        "crates/audio-core/src/lib.rs".to_owned(),
        "fn a() {}\n".to_owned(),
    );
    check(
        lines,
        &changed_lines_steps(),
        &changes_mutants_command(FAKE_JOBS),
        root,
        "the lines changed",
    );
}

/// The parts of the native check `paths` select, changed, each with the path that selects
/// it, when `reach` is what they widen to; the crates as [`crate::tests::NATIVE_METADATA`]
/// and the GPU workspace's build them.
fn native_selection(
    paths: &[&str],
    reach: &crate::changes::Reach,
) -> Vec<(crate::NativePart, String)> {
    let metadata = crate::merged_metadata(
        crate::tests::NATIVE_METADATA,
        &[crate::tests::GPU_METADATA.to_owned()],
    )
    .expect("merged");
    let closures = crate::changes::crate_closures(&metadata).expect("valid metadata");
    let changed: Vec<String> = paths.iter().map(|&path| path.to_owned()).collect();
    crate::NativePart::ALL
        .into_iter()
        .filter_map(|part| {
            crate::changes::native_reached(part, &changed, reach, &closures)
                .map(|(path, _why)| (part, path))
        })
        .collect()
}

#[test]
fn selects_each_native_part_by_the_inputs_of_the_crates_it_runs() {
    use crate::NativePart::{Audio, Gpu, Tests, Visual};
    let none = crate::changes::Reach::Crates(alloc::collections::BTreeMap::new());
    let at = |part, path: &str| (part, path.to_owned());
    for (paths, parts, why) in [
        (
            &["crates/audio-core/src/lib.rs"][..],
            vec![
                at(Tests, "crates/audio-core/src/lib.rs"),
                at(Audio, "crates/audio-core/src/lib.rs"),
            ],
            "a crate the native shell builds",
        ),
        (
            &["crates/fpenv/src/word.rs"],
            vec![
                at(Tests, "crates/fpenv/src/word.rs"),
                at(Audio, "crates/fpenv/src/word.rs"),
            ],
            "the floating-point mode itself",
        ),
        (
            &["conformance/references.json"],
            vec![
                at(Tests, "conformance/references.json"),
                at(Audio, "conformance/references.json"),
            ],
            "the native shell's test data",
        ),
        (
            &["crates/visual-cli/src/main.rs"],
            vec![
                at(Tests, "crates/visual-cli/src/main.rs"),
                at(Visual, "crates/visual-cli/src/main.rs"),
            ],
            "the visual tool alone",
        ),
        (
            &["conformance/visual/cases.json"],
            vec![
                at(Tests, "conformance/visual/cases.json"),
                at(Audio, "conformance/visual/cases.json"),
                at(Visual, "conformance/visual/cases.json"),
            ],
            "the visual corpus, below the audio shell's test data too",
        ),
        (
            &["crates/visual-core/src/image.rs"],
            vec![
                at(Tests, "crates/visual-core/src/image.rs"),
                at(Visual, "crates/visual-core/src/image.rs"),
                at(Gpu, "crates/visual-core/src/image.rs"),
            ],
            "the visual core, which the GPU workspace builds too",
        ),
        (
            &["crates/visual-gpu/Cargo.lock"],
            vec![at(Gpu, "crates/visual-gpu/Cargo.lock")],
            "the GPU workspace's own lock file",
        ),
        (
            &["spec/gpu-time-baseline.txt"],
            vec![at(Gpu, "spec/gpu-time-baseline.txt")],
            "the GPU's time baseline",
        ),
        (&["apps/roadmap/board.json"], vec![], "a file no part reads"),
        (
            &["crates/coordinator/src/lib.rs"],
            vec![at(Tests, "crates/coordinator/src/lib.rs")],
            "a crate of the root workspace no corpus's tool builds: its tests alone",
        ),
        (
            &[
                "README.md",
                "crates/visual-gpu/src/lib.rs",
                "crates/fpenv/src/lib.rs",
            ],
            vec![
                at(Tests, "crates/fpenv/src/lib.rs"),
                at(Audio, "crates/fpenv/src/lib.rs"),
                at(Gpu, "crates/visual-gpu/src/lib.rs"),
            ],
            "each part by the first path that selects it",
        ),
    ] {
        assert_eq!(native_selection(paths, &none), parts, "{why}");
    }
}

#[test]
fn selects_every_native_part_a_shared_input_widens_to() {
    use crate::NativePart::{Audio, Gpu, Tests, Visual};
    let at = |part, path: &str| (part, path.to_owned());
    let everything = crate::changes::Reach::Everything {
        path: "rust-toolchain.toml".to_owned(),
        unnarrowed: None,
    };
    assert_eq!(
        native_selection(&["rust-toolchain.toml"], &everything),
        [Tests, Audio, Visual, Gpu].map(|part| at(part, "rust-toolchain.toml")),
        "what every crate depends on selects every part"
    );
    let widened = crate::changes::Reach::Crates(alloc::collections::BTreeMap::from([(
        "visual-cli",
        "Cargo.lock".to_owned(),
    )]));
    assert_eq!(
        native_selection(&["README.md", "Cargo.lock"], &widened),
        [at(Tests, "Cargo.lock"), at(Visual, "Cargo.lock")],
        "a change widened to one of the part's crates"
    );
    // A lock change narrowed to xtask and to a part's crate: every part, as what builds xtask
    // (a rule over the reason the part's own crates would give).
    let both = crate::changes::Reach::Crates(alloc::collections::BTreeMap::from([
        ("xtask", "Cargo.lock".to_owned()),
        ("visual-cli", "Cargo.lock".to_owned()),
    ]));
    let metadata = crate::merged_metadata(
        crate::tests::NATIVE_METADATA,
        &[crate::tests::GPU_METADATA.to_owned()],
    )
    .expect("merged");
    let closures = crate::changes::crate_closures(&metadata).expect("valid metadata");
    let changed = ["README.md".to_owned(), "Cargo.lock".to_owned()];
    for part in crate::NativePart::ALL {
        assert_eq!(
            crate::changes::native_reached(part, &changed, &both, &closures),
            Some(("Cargo.lock".to_owned(), ", what builds xtask".to_owned())),
            "{part:?}"
        );
    }
}

/// The lines `native_parts` says for the changes `paths` (through the fake's diff since
/// the merge base), with what it selects.
fn native_said(fake: &mut Fake) -> (Result<Vec<crate::NativePart>, String>, String) {
    let mut out = Vec::new();
    let parts = crate::changes::native_parts(fake, &mut out, NATIVE_COMMIT)
        .map_err(|failure| failure.summary);
    (parts, String::from_utf8(out).expect("UTF-8"))
}

/// The commit the native check is told it checks: unlike the merge base, so that a line
/// naming one for the other shows.
const NATIVE_COMMIT: &str = "c0ffee0c0ffee0c0ffee0c0ffee0c0ffee0c0ffe";

#[test]
fn says_which_native_parts_the_changes_since_main_reach() {
    use crate::NativePart::{Audio, Gpu, Tests, Visual};
    let base = MAIN_BASE.1.trim();
    let head =
        format!("  the changes of {NATIVE_COMMIT} since {base} (merge base with origin/main)\n");
    // Why a lock that cannot be narrowed selects every part, as the default check says it.
    let unreadable_lock: String = ["Rust tests", "audio", "visual", "GPU"]
        .map(|part| {
            format!(
                "  Cargo.lock changed and cannot be narrowed (cannot read Cargo.lock at the merge \
                 base {base}: `git ls-tree --name-only {base} -- Cargo.lock` exited with code 1: \
                 failed), what every test depends on: the {part} part runs\n"
            )
        })
        .concat();
    for (paths, parts, lines, what) in [
        (
            vec!["crates/visual-gpu/src/lib.rs", "crates/fpenv/src/new.rs"],
            vec![Tests, Audio, Gpu],
            "  crates/fpenv/src/new.rs changed: the Rust tests part runs\n  \
             crates/fpenv/src/new.rs changed: the audio part runs\n  \
             no change reaches the visual part: not run\n  \
             crates/visual-gpu/src/lib.rs changed: the GPU part runs\n",
            "the crates of two parts",
        ),
        (
            vec![],
            vec![],
            "  no change reaches the Rust tests part: not run\n  \
             no change reaches the audio part: not run\n  \
             no change reaches the visual part: not run\n  \
             no change reaches the GPU part: not run\n",
            "no change",
        ),
        (
            vec!["README.md", "crates/xtask/src/actions.rs"],
            vec![Tests, Audio, Visual, Gpu],
            "  crates/xtask/src/actions.rs changed, the mutation rules: the Rust tests part runs\n  \
             crates/xtask/src/actions.rs changed, the mutation rules: the audio part runs\n  \
             crates/xtask/src/actions.rs changed, the mutation rules: the visual part runs\n  \
             crates/xtask/src/actions.rs changed, the mutation rules: the GPU part runs\n",
            "what decides xtask's checks, and why",
        ),
        (
            vec!["crates/xtask/Cargo.toml"],
            vec![Tests, Audio, Visual, Gpu],
            "  crates/xtask/Cargo.toml changed, what builds xtask: the Rust tests part runs\n  \
             crates/xtask/Cargo.toml changed, what builds xtask: the audio part runs\n  \
             crates/xtask/Cargo.toml changed, what builds xtask: the visual part runs\n  \
             crates/xtask/Cargo.toml changed, what builds xtask: the GPU part runs\n",
            "xtask's build",
        ),
        (
            vec!["Cargo.lock"],
            vec![Tests, Audio, Visual, Gpu],
            &unreadable_lock,
            "a lock whose base cannot be read",
        ),
        (
            // Through `reach`, which widens what every test depends on to every crate.
            vec!["rust-toolchain.toml"],
            vec![Tests, Audio, Visual, Gpu],
            "  rust-toolchain.toml changed, what every test depends on: the Rust tests part runs\n  \
             rust-toolchain.toml changed, what every test depends on: the audio part runs\n  \
             rust-toolchain.toml changed, what every test depends on: the visual part runs\n  \
             rust-toolchain.toml changed, what every test depends on: the GPU part runs\n",
            "the toolchain",
        ),
    ] {
        let (selected, said) = native_said(&mut crate::tests::native_fake(&paths));
        assert_eq!(selected, Ok(parts), "{what}");
        assert_eq!(said, format!("{head}{lines}"), "{what}: each part said");
    }
}

#[test]
fn stops_at_the_first_native_line_it_cannot_write() {
    // The base and the audio part's line said, then the next fails: a part that runs, then
    // one that does not.
    for (lines, what) in [(1, "the audio part's line"), (2, "the visual part's line")] {
        let mut fake = crate::tests::native_fake(&["crates/fpenv/src/lib.rs"]);
        assert_eq!(
            crate::changes::native_parts(
                &mut fake,
                &mut crate::tests::Limited(lines),
                NATIVE_COMMIT
            )
            .map_err(|failure| failure.summary),
            Err("cannot write to stdout: closed".to_owned()),
            "{what}"
        );
    }
}

#[test]
fn says_a_changed_path_with_control_characters_escaped() {
    let base = MAIN_BASE.1.trim();
    let mut fake = crate::tests::native_fake(&[]);
    let _scripted = fake.files.insert(
        format!("capture git diff --no-color --name-only -z --no-renames {base}"),
        "crates/fpenv/src/a\u{1b}b\nc\u{9b}d.rs\0".to_owned(),
    );
    let (selected, said) = native_said(&mut fake);
    assert_eq!(
        selected,
        Ok(vec![crate::NativePart::Tests, crate::NativePart::Audio]),
        "selected"
    );
    assert_eq!(
        said.lines().nth(1),
        Some(r"  crates/fpenv/src/a\u{1b}b\u{a}c\u{9b}d.rs changed: the Rust tests part runs"),
        "the path's escape, line break and C1 control neutralized: {said}"
    );
}

#[test]
fn says_why_it_cannot_select_the_native_parts() {
    let base = MAIN_BASE.1.trim();
    let metadata = format!(
        "capture {}",
        crate::metadata_command(crate::CargoWorkspace::Root).line()
    );
    for (missing, reason) in [
        (
            format!("capture {}", MAIN_BASE.0),
            "no merge base with origin/main or main to read the changes of the native check from (fetch or create one of them)".to_owned(),
        ),
        (
            metadata.clone(),
            format!(
                "cannot list the crates: `{}` exited with code 1: failed",
                metadata.trim_start_matches("capture ")
            ),
        ),
    ] {
        let mut fake = crate::tests::native_fake(&["crates/fpenv/src/lib.rs"]);
        let _removed = fake.files.remove(&missing);
        assert_eq!(
            crate::changes::native_parts(&mut fake, &mut Vec::new(), NATIVE_COMMIT)
                .map_err(|failure| failure.summary),
            Err(reason),
            "{missing} ({base})"
        );
    }
    // The changes, the GPU workspace's crates: a command that cannot start.
    let changes = format!("git diff --no-color --name-only -z --no-renames {base}");
    let gpu = crate::metadata_command(crate::CargoWorkspace::Gpu).line();
    for (broken, what) in [(changes, "list the changes"), (gpu, "list the crates")] {
        let mut fake = crate::tests::native_fake(&["crates/fpenv/src/lib.rs"]);
        fake.broken = Some(broken.clone());
        let failed = crate::changes::native_parts(&mut fake, &mut Vec::new(), NATIVE_COMMIT)
            .map_err(|failure| failure.summary);
        assert_eq!(
            failed,
            Err(format!("cannot {what}: cannot run `{broken}`: not found")),
            "{broken}"
        );
    }
    assert_eq!(
        crate::changes::native_parts(
            &mut crate::tests::native_fake(&[]),
            &mut crate::tests::Broken,
            NATIVE_COMMIT
        )
        .map_err(|failure| failure.summary),
        Err("cannot write to stdout: closed".to_owned()),
        "stdout broken"
    );
}

#[test]
fn selects_every_native_part_when_what_decides_the_checks_changes() {
    use crate::NativePart::{Audio, Gpu, Tests, Visual};
    let none = crate::changes::Reach::Crates(alloc::collections::BTreeMap::new());
    // Every file of xtask's own code, the reviews of unviable mutants the GPU part's gate
    // reads, and the Cargo configuration: the rule of the mutation proof record.
    for source in [
        "crates/xtask/src/steps.rs",
        "crates/xtask/src/changes.rs",
        "crates/xtask/src/cli.rs",
        "crates/xtask/src/actions.rs",
        "crates/xtask/src/workspace.rs",
        "mutants-unviable.txt",
        ".cargo/config.toml",
    ] {
        assert_eq!(
            native_selection(&["README.md", source], &none),
            [Tests, Audio, Visual, Gpu].map(|part| (part, source.to_owned())),
            "{source}"
        );
    }
    assert_eq!(
        native_selection(&["crates/xtask/src/steps_tests.rs", "README.md"], &none),
        [(Tests, "crates/xtask/src/steps_tests.rs".to_owned())],
        "a test file of xtask, a crate of the root workspace: its tests alone; and a file \
         no part reads"
    );
}

#[test]
fn reads_a_section_path_and_a_lock_value_only_where_written_so() {
    use crate::changes::{lock_value, section_path};
    assert_eq!(
        section_path("a/x/y b/x/y"),
        Some("x/y"),
        "a path, the same twice"
    );
    // Its half falls inside the `€`: no path is cut there.
    assert_eq!(
        section_path("a/\u{20ac} b/xx"),
        None,
        "split inside a character"
    );
    assert_eq!(section_path("a/x b/y"), None, "two paths");
    assert_eq!(
        lock_value("name = \"x\"", "name"),
        Some("x"),
        "the key's value"
    );
    assert_eq!(lock_value("version = \"1\"", "name"), None, "another key");
}

/// A scenario of the harness: its name, the steps it runs and the passing fake it runs them on.
type Scenario<'a> = (&'a str, &'a [Step], &'a dyn Fn() -> Fake);

#[test]
fn stops_each_mutation_of_the_changes_at_whichever_call_fails() {
    // The harness of the whole checks, over the paths of the mutation of the changes that
    // only these scenarios reach: a valid proof record, one a dependency's change undoes,
    // the GPU workspace reached, the lines changed alone, and the GPU's lines among them.
    let plan = one_line_change("crates/audio-core/src/plan.rs");
    let proven = || proven_fake(&proof_record(PROVEN[2]), &plan);
    let linked = || {
        let mut fake = proven();
        drop(fake.files.insert(
            format!(
                "capture {}",
                crate::metadata_command(crate::CargoWorkspace::Root).line()
            ),
            closure_metadata(true),
        ));
        fake
    };
    let gpu_change = |mut fake: Fake| {
        let _gpu_change = fake.files.insert(
            format!("capture {}", CHANGES.0),
            one_line_change("crates/visual-gpu/src/lib.rs"),
        );
        fake
    };
    let gpu_reached = || {
        let mut fake = gpu_change(passing());
        let _gpu_report = fake.files.insert(
            format!("{}/mutants.out/unviable.txt", crate::GPU_MUTANTS_OUT),
            "src/lib.rs:1:1: replace f -> T with Default::default()\n".to_owned(),
        );
        let _none = fake.files.remove(UNVIABLE_REPORT);
        fake
    };
    let gpu_lines = || gpu_change(changed_lines_fake());
    let mutation = rust_mutation_of_changes();
    let first: Vec<Step> = mutation.iter().take(1).cloned().collect();
    let lines = changed_lines_steps();
    let new_source = || {
        let mut fake = passing();
        let _changes = fake
            .files
            .insert(format!("capture {}", CHANGES.0), String::new());
        let _listed = fake.files.insert(
            format!("capture {}", NEW_RUST_FILES.0),
            "crates/audio-core/src/plan.rs\0crates/audio-core/src/plan_tests.rs\0".to_owned(),
        );
        for (path, text) in [
            ("crates/audio-core/src/plan.rs", declares("plan_tests.rs")),
            (
                "crates/audio-core/src/plan_tests.rs",
                "fn t() {}\n".to_owned(),
            ),
        ] {
            let _source = fake.files.insert(path.to_owned(), text);
        }
        fake
    };
    let other_base = || {
        let mut record = proof_record(PROVEN[2]);
        record.base = "4".repeat(40);
        proven_fake(&record, "")
    };
    let rules_changed = || {
        proven_fake(
            &proof_record(PROVEN[2]),
            &one_line_change("crates/xtask/src/lib.rs"),
        )
    };
    // Git's common directory not found: no record is read, and the check says so.
    let no_record = || Fake {
        fail: Some("git rev-parse --git-common-dir".to_owned()),
        ..passing()
    };
    let scenarios: [Scenario<'_>; 10] = [
        ("a new source tested whole", &mutation, &new_source),
        ("a record of another base", &mutation, &other_base),
        ("a record whose rules changed", &mutation, &rules_changed),
        ("no record to read", &mutation, &no_record),
        ("a valid proof record", &mutation, &proven),
        ("a record a dependency's change undoes", &mutation, &linked),
        ("the GPU workspace reached", &first, &gpu_reached),
        ("the lines changed", &lines, &changed_lines_fake),
        ("the GPU's lines changed", &lines, &gpu_lines),
        ("the changes", &mutation, &passing),
    ];
    let unnamed: Vec<String> = scenarios
        .iter()
        .flat_map(|(label, steps, fake)| crate::tests::unrefused(label, steps, *fake))
        .collect();
    assert_eq!(
        unnamed,
        Vec::<String>::new(),
        "each broken call refused or said"
    );
}

#[test]
fn stops_selecting_the_native_parts_at_whichever_call_fails() {
    // The harness of the whole checks over the selection of the native parts: changes
    // reaching parts, and changes reaching none.
    let select = |runner: &mut Fake, out: &mut dyn std::io::Write| {
        crate::changes::native_parts(runner, out, NATIVE_COMMIT).map(drop)
    };
    let unnamed: Vec<String> = [
        &[
            "crates/audio-core/src/lib.rs",
            "crates/visual-gpu/src/lib.rs",
        ][..],
        &["README.md"],
    ]
    .iter()
    .flat_map(|paths| {
        crate::tests::unrefused_by(&format!("{paths:?}"), &select, &|| {
            crate::tests::native_fake(paths)
        })
    })
    .collect();
    assert_eq!(
        unnamed,
        Vec::<String>::new(),
        "each broken call refused or said"
    );
    // Metadata whose crate lists a dependency without a name: the selection refuses it.
    let mut unnamed_dependency = crate::tests::native_fake(&["README.md"]);
    let _metadata = unnamed_dependency.files.insert(
        format!(
            "capture {}",
            crate::metadata_command(crate::CargoWorkspace::Root).line()
        ),
        r#"{"workspace_root":"/r","packages":[{"name":"audio-core","dependencies":[{}]}]}"#
            .to_owned(),
    );
    assert_eq!(
        native_said(&mut unnamed_dependency).0,
        Err("cargo metadata lists a dependency without a name".to_owned()),
        "a dependency without a name"
    );
}

#[test]
fn stops_each_widened_mutation_at_whichever_call_fails() {
    // The harness of the whole checks over the changes that widen the mutation: a unit test
    // no source declares, and a shared input.
    let changed = |paths: &[&str]| {
        let mut fake = passing();
        let _changes = fake.files.insert(
            format!("capture {}", CHANGES.0),
            paths.iter().map(|path| one_line_change(path)).collect(),
        );
        for path in [
            "crates/audio-core/src/lib.rs",
            "crates/audio-core/src/orphan_tests.rs",
        ] {
            let _source = fake.files.insert(path.to_owned(), "fn a() {}\n".to_owned());
        }
        fake
    };
    // A unit test no source declares: its crate is mutated whole, and said so.
    let orphan = || changed(&["crates/audio-core/src/orphan_tests.rs"]);
    // A shared input changed: every Rust mutant, and said so.
    let shared = || changed(&["rust-toolchain.toml"]);
    let mutation = rust_mutation_of_changes();
    let mut unnamed = crate::tests::unrefused("a unit test no source declares", &mutation, &orphan);
    unnamed.extend(crate::tests::unrefused(
        "a shared input changed",
        &mutation,
        &shared,
    ));
    assert_eq!(
        unnamed,
        Vec::<String>::new(),
        "each broken call refused or said"
    );
}
