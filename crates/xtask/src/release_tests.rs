//! Tests of versions, the changelog and the release gates.

use alloc::collections::BTreeSet;
use std::path::Path;

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
    let next = CARGO.replace(
        "0.2.0\"\n\n[workspace.dependencies]",
        "0.3.0\"\n\n[workspace.dependencies]",
    );
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
    let bad = CARGO.replace(
        "0.2.0\"\n\n[workspace.dependencies]",
        "0.2\"\n\n[workspace.dependencies]",
    );
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
    let old = CARGO.replace(
        "0.2.0\"\n\n[workspace.dependencies]",
        "0.1.9\"\n\n[workspace.dependencies]",
    );
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
    for (engines, read) in [
        (r#""engines": {"node": ">=22.17"}"#, ">=22.17"),
        (r#""engines": {"node": ">=22.18.1"}"#, ">=22.18.1"),
        (r#""engines": {}"#, "(missing)"),
    ] {
        let manifest = format!(r#"{{"version": "0.2.0", {engines}}}"#);
        assert_eq!(
            release_gate(CARGO, &roadmap_package(&manifest), LOG)
                .map_err(|failure| failure.summary),
            Err(format!(
                "the versions and the changelog: {ROADMAP_PACKAGE}: engines.node is {read}, the packages support Node.js >=22.18 (NODE_FLOOR)"
            )),
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
    let restore = "copy it back as fed4560 holds it (see: git show fed4560:CHANGELOG.md)";
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
    for (changelog, at_line, why) in [
        (
            RELEASED.replace("- one.\n", "- one.\n\n"),
            14_u32,
            "a line added at its end",
        ),
        (
            RELEASED.replace("- one.\n", "- one."),
            13_u32,
            "its last line break removed",
        ),
        (
            RELEASED.replace("- one.\n", "- one.\r\n"),
            13_u32,
            "a line ending changed",
        ),
    ] {
        assert_eq!(
            crate::released_sections_gate(Some(RELEASED), &changelog, at)
                .map_err(|failure| failure.summary),
            Err(format!(
                "the released changelog sections: CHANGELOG.md:{at_line}: the released section [0.1.0] differs from the one at fed4560 (merge base with origin/main): a released section is never edited: copy it back as fed4560 holds it (see: git show fed4560:CHANGELOG.md)"
            )),
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
             edited: copy it back as {with} holds it (see: git show {with}:CHANGELOG.md)",
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
                         at {base}: a released section is never edited: copy it back as fed4560000000000000000000000000000000000 holds it (see: git show fed4560000000000000000000000000000000000:CHANGELOG.md)"
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
    assert_eq!(
        run_steps(&step, &mut baseless, &mut Vec::new()).map_err(|failure| failure.summary),
        Err("One version everywhere, changelog well formed, released sections as on main: no merge base with origin/main or main to read the released changelog sections from (fetch or create one of them)".to_owned()),
        "no base"
    );
    let mut unreadable = passing();
    unreadable.broken = Some(CHANGELOG_LISTED.0.to_owned());
    assert_eq!(
        run_steps(&step, &mut unreadable, &mut Vec::new()).map_err(|failure| failure.summary),
        Err("One version everywhere, changelog well formed, released sections as on main: cannot read CHANGELOG.md at the merge base fed4560000000000000000000000000000000000: cannot run `git ls-tree --name-only fed4560000000000000000000000000000000000 -- CHANGELOG.md`: not found".to_owned()),
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
             (see: git diff --stat fed4560), or it breaks nothing released"
        )
    };
    let unsaid = |line: u8, names: &str| {
        format!(
            "CHANGELOG.md:{line}: a breaking entry cites {names}, which fed4560 (merge base with \
             origin/main) does not hold and no entry under `### Added` cites: say what the \
             release adds in its added entry, and in the breaking entry what changes of the last \
             version"
        )
    };
    assert_eq!(
        crate::breaking_names_gate(
            &unreleased_with(
                "### Added\n\n- `audio_host::Host`, wrapped\n  in `Wrap`.\n\n### Changed\n\n\
                 - **Breaking:** `audio_wasm::Engine` is `audio_host::Host` in `Wrap`, \
                 `0x5059_0005`.\n"
            ),
            &held,
            at
        ),
        Ok(()),
        "a name the last version has, every new one cited by an added entry, a number naming \
         nothing"
    );
    assert_eq!(
        crate::breaking_names_gate(
            &unreleased_with(
                "### Added\n\n- `Other`.\n\n### Changed\n\n- `Host`, changed.\n\
                 - **Breaking:** `Engine` is `Host`, then `Wrap` and `Host`.\n\
                 ### Fixed\n\n- `Wrap`.\n"
            ),
            &held,
            at
        ),
        failed(format!(
            "the breaking entries of the changelog: {}",
            unsaid(12, "Host, Wrap")
        )),
        "a new name cited only by an entry of another kind, each named once, in its order"
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

/// What the breaking names gate answers, through real git, in a repository at `dir` whose
/// merge base with main holds `released_name`, after `configure` ran there: for a breaking
/// entry citing it, for one citing only a name of the branch and a part of a word, and for
/// no breaking entry; with the merge base.
fn breaking_names_answers(
    dir: &Path,
    configure: &dyn Fn(&Path),
) -> (String, Vec<Result<(), String>>) {
    let _init = hermetic_git(dir, &["init", "--quiet", "--initial-branch=main"]);
    std::fs::write(dir.join("CHANGELOG.md"), "# Changelog\n").expect("a changelog");
    std::fs::write(
        dir.join("lib.rs"),
        "pub fn released_name() {}\npub fn other() {}\n",
    )
    .expect("a source");
    let _add = hermetic_git(dir, &["add", "."]);
    let _commit = hermetic_git(dir, &["commit", "--quiet", "-m", "docs: released"]);
    let base = hermetic_git(dir, &["rev-parse", "HEAD"]).trim().to_owned();
    let _branch = hermetic_git(dir, &["checkout", "--quiet", "-b", "work"]);
    std::fs::write(dir.join("lib.rs"), "pub fn new_name() {}\n").expect("a source");
    let _changed = hermetic_git(dir, &["commit", "--quiet", "-am", "feat!: rename"]);
    configure(dir);
    let mut runner = system(dir.to_path_buf());
    let answers = [
        "### Added\n\n- `new_name`.\n\n### Changed\n\n- **Breaking:** `released_name` is \
         `new_name`.\n",
        "- **Breaking:** `new_name` and `released` changed.\n",
        "- `x` added.\n",
    ]
    .map(|entry| {
        crate::release::breaking_names(&mut runner, &unreleased_with(entry))
            .map_err(|failure| failure.summary)
    })
    .to_vec();
    (base, answers)
}

/// The answers [`breaking_names_answers`] must give, at the merge base `base`.
fn expected_breaking_names(base: &str) -> Vec<Result<(), String>> {
    vec![
        Ok(()),
        Err(format!(
            "the breaking entries of the changelog: CHANGELOG.md:5: a breaking entry cites no \
             name that {base} (merge base with main) holds (new_name, released): name what it \
             changes as the last version has it (see: git diff --stat {base}), or it breaks \
             nothing released"
        )),
        Ok(()),
    ]
}

#[test]
fn searches_the_names_real_git_holds_at_the_merge_base_with_main() {
    let dir = scratch("breaking-names");
    let (base, answers) = breaking_names_answers(&dir, &|_| {});
    assert_eq!(
        answers,
        expected_breaking_names(&base),
        "a name the merge base holds, the new one apart; a name only the branch holds, and a \
         part of a word, are not found; no breaking entry, nothing searched"
    );
}

#[test]
fn searches_the_names_whatever_the_user_s_git_configuration() {
    let dir = scratch("breaking-names-hostile");
    let (base, answers) = breaking_names_answers(&dir, &|repository| {
        for (key, value) in [
            ("grep.lineNumber", "true"),
            ("grep.column", "true"),
            ("grep.fullName", "true"),
            ("grep.patternType", "perl"),
            ("grep.extendedRegexp", "true"),
            ("color.ui", "always"),
            ("color.grep", "always"),
        ] {
            let _set = hermetic_git(repository, &["config", key, value]);
        }
        // Finding H11-1 of the fifth deep review of visual block 2 (#34): attributes
        // marking every file binary.
        std::fs::write(repository.join(".git/info/attributes"), "* binary\n")
            .expect("the attributes");
    });
    assert_eq!(
        answers,
        expected_breaking_names(&base),
        "git grep's names alone, never a line or column number before them"
    );
}

#[test]
fn searches_the_breaking_names_against_main() {
    let step: Vec<Step> = check_steps(CHANGES_MODE)
        .into_iter()
        .filter(|step| step.action == Action::VersionGate)
        .collect();
    let breaking = LOG.replace("- x\n", "- **Breaking:** `Gone` and `Kept` changed.\n");
    let search = "capture git grep --color=never --no-line-number --no-column --only-matching \
                  -h --text --word-regexp \
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
    // Finding H11-1 of the fifth deep review of visual block 2 (#34): the title's word was
    // searched but never asserted found, so `Binary file … matches`, a line of no name,
    // read as every name lacking.
    let _binary = fake.files.insert(
        search.to_owned(),
        "Binary file fed4560000000000000000000000000000000000:CHANGELOG.md matches\n".to_owned(),
    );
    assert_eq!(
        run_steps(&step, &mut fake, &mut Vec::new()),
        failed(
            "One version everywhere, changelog well formed, released sections as on main: git \
             grep found no `Changelog`, the word of the changelog's title, at the merge base \
             fed4560000000000000000000000000000000000: its output is not the words it matched, \
             so no name it cites can be judged"
        ),
        "the title's word not found"
    );
    let _lost = fake
        .files
        .insert(search.to_owned(), "Changelog\n".to_owned());
    assert_eq!(
        run_steps(&step, &mut fake, &mut Vec::new()).map_err(|failure| failure.summary),
        Err("One version everywhere, changelog well formed, released sections as on main: the breaking entries of the changelog: CHANGELOG.md:9: a breaking entry cites no name that fed4560000000000000000000000000000000000 (merge base with origin/main) holds (Gone, Kept): name what it changes as the last version has it (see: git diff --stat fed4560000000000000000000000000000000000), or it breaks nothing released".to_owned()),
        "none found"
    );
    let _unsearched = fake.files.remove(search);
    assert_eq!(
        run_steps(&step, &mut fake, &mut Vec::new()).map_err(|failure| failure.summary),
        Err("One version everywhere, changelog well formed, released sections as on main: cannot search the names the breaking and removed entries cite at the merge base fed4560000000000000000000000000000000000: `git grep --color=never --no-line-number --no-column --only-matching -h --text --word-regexp --fixed-strings -e Changelog -e Gone -e Kept fed4560000000000000000000000000000000000 --` exited with code 1: failed".to_owned()),
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

#[test]
fn refuses_a_breaking_entry_spelled_otherwise_than_the_gates_read_it() {
    let refused = |line: u8| {
        format!(
            "CHANGELOG.md:{line}: a breaking entry starts `- **Breaking`, which the gates of \
             breaking changes read"
        )
    };
    assert_eq!(
        crate::breaking_spellings_gate(&unreleased_with(
            "- **Breaking:** `Released` changed.\n- **Breaking (names):** `x` renamed.\n\
             - `Breaking` is a word.\n- Breakage of a cable.\n  - **BREAKING:** inside an entry.\n\
             - The gates read the breaking entries.\n"
        )),
        Ok(()),
        "the canonical forms, other words and lines within an entry"
    );
    for (entry, form) in [
        ("- **BREAKING:** `x` changed.\n", "upper case"),
        ("- Breaking: `x` changed.\n", "no bold"),
        ("- breaking change: `x`.\n", "lower case"),
        ("- *Breaking*: `x` changed.\n", "italics"),
        ("- __Breaking:__ `x` changed.\n", "underscore bold"),
        ("- _Breaking_: `x` changed.\n", "underscore italics"),
        ("- `x` changed (breaking).\n", "a mark at its end"),
        (
            "- `x` changed (Breaking change).\n",
            "a mark of another wording",
        ),
        ("- `x` changed, breaking.\n", "a word at its end"),
    ] {
        assert_eq!(
            crate::breaking_spellings_gate(&unreleased_with(entry)),
            failed(format!(
                "the breaking entries of the changelog: {}",
                refused(5)
            )),
            "{form}"
        );
    }
    assert_eq!(
        crate::breaking_spellings_gate(
            "# Changelog\n\n## [Unreleased]\n\n## [0.1.0] - 2026-11-08\n\n- BREAKING: `x`.\n"
        ),
        Ok(()),
        "released sections are never read"
    );
}

#[test]
fn refuses_every_line_of_unreleased_its_gates_do_not_read() {
    let refused = |lines: &[u8]| {
        let problems: Vec<String> = lines
            .iter()
            .map(|line| {
                format!(
                    "CHANGELOG.md:{line}: a line of [Unreleased] is blank, a heading `### \
                     <kind>` (Added, Changed, Deprecated, Removed, Fixed, Security), an entry \
                     `- <text>`, or a line indented below an entry"
                )
            })
            .collect();
        found("the shape of [Unreleased]", &problems)
    };
    assert_eq!(
        crate::CHANGE_KINDS,
        [
            "Added",
            "Changed",
            "Deprecated",
            "Removed",
            "Fixed",
            "Security"
        ],
        "Keep a Changelog's kinds"
    );
    assert_eq!(
        crate::unreleased_shape_gate(&unreleased_with(
            "### Added\n\n- **Breaking:** `x` changed,\n  wrapped.\n  - a list in it.\n\n\
             \t  and a tab.\n\n### Changed\n   \n### Deprecated\n### Removed\n### Fixed\n\
             ### Security\n- An entry.\n"
        )),
        Ok(()),
        "every shape it reads, each kind"
    );
    for (entries, form) in [
        ("* **Breaking:** `x` changed.\n", "another bullet"),
        ("+ **Breaking:** `x` changed.\n", "a plus bullet"),
        ("1. **Breaking:** `x` changed.\n", "a numbered entry"),
        ("-**Breaking:** `x` changed.\n", "no space after the bullet"),
        ("-  **Breaking:** `x` changed.\n", "two spaces"),
        ("- \n", "an empty entry"),
        ("A paragraph.\n", "a paragraph"),
        ("### Other\n", "another kind"),
        ("#### Added\n", "another level"),
        (
            "  indented, under no entry.\n",
            "a line indented under no entry",
        ),
    ] {
        assert_eq!(
            crate::unreleased_shape_gate(&unreleased_with(entries)),
            refused(&[5]),
            "{form}"
        );
    }
    assert_eq!(
        crate::unreleased_shape_gate(&unreleased_with(
            "- An entry.\n### Fixed\n  under a heading.\n* one\n* two\n"
        )),
        refused(&[7, 8, 9]),
        "every line, each with its number; a heading ends the entry above it"
    );
    assert_eq!(
        crate::unreleased_shape_gate(
            "# Changelog\n\nA preamble.\n* free\n\n## [Unreleased]\n\n## [0.1.0] - 2026-11-08\n\n* old\n"
        ),
        Ok(()),
        "the preamble and the released sections are never read"
    );
    assert_eq!(
        crate::unreleased_shape_gate("# Changelog\n\n* no section\n"),
        Ok(()),
        "no section, nothing read (the release gate refuses it)"
    );
}

#[test]
fn refuses_a_blank_line_between_two_entries_of_one_section() {
    let split = |lines: &[u8]| {
        let problems: Vec<String> = lines
            .iter()
            .map(|line| {
                format!(
                    "CHANGELOG.md:{line}: a blank line between two entries of one section: the \
                     entries of a section follow each other, one list"
                )
            })
            .collect();
        found("the shape of [Unreleased]", &problems)
    };
    assert_eq!(
        crate::unreleased_shape_gate(&unreleased_with("### Added\n\n- One.\n\n- Two.\n")),
        split(&[8]),
        "one blank line, named at its line"
    );
    assert_eq!(
        crate::unreleased_shape_gate(&unreleased_with(
            "### Added\n\n- One,\n  wrapped.\n\n \n- Two.\n- Three.\n\n\n- Four.\n"
        )),
        split(&[9, 13]),
        "after a wrapped entry, a run of blank lines named at its first, each split"
    );
    assert_eq!(
        crate::unreleased_shape_gate(&unreleased_with(
            "### Added\n\n- One,\n\n  continued after a blank line.\n- Two.\n\n### Fixed\n\n\
             - Three.\n"
        )),
        Ok(()),
        "a blank line inside an entry, or around a heading, splits no list"
    );
}

#[test]
fn reads_an_entry_with_the_lines_indented_below_it() {
    assert_eq!(
        crate::release::unreleased_entries(&unreleased_with(
            "### Added\n\n- One `a`\n  wrapped `b\n  c`.\n\n  and on.\n- Two.\n### Fixed\n\
             \x20 under a heading.\n- Three.\n"
        )),
        vec![
            (7, "One `a` wrapped `b c`. and on.".to_owned()),
            (12, "Two.".to_owned()),
            (15, "Three.".to_owned())
        ],
        "each entry whole, a span wrapped across lines one span; a heading ends it"
    );
    let gate = |entries: &str| crate::duplicate_entries_gate(&unreleased_with(entries));
    assert_eq!(
        gate(
            "- First change.\n  It cites `a`, `b` and `c`.\n- Second change.\n  Again `a`, `b` and `c`.\n"
        ),
        failed(format!(
            "the entries of [Unreleased]: {}",
            repeated(7, 5, "both cite 3 of the 3 code spans either cites")
        )),
        "the spans of the lines below an entry are the entry's"
    );
    assert_eq!(
        crate::release::breaking_entries(&unreleased_with(
            "- **Breaking:** the first,\n  `wrapped::Name` below it.\n"
        )),
        vec![(5, vec!["Name".to_owned()])],
        "a breaking entry's names below its first line"
    );
}

#[test]
fn reports_every_gate_of_the_version_step_that_fails() {
    // The gates of one step are independent: each runs whatever the others found, so that
    // one run reports every failure (`all_of`), as the full check reports every failed step.
    let step: Vec<Step> = check_steps(CHANGES_MODE)
        .into_iter()
        .filter(|step| step.action == Action::VersionGate)
        .collect();
    let mut fake = passing();
    let changelog = LOG.replace(
        "- x\n",
        "- One `a`, `b` and `c`.\n- Again `a`, `b` and `c`.\n- **BREAKING:** `y` changed.\n\
         * An entry under another bullet.\n",
    );
    let _changelog = fake.files.insert(CHANGELOG.to_owned(), changelog);
    assert_eq!(
        run_steps(&step, &mut fake, &mut Vec::new()),
        Err(Failure {
            summary: "One version everywhere, changelog well formed, released sections as on \
                      main: 3 gates failed, listed above"
                .to_owned(),
            details: [
                "the shape of [Unreleased]: CHANGELOG.md:12: a line of [Unreleased] is blank, a \
                 heading `### <kind>` (Added, Changed, Deprecated, Removed, Fixed, Security), \
                 an entry `- <text>`, or a line indented below an entry",
                "the entries of [Unreleased]: CHANGELOG.md:10: this entry and the one at line 9 \
                 describe one change (both cite 3 of the 3 code spans either cites): keep one \
                 entry per change, saying it from the last version",
                "the breaking entries of the changelog: CHANGELOG.md:11: a breaking entry starts \
                 `- **Breaking`, which the gates of breaking changes read",
            ]
            .map(str::to_owned)
            .to_vec(),
            usage: false,
        }),
        "every failure of the step, each gate's summary"
    );
}

#[test]
fn refuses_two_entries_sharing_a_long_run_of_words() {
    let context = "the entries of [Unreleased]";
    let gate = |entries: &str| crate::duplicate_entries_gate(&unreleased_with(entries));
    assert_eq!(crate::SAME_RUN, 16, "as measured");
    // The pair a block kept (second deep review of visual block 2, B7-2): its added entry, then its
    // breaking one, worded from the same sentence, with different openings and one span alike.
    assert_eq!(
        gate(
            "- No visual box may output a subnormal value or -0: the finite-values law refuses \
             both (a box writes 0 instead), so that a target flushing subnormals to zero, as \
             GPUs do, cannot change an image's bits or its hash (`spec/visual-design.md`, \
             section 3).\n\
             - **Breaking:** a visual box may no longer output a subnormal value or -0, a \
             contract `VisualBox::render` now states: the finite-values law refuses both (a \
             box writes 0 instead), so that a target flushing subnormals to zero, as GPUs do, \
             cannot change an image's bits or its hash (`spec/visual-design.md`, section 3); \
             a box that wrote one fails the laws.\n"
        ),
        failed(format!(
            "{context}: {}",
            repeated(6, 5, "both hold the same run of 16 words")
        )),
        "a sentence kept in two entries"
    );
    // At the threshold, and one below, the run inside each entry.
    let run = |words: usize| {
        let shared: Vec<String> = (1..=words).map(|word| format!("w{word}")).collect();
        let shared = shared.join(" ");
        gate(&format!(
            "- First: {shared} then one.\n- Second, otherwise: {shared} and two.\n"
        ))
    };
    assert_eq!(run(15), Ok(()), "fifteen words alike");
    assert_eq!(
        run(16),
        failed(format!(
            "{context}: {}",
            repeated(6, 5, "both hold the same run of 16 words")
        )),
        "sixteen"
    );
}

#[test]
fn refuses_a_removed_entry_naming_what_the_last_version_lacked() {
    let at = ("fed4560", "origin/main");
    let found: BTreeSet<String> = ["Image", "new", "Released"].map(str::to_owned).into();
    let gate = |entries: &str| crate::removed_names_gate(&unreleased_with(entries), &found, at);
    assert_eq!(
        gate(
            "### Added\n\n- `DisplaySizeMismatch` and `OutOfMemory` added.\n\n### Removed\n\n\
             - `Image::new`, gone: its replacement belongs to the entry that adds it.\n"
        ),
        Ok(()),
        "every name a removed entry cites, the last version held; other kinds are not read"
    );
    // The entries a block's rebase left under Removed (second deep review of visual block 2,
    // B14-2): a variant added on the branch and removed again, or never released.
    assert_eq!(
        gate(
            "### Removed\n\n- `GpuError::DisplaySizeMismatch` replaces `Image`'s check.\n\
             - Nothing named.\n\n### Fixed\n\n- `OutOfMemory` fixed.\n"
        ),
        failed(
            "the removed entries of the changelog: CHANGELOG.md:7: a removed entry cites \
             DisplaySizeMismatch, which fed4560 (merge base with origin/main) does not hold: \
             [Unreleased] removes only what the last version had (see: git diff --stat \
             fed4560)"
        ),
        "a name only the branch held; an entry naming nothing has nothing to check"
    );
    assert_eq!(
        gate("### Removed\n- `Gone` and `Lost`, before any blank line, `Gone` once.\n"),
        failed(
            "the removed entries of the changelog: CHANGELOG.md:6: a removed entry cites Gone, \
             Lost, which fed4560 (merge base with origin/main) does not hold: [Unreleased] \
             removes only what the last version had (see: git diff --stat fed4560)"
        ),
        "every name missing"
    );
}

#[test]
fn searches_the_removed_names_against_main() {
    let step: Vec<Step> = check_steps(CHANGES_MODE)
        .into_iter()
        .filter(|step| step.action == Action::VersionGate)
        .collect();
    let removed = LOG.replace("- x\n", "- x\n\n### Removed\n\n- `Gone` and `Kept`.\n");
    let search = "capture git grep --color=never --no-line-number --no-column --only-matching \
                  -h --text --word-regexp \
                  --fixed-strings -e Changelog -e Gone -e Kept \
                  fed4560000000000000000000000000000000000 --";
    let mut fake = passing();
    let _log = fake.files.insert(CHANGELOG.to_owned(), removed);
    let _found = fake
        .files
        .insert(search.to_owned(), "Changelog\nKept\nGone\n".to_owned());
    assert_eq!(
        run_steps(&step, &mut fake, &mut Vec::new()),
        Ok(()),
        "both names held at the merge base"
    );
    let _lost = fake
        .files
        .insert(search.to_owned(), "Changelog\nKept\n".to_owned());
    assert_eq!(
        run_steps(&step, &mut fake, &mut Vec::new()).map_err(|failure| failure.summary),
        Err("One version everywhere, changelog well formed, released sections as on main: the removed entries of the changelog: CHANGELOG.md:13: a removed entry cites Gone, which fed4560000000000000000000000000000000000 (merge base with origin/main) does not hold: [Unreleased] removes only what the last version had (see: git diff --stat fed4560000000000000000000000000000000000)".to_owned()),
        "one name the merge base lacks"
    );
}
