//! Tests of versions, the changelog and the release gates.

use alloc::collections::BTreeSet;

use crate::tests::{
    CARGO, CHANGELOG_LISTED, CHANGES_MODE, LOG, MAIN_BASE, PACKAGE, failed, found, hermetic_git,
    passing, scratch, system,
};
use crate::{
    Action, CARGO_TOML, CHANGELOG, Failure, Step, Version, check_steps, package_jsons,
    release_gate, run_steps,
};

/// The roadmap's manifest.
const ROADMAP_PACKAGE: &str = "apps/roadmap/package.json";

/// Every manifest correct except the roadmap one, with `text`.
fn roadmap_package(text: &str) -> Vec<(String, String)> {
    package_jsons()
        .into_iter()
        .map(|path| {
            let content = if path == ROADMAP_PACKAGE {
                text
            } else {
                PACKAGE
            };
            (path, content.to_owned())
        })
        .collect()
}

#[test]
fn parses_semantic_versions() {
    assert_eq!(
        Version::parse("0.1.0"),
        Some(Version {
            major: 0,
            minor: 1,
            patch: 0
        }),
        "basic"
    );
    assert_eq!(
        Version::parse("10.20.30"),
        Some(Version {
            major: 10,
            minor: 20,
            patch: 30
        }),
        "multi-digit"
    );
    for bad in [
        "",
        "1",
        "1.2",
        "1.2.3.4",
        "01.2.3",
        "1.02.3",
        "1.2.03",
        "1.2.x",
        "1..3",
        "-1.2.3",
        "1.2.3-rc.1",
        "+1.2.3",
    ] {
        assert_eq!(Version::parse(bad), None, "{bad:?} is not X.Y.Z");
    }
    assert_eq!(
        Version {
            major: 1,
            minor: 2,
            patch: 3
        }
        .to_string(),
        "1.2.3",
        "display"
    );
    assert!(
        Version::parse("0.10.0") > Version::parse("0.9.9"),
        "numeric ordering"
    );
}

#[test]
fn accepts_a_consistent_release_setup() {
    assert_eq!(
        release_gate(CARGO, &packages(PACKAGE), LOG),
        Ok(()),
        "current version is the latest release"
    );
    let next = CARGO.replace("0.2.0\"\n\n[other]", "0.3.0\"\n\n[other]");
    assert_eq!(
        release_gate(&next, &packages(&PACKAGE.replace("0.2.0", "0.3.0")), LOG),
        Ok(()),
        "a newer, unreleased version"
    );
    assert_eq!(
        release_gate(
            CARGO,
            &packages(PACKAGE),
            "# Changelog\n\n## [Unreleased]\n"
        ),
        Ok(()),
        "no release yet"
    );
}

#[test]
fn rejects_inconsistent_versions() {
    assert_eq!(
        release_gate(
            CARGO,
            &roadmap_package(r#"{"version": "0.3.0", "engines": {"node": ">=22.18"}}"#),
            LOG
        ),
        found(
            "the versions and the changelog",
            &[format!(
                "{ROADMAP_PACKAGE}: version 0.3.0 differs from the workspace version 0.2.0"
            )]
        ),
        "different versions"
    );
    assert_eq!(
        release_gate(CARGO, &roadmap_package("not json"), LOG),
        found(
            "the versions and the changelog",
            &[
                format!(
                    "{ROADMAP_PACKAGE}: engines.node is (missing), the packages support Node.js \
                     >=22.18 (NODE_FLOOR)"
                ),
                format!(
                    "{ROADMAP_PACKAGE}: version (missing) differs from the workspace version 0.2.0"
                )
            ]
        ),
        "unreadable package"
    );
    assert_eq!(
        release_gate(
            CARGO,
            &packages(r#"{"version": "0.1.0", "engines": {"node": ">=22.18"}}"#),
            LOG
        ),
        found(
            "the versions and the changelog",
            &package_jsons()
                .iter()
                .map(|path| format!(
                    "{path}: version 0.1.0 differs from the workspace version 0.2.0"
                ))
                .collect::<Vec<_>>()
        ),
        "every manifest is checked"
    );
    assert_eq!(
        release_gate(
            "[workspace]\n",
            &packages(r#"{"version": 1, "engines": {"node": ">=22.18"}}"#),
            LOG
        ),
        found(
            "the versions and the changelog",
            &[format!(
                "{CARGO_TOML}: [workspace.package] version must be X.Y.Z"
            )]
        ),
        "no workspace version, and none in the package either"
    );
    let bad = CARGO.replace("0.2.0\"\n\n[other]", "0.2\"\n\n[other]");
    assert_eq!(
        release_gate(&bad, &packages(&PACKAGE.replace("0.2.0", "0.2")), LOG),
        found(
            "the versions and the changelog",
            &[format!(
                "{CARGO_TOML}: [workspace.package] version must be X.Y.Z"
            )]
        ),
        "not X.Y.Z"
    );
    let old = CARGO.replace("0.2.0\"\n\n[other]", "0.1.9\"\n\n[other]");
    assert_eq!(
        release_gate(&old, &packages(&PACKAGE.replace("0.2.0", "0.1.9")), LOG),
        found(
            "the versions and the changelog",
            &["version 0.1.9 is older than the latest release 0.2.0".to_owned()]
        ),
        "older than released"
    );
}

#[test]
fn rejects_a_malformed_changelog() {
    assert_eq!(
        release_gate(
            CARGO,
            &packages(PACKAGE),
            "Changelog\n## [0.2.0] - 2026-11-07\n"
        ),
        found(
            "the versions and the changelog",
            &[
                format!("{CHANGELOG}: must start with \"# Changelog\""),
                format!("{CHANGELOG}: the first section must be \"## [Unreleased]\"")
            ]
        ),
        "missing title and unreleased section"
    );
    for heading in [
        "## [0.2] - 2026-11-07",
        "## 0.2.0 - 2026-11-07",
        "## [0.2.0] 2026-11-07",
        "## [0.2.0] - 26-11-07",
        "## [0.2.0] - 2026-1-07",
        "## [0.2.0] - 2026-11-7",
        "## [0.2.0] - 2026-11-07-01",
        "## [0.2.0] - 2026-11",
        "## [0.2.0] - 2026-aa-07",
    ] {
        assert_eq!(
            release_gate(
                CARGO,
                &packages(PACKAGE),
                &format!("# Changelog\n## [Unreleased]\n{heading}\n")
            ),
            found(
                "the versions and the changelog",
                &[format!("{CHANGELOG}: malformed heading \"{heading}\"")]
            ),
            "{heading}"
        );
    }
    for order in [
        "## [0.1.0] - 2026-01-01\n## [0.2.0] - 2026-02-01",
        "## [0.2.0] - 2026-01-01\n## [0.2.0] - 2026-02-01",
    ] {
        assert_eq!(
            release_gate(
                CARGO,
                &packages(PACKAGE),
                &format!("# Changelog\n## [Unreleased]\n{order}\n")
            ),
            found(
                "the versions and the changelog",
                &[format!(
                    "{CHANGELOG}: releases must be in strictly decreasing order"
                )]
            ),
            "{order}"
        );
    }
}

#[test]
fn requires_every_package_to_support_the_node_floor() {
    for engines in [
        r#""engines": {"node": ">=22.17"}"#,
        r#""engines": {"node": ">=22.18.1"}"#,
        r#""engines": {}"#,
    ] {
        let manifest = format!(r#"{{"version": "0.2.0", {engines}}}"#);
        assert!(
            release_gate(CARGO, &roadmap_package(&manifest), LOG).is_err_and(|failure| failure
                .summary
                .contains(&format!("{ROADMAP_PACKAGE}: engines.node is "))),
            "{engines}"
        );
    }
    assert_eq!(
        release_gate(CARGO, &roadmap_package(r#"{"version": "0.2.0"}"#), LOG),
        found(
            "the versions and the changelog",
            &[format!(
                "{ROADMAP_PACKAGE}: engines.node is (missing), the packages support Node.js >=22.18 (NODE_FLOOR)"
            )]
        ),
        "no engines"
    );
}

/// A changelog with two released sections, as the merge base holds it.
const RELEASED: &str = "# Changelog\n\n## [Unreleased]\n\n- next.\n\n## [0.2.0] - 2026-11-09\n\n\
                        - two.\n\n## [0.1.0] - 2026-11-08\n\n- one.\n";

#[test]
fn keeps_every_released_section_byte_for_byte() {
    let at = ("fed4560", "origin/main");
    let base = "fed4560 (merge base with origin/main)";
    let restore = "restore it from `git show fed4560:CHANGELOG.md`";
    let context = "the released changelog sections";
    for (changelog, why) in [
        (RELEASED.to_owned(), "unchanged"),
        (
            RELEASED.replace("- next.", "- next, reworded."),
            "the unreleased section edited",
        ),
        (
            RELEASED.replace(
                "## [0.2.0]",
                "## [0.3.0] - 2026-11-10\n\n- three.\n\n## [0.2.0]",
            ),
            "a version added",
        ),
    ] {
        assert_eq!(
            crate::released_sections_gate(Some(RELEASED), &changelog, at),
            Ok(()),
            "{why}"
        );
    }
    assert_eq!(
        crate::released_sections_gate(None, "anything\n", at),
        Ok(()),
        "no changelog at the base: nothing released"
    );
    assert_eq!(
        crate::released_sections_gate(Some(RELEASED), &RELEASED.replace("- one.", "- One."), at),
        failed(format!(
            "{context}: CHANGELOG.md:13: the released section [0.1.0] differs from the one at {base}: \
             a released section is never edited: {restore}"
        )),
        "a released section edited, its first differing line named"
    );
    for (changelog, why) in [
        (
            RELEASED.replace("- one.\n", "- one.\n\n"),
            "a line added at its end",
        ),
        (
            RELEASED.replace("- one.\n", "- one."),
            "its last line break removed",
        ),
        (
            RELEASED.replace("- one.\n", "- one.\r\n"),
            "a line ending changed",
        ),
    ] {
        assert!(
            crate::released_sections_gate(Some(RELEASED), &changelog, at).is_err_and(|failure| {
                failure
                    .summary
                    .contains("the released section [0.1.0] differs")
            }),
            "{why}"
        );
    }
    assert_eq!(
        crate::released_sections_gate(
            Some(RELEASED),
            &RELEASED
                .replace("2026-11-09", "2026-11-10")
                .replace("- one.\n", ""),
            at
        ),
        failed(format!(
            "{context}: 2 problems, listed above\n\
             CHANGELOG.md:7: the released section [0.2.0] differs from the one at {base}: a released \
             section is never edited: {restore}\n\
             CHANGELOG.md:13: the released section [0.1.0] differs from the one at {base}: a released \
             section is never edited: {restore}"
        )),
        "a heading's date changed, a line removed: each section named"
    );
    assert_eq!(
        crate::released_sections_gate(
            Some(RELEASED),
            &RELEASED.replace("## [0.1.0] - 2026-11-08\n\n- one.\n", ""),
            at
        ),
        failed(format!(
            "{context}: CHANGELOG.md: the released section [0.1.0] of {base} is missing: {restore}"
        )),
        "a released section removed"
    );
}

#[test]
fn reads_the_changelog_real_git_holds_at_the_merge_base_with_main() {
    let dir = scratch("released-sections");
    let _init = hermetic_git(&dir, &["init", "--quiet", "--initial-branch=main"]);
    std::fs::write(dir.join("README.md"), "x\n").expect("a file");
    let _add = hermetic_git(&dir, &["add", "."]);
    let _first = hermetic_git(&dir, &["commit", "--quiet", "-m", "without"]);
    let without = hermetic_git(&dir, &["rev-parse", "HEAD"]);
    std::fs::write(dir.join("CHANGELOG.md"), RELEASED).expect("a changelog");
    let _add_log = hermetic_git(&dir, &["add", "."]);
    let _second = hermetic_git(&dir, &["commit", "--quiet", "-m", "with"]);
    let with = hermetic_git(&dir, &["rev-parse", "HEAD"]);
    let mut runner = system(dir.path());
    assert_eq!(
        crate::file_at(&mut runner, without.trim(), "CHANGELOG.md")
            .map_err(|failure| failure.summary),
        Ok(None),
        "none at a commit without it"
    );
    assert_eq!(
        crate::file_at(&mut runner, with.trim(), "CHANGELOG.md").map_err(|failure| failure.summary),
        Ok(Some(RELEASED.to_owned())),
        "its bytes at a commit with it"
    );
    let _branch = hermetic_git(&dir, &["checkout", "--quiet", "-b", "work"]);
    assert_eq!(
        crate::release::released_sections(&mut runner, &RELEASED.replace("- one.", "- One."))
            .map_err(|failure| failure.summary),
        Err(format!(
            "the released changelog sections: CHANGELOG.md:13: the released section [0.1.0] \
             differs from the one at {with} (merge base with main): a released section is never \
             edited: restore it from `git show {with}:CHANGELOG.md`",
            with = with.trim()
        )),
        "a changelog against main's at the merge base"
    );
    assert_eq!(
        hermetic_git(&dir, &["show", &format!("{}:CHANGELOG.md", with.trim())]),
        RELEASED,
        "the command given shows the sections as released"
    );
    assert_eq!(
        crate::release::released_sections(&mut runner, RELEASED).map_err(|failure| failure.summary),
        Ok(()),
        "as released"
    );
}

#[test]
fn reads_the_released_sections_against_main() {
    let step: Vec<Step> = check_steps(CHANGES_MODE)
        .into_iter()
        .filter(|step| step.action == Action::VersionGate)
        .collect();
    let mut fake = passing();
    let listed = fake.files.insert(
        format!("capture {}", CHANGELOG_LISTED.0),
        "CHANGELOG.md\n".to_owned(),
    );
    assert!(listed.is_some(), "the changelog listed at the base");
    let shown = fake.files.insert(
        "capture git show fed4560000000000000000000000000000000000:CHANGELOG.md".to_owned(),
        LOG.replace("2026", "2025"),
    );
    assert!(shown.is_none(), "fresh fake");
    let base = "fed4560000000000000000000000000000000000 (merge base with origin/main)";
    assert_eq!(
        run_steps(&step, &mut fake, &mut Vec::new()),
        Err(Failure {
            summary: "One version everywhere, changelog well formed, released sections as on \
                      main: the released changelog sections: 3 problems, listed above"
                .to_owned(),
            details: [(11_u8, "0.2.0"), (13, "0.1.10"), (15, "0.1.2")]
                .map(|(line, version)| {
                    format!(
                        "CHANGELOG.md:{line}: the released section [{version}] differs from the one \
                         at {base}: a released section is never edited: restore it from `git show \
                         fed4560000000000000000000000000000000000:CHANGELOG.md`"
                    )
                })
                .to_vec(),
            usage: false,
        }),
        "the changelog against the one at the merge base with main"
    );
    let mut baseless = passing();
    let _main = baseless.files.remove(&format!("capture {}", MAIN_BASE.0));
    let _local = baseless.files.remove("capture git merge-base HEAD main");
    assert!(
        run_steps(&step, &mut baseless, &mut Vec::new()).is_err_and(|failure| failure
            .summary
            .starts_with(
                "One version everywhere, changelog well formed, released sections as on main: \
                 no merge base with origin/main or main to read the released changelog \
                 sections from (fetch or create one of them)"
            )),
        "no base"
    );
    let mut unreadable = passing();
    unreadable.broken = Some(CHANGELOG_LISTED.0.to_owned());
    assert!(
        run_steps(&step, &mut unreadable, &mut Vec::new()).is_err_and(|failure| failure
            .summary
            .contains("cannot read CHANGELOG.md at the merge base fed4560")),
        "git cannot list it"
    );
}

/// A changelog whose `[Unreleased]` holds `entries`, above a released breaking entry.
fn unreleased_with(entries: &str) -> String {
    format!(
        "# Changelog\n\n## [Unreleased]\n\n{entries}\n## [0.1.0] - 2026-11-08\n\n\
         - **Breaking:** `Released` changed.\n"
    )
}

#[test]
fn reads_the_names_each_breaking_entry_cites() {
    let changelog = unreleased_with(
        "- **Breaking:** `audio_wasm::Engine` is `audio_host::Engine<N>`, `Block<S, N>` \
         takes it, `Create::create` (`create<S, N>`) too; ABI `0x5059_0005`, `{ a, b? }`, \
         `audio_host::new_name`.\n\
         - `NotBreaking` changed.\n  - **Breaking** inside an entry.\n\
         - **Breaking (names):** `x.y` and `--flag`.\n- **Breaking:** none cited.\n",
    );
    assert_eq!(
        crate::release::breaking_entries(&changelog),
        vec![
            (
                5,
                [
                    "Engine", "Engine", "Block", "create", "create", "b", "new_name"
                ]
                .map(str::to_owned)
                .to_vec()
            ),
            (8, vec!["y".to_owned(), "flag".to_owned()]),
            (9, vec![]),
        ],
        "the last identifier of each code span before its generics or arguments, a number \
         none; entries of [Unreleased] only, each with its line"
    );
}

#[test]
fn refuses_a_breaking_entry_citing_nothing_the_last_version_has() {
    let at = ("fed4560", "origin/main");
    let held: BTreeSet<String> = ["Engine", "Changelog"].map(str::to_owned).into();
    let refused = |line: u8, names: &str| {
        format!(
            "CHANGELOG.md:{line}: a breaking entry cites no name that fed4560 (merge base with \
             origin/main) holds ({names}): name what it changes as the last version has it \
             (`git show fed4560:<file>`), or it breaks nothing released"
        )
    };
    assert_eq!(
        crate::breaking_names_gate(
            &unreleased_with("- **Breaking:** `audio_wasm::Engine` is `audio_host::Host`.\n"),
            &held,
            at
        ),
        Ok(()),
        "one name the last version has is enough"
    );
    assert_eq!(
        crate::breaking_names_gate(&unreleased_with("- `New` added.\n"), &BTreeSet::new(), at),
        Ok(()),
        "no breaking entry; the released one is never read"
    );
    assert_eq!(
        crate::breaking_names_gate(
            &unreleased_with("- **Breaking:** `audio_host::Host` changed.\n"),
            &held,
            at
        ),
        failed(format!(
            "the breaking entries of the changelog: {}",
            refused(5, "Host")
        )),
        "a name only this branch added"
    );
    assert_eq!(
        crate::breaking_names_gate(
            &unreleased_with(
                "- **Breaking:** nothing cited.\n- **Breaking:** `New`, `Newer` changed.\n"
            ),
            &held,
            at
        ),
        found(
            "the breaking entries of the changelog",
            &[refused(5, "it cites none"), refused(6, "New, Newer")]
        ),
        "each entry at fault, on its line"
    );
}

#[test]
fn searches_the_names_real_git_holds_at_the_merge_base_with_main() {
    let dir = scratch("breaking-names");
    let _init = hermetic_git(&dir, &["init", "--quiet", "--initial-branch=main"]);
    std::fs::write(dir.join("CHANGELOG.md"), "# Changelog\n").expect("a changelog");
    std::fs::write(
        dir.join("lib.rs"),
        "pub fn released_name() {}\npub fn other() {}\n",
    )
    .expect("a source");
    let _add = hermetic_git(&dir, &["add", "."]);
    let _commit = hermetic_git(&dir, &["commit", "--quiet", "-m", "docs: released"]);
    let base = hermetic_git(&dir, &["rev-parse", "HEAD"]);
    let _branch = hermetic_git(&dir, &["checkout", "--quiet", "-b", "work"]);
    std::fs::write(dir.join("lib.rs"), "pub fn new_name() {}\n").expect("a source");
    let _changed = hermetic_git(&dir, &["commit", "--quiet", "-am", "feat!: rename"]);
    let mut runner = system(dir.path());
    assert_eq!(
        crate::release::breaking_names(
            &mut runner,
            &unreleased_with("- **Breaking:** `released_name` is `new_name`.\n")
        )
        .map_err(|failure| failure.summary),
        Ok(()),
        "a name the merge base holds, the new one apart"
    );
    assert_eq!(
        crate::release::breaking_names(
            &mut runner,
            &unreleased_with("- **Breaking:** `new_name` and `released` changed.\n")
        )
        .map_err(|failure| failure.summary),
        Err(format!(
            "the breaking entries of the changelog: CHANGELOG.md:5: a breaking entry cites no \
             name that {base} (merge base with main) holds (new_name, released): name what it \
             changes as the last version has it (`git show {base}:<file>`), or it breaks \
             nothing released",
            base = base.trim()
        )),
        "a name only the branch holds, and a part of a word, are not found"
    );
    assert_eq!(
        crate::release::breaking_names(&mut runner, &unreleased_with("- `x` added.\n"))
            .map_err(|failure| failure.summary),
        Ok(()),
        "no breaking entry, nothing searched"
    );
}

#[test]
fn searches_the_breaking_names_against_main() {
    let step: Vec<Step> = check_steps(CHANGES_MODE)
        .into_iter()
        .filter(|step| step.action == Action::VersionGate)
        .collect();
    let breaking = LOG.replace("- x\n", "- **Breaking:** `Gone` and `Kept` changed.\n");
    let search = "capture git grep --color=never --only-matching -h --word-regexp \
                  --fixed-strings -e Changelog -e Gone -e Kept \
                  fed4560000000000000000000000000000000000 --";
    let mut fake = passing();
    let _log = fake.files.insert(CHANGELOG.to_owned(), breaking);
    let _found = fake
        .files
        .insert(search.to_owned(), "Changelog\nKept\n".to_owned());
    assert_eq!(
        run_steps(&step, &mut fake, &mut Vec::new()),
        Ok(()),
        "a name the merge base holds"
    );
    let _lost = fake
        .files
        .insert(search.to_owned(), "Changelog\n".to_owned());
    assert!(
        run_steps(&step, &mut fake, &mut Vec::new()).is_err_and(|failure| failure.summary
            == "One version everywhere, changelog well formed, released sections as on main: \
                the breaking entries of the changelog: CHANGELOG.md:9: a breaking entry cites \
                no name that fed4560000000000000000000000000000000000 (merge base with \
                origin/main) holds (Gone, Kept): name what it changes as the last version has \
                it (`git show fed4560000000000000000000000000000000000:<file>`), or it breaks \
                nothing released"),
        "none found"
    );
    let _unsearched = fake.files.remove(search);
    assert!(
        run_steps(&step, &mut fake, &mut Vec::new()).is_err_and(|failure| failure
            .summary
            .contains(
                "cannot search the names the breaking entries cite at the merge base fed4560"
            )),
        "git cannot search"
    );
}

/// Every workspace manifest with the same content.
fn packages(text: &str) -> Vec<(String, String)> {
    package_jsons()
        .into_iter()
        .map(|path| (path, text.to_owned()))
        .collect()
}

/// The refusal of the entry at `line` of a changelog by [`crate::duplicate_entries_gate`],
/// repeating the one at `first` for `why`.
fn repeated(line: u8, first: u8, why: &str) -> String {
    format!(
        "CHANGELOG.md:{line}: this entry and the one at line {first} describe one change \
         ({why}): keep one entry per change, saying it from the last version"
    )
}

#[test]
fn refuses_two_entries_of_one_change_by_their_opening_or_their_names() {
    let context = "the entries of [Unreleased]";
    let gate = |entries: &str| crate::duplicate_entries_gate(&unreleased_with(entries));
    // The thresholds the cases below are built around.
    assert_eq!(
        (crate::SAME_OPENING, crate::SAME_NAMES),
        (8, 3),
        "as stated"
    );
    // A rebase's copy (kept twice in the history): the same opening, then each wording.
    assert_eq!(
        gate(
            "- The engine's output stage (`spec/engine-design.md`, decision 12): after the \
             plan, every non-finite sample is replaced.\n\n\
             - The engine's output stage (`spec/engine-design.md`, decision 12): after the \
             plan, before delivery, every non-finite sample is replaced and reported.\n"
        ),
        failed(format!(
            "{context}: {}",
            repeated(7, 5, "both open with the same 8 words")
        )),
        "the same opening"
    );
    // Its other copy (kept once in the history): the same names, worded apart.
    assert_eq!(
        gate(
            "- **Breaking:** `audio_core::CoreError` has three variants more: \
             `TooManyChannels` (with the `Direction` of the channels refused), `OfflineOnly` \
             and `Unbuilt`.\n\
             - **Breaking:** `audio_core::CoreError` has five variants more: `TooManyChannels` \
             (with the `Direction` of the channels refused), `OfflineOnly`, `Unbuilt`, \
             `StageChannels` and `UsedStage`.\n"
        ),
        failed(format!(
            "{context}: {}",
            repeated(6, 5, "both cite 5 of the 7 code spans either cites")
        )),
        "the same names"
    );
    // Two changes of one function from the history: six words alike, and three of five
    // spans alike.
    assert_eq!(
        gate(
            "- `WasmEngine.create` and the benchmarks' counted instance read the engine's \
             explanation without checking it (`ModuleError`).\n\
             - `WasmEngine.create` and the benchmarks' counted instance took any `engine_load` \
             result other than 1 for a refused patch (`ModuleError`).\n\
             - engine-wasm: `renderOffline` refuses a negative number of frames with a \
             `RangeError`, as `capture` and `captureEngine` do.\n\
             - engine-wasm: `renderOffline`, `capture` and `captureEngine` no longer wait \
             forever (`AudioContext`).\n"
        ),
        Ok(()),
        "two changes each"
    );
}

#[test]
fn takes_two_entries_for_one_change_from_its_thresholds_on() {
    let context = "the entries of [Unreleased]";
    let gate = |entries: &str| crate::duplicate_entries_gate(&unreleased_with(entries));
    // At each threshold, and one below: seven words alike, then eight; two spans alike in
    // three, then three in four; four spans alike in six, then in seven.
    assert_eq!(
        gate("- one two three four five six seven a.\n- one two three four five six seven b.\n"),
        Ok(()),
        "seven words"
    );
    assert_eq!(
        gate(
            "- one two three four five six seven eight a.\n- one two three four five six seven eight b.\n"
        ),
        failed(format!(
            "{context}: {}",
            repeated(6, 5, "both open with the same 8 words")
        )),
        "eight words"
    );
    assert_eq!(
        gate("- `a` and `b`.\n- Then `a`, `b` and `c`.\n"),
        Ok(()),
        "two spans alike"
    );
    assert_eq!(
        gate("- `a`, `b` and `c`.\n- Then `a`, `b`, `c` and `d`.\n"),
        failed(format!(
            "{context}: {}",
            repeated(6, 5, "both cite 3 of the 4 code spans either cites")
        )),
        "three spans alike, of four"
    );
    assert_eq!(
        gate("- `a`, `b`, `c`, `d` and `e`.\n- Then `a`, `b`, `c`, `d` and `f`.\n"),
        failed(format!(
            "{context}: {}",
            repeated(6, 5, "both cite 4 of the 6 code spans either cites")
        )),
        "two thirds exactly"
    );
    assert_eq!(
        gate("- `a`, `b`, `c`, `d` and `e`.\n- Then `a`, `b`, `c`, `d`, `f` and `g`.\n"),
        Ok(()),
        "four spans alike, of seven"
    );
    // Across the kinds of change, each later copy, and never a line within an entry nor
    // the released sections.
    assert_eq!(
        gate(
            "### Added\n\n- `a`, `b` and `c` added.\n\n### Changed\n\n\
             - `a`, `b` and `c` changed.\n  - `a`, `b` and `c` inside it.\n\
             - Again `a`, `b` and `c`.\n"
        ),
        found(
            context,
            &[
                repeated(11, 7, "both cite 3 of the 3 code spans either cites"),
                repeated(13, 7, "both cite 3 of the 3 code spans either cites"),
            ]
        ),
        "each copy, of the first entry it repeats"
    );
    assert_eq!(
        crate::duplicate_entries_gate(
            "# Changelog\n\n## [Unreleased]\n\n- `x`, `y` and `z` added.\n\n\
             ## [0.1.0] - 2026-11-08\n\n- `x`, `y` and `z` added.\n- `x`, `y` and `z` again.\n"
        ),
        Ok(()),
        "released sections are never read"
    );
}
