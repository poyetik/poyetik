//! Tests of the supply-chain configuration xtask owns, and its gate.

use crate::tests::{PACKAGE, PNPM, failed, found};

/// The texts of the supply-chain files, in the order of `SUPPLY_CHAIN_FILES`.
fn supply_texts(deny: &str, dependabot: &str, pnpm: &str, package: &str) -> [String; 4] {
    [deny, dependabot, pnpm, package].map(str::to_owned)
}

#[test]
fn requires_the_supply_chain_configuration_xtask_defines() {
    let accepted = supply_texts(crate::DENY_CONFIG, crate::DEPENDABOT_CONFIG, PNPM, PACKAGE);
    assert_eq!(
        crate::supply_chain_gate(&accepted, &[]),
        Ok(()),
        "xtask's files, the pnpm keys with comments and values"
    );
    let line_after = |text: &str, line: &str| {
        text.lines()
            .position(|found| found == line)
            .expect("the line")
            .saturating_add(2)
    };
    let policy = "the file is xtask's, so a change of its policy (an exception included) is \
                  a change of xtask";
    let yanked = "yanked = \"deny\"";
    let ignored = crate::DENY_CONFIG.replacen(
        yanked,
        &format!("{yanked}\nignore = [\"RUSTSEC-2099-0001\"]"),
        1,
    );
    let cooldown = "      default-days: 7";
    let shortened = crate::DEPENDABOT_CONFIG.replacen(
        cooldown,
        &format!("{cooldown}\n      semver-patch-days: 0"),
        1,
    );
    assert_eq!(
        crate::supply_chain_gate(&supply_texts(&ignored, &shortened, PNPM, PACKAGE), &[]),
        found(
            "the supply-chain configuration",
            &[
                format!(
                    "deny.toml:{}: `ignore = [\"RUSTSEC-2099-0001\"]`, where xtask's \
                     `DENY_CONFIG` has an empty line: {policy}",
                    line_after(crate::DENY_CONFIG, yanked)
                ),
                format!(
                    ".github/dependabot.yml:{}: `      semver-patch-days: 0`, where xtask's \
                     `DEPENDABOT_CONFIG` has `    commit-message:`: {policy}",
                    line_after(crate::DEPENDABOT_CONFIG, cooldown)
                ),
            ]
        ),
        "an advisory ignored, a cooldown shortened"
    );
    let actions = "  - package-ecosystem: github-actions";
    let without = crate::DEPENDABOT_CONFIG
        .split_once(actions)
        .map(|(kept, _)| kept)
        .expect("the ecosystem");
    let lines = |text: &str| text.split('\n').count();
    assert_eq!(
        crate::supply_chain_gate(
            &supply_texts(
                &format!("{}\n", crate::DENY_CONFIG),
                without.trim_end_matches('\n'),
                PNPM,
                PACKAGE
            ),
            &[]
        ),
        found(
            "the supply-chain configuration",
            &[
                format!(
                    "deny.toml:{}: an empty line, where xtask's `DENY_CONFIG` has the end: \
                     {policy}",
                    lines(crate::DENY_CONFIG).saturating_add(1)
                ),
                format!(
                    ".github/dependabot.yml:{}: the end, where xtask's `DEPENDABOT_CONFIG` has \
                     `{actions}`: {policy}",
                    lines(without)
                ),
            ]
        ),
        "a line more, an ecosystem removed"
    );
}

#[test]
fn reads_the_pnpm_workspace_by_its_top_level_keys() {
    let pnpm = |text: &str| {
        crate::supply_chain_gate(
            &supply_texts(crate::DENY_CONFIG, crate::DEPENDABOT_CONFIG, text, PACKAGE),
            &[],
        )
    };
    let not_accepted = |line: usize, text: &str| {
        format!(
            "pnpm-workspace.yaml:{line}: `{text}`, not a top-level key xtask accepts (packages, \
             minimumReleaseAge, overrides, catalog): a setting of pnpm is a change of xtask"
        )
    };
    assert_eq!(
        pnpm(&format!(
            "{PNPM}minimumReleaseAgeExclude:\n  - \"x\"\n\"auditConfig\": {{}}\n"
        )),
        found(
            "the supply-chain configuration",
            &[
                not_accepted(12, "minimumReleaseAgeExclude:"),
                not_accepted(14, "\"auditConfig\": {}"),
            ]
        ),
        "a key xtask has not reviewed, quoted or not"
    );
    assert_eq!(
        pnpm(&format!("{PNPM}- \"tooling/*\"\n")),
        failed(format!(
            "the supply-chain configuration: {}",
            not_accepted(12, "- \"tooling/*\"")
        )),
        "a line of no key at column 0"
    );
    assert_eq!(
        pnpm(&PNPM.replace(
            "minimumReleaseAge: 10080\n",
            "minimumReleaseAge: 0\ncatalog: {}\n"
        )),
        found(
            "the supply-chain configuration",
            &[
                "pnpm-workspace.yaml:5: `minimumReleaseAge: 0`, where xtask requires \
                 `minimumReleaseAge: 10080`"
                    .to_owned(),
                "pnpm-workspace.yaml:11: `catalog` a second time".to_owned(),
            ]
        ),
        "a shorter age, a key twice"
    );
    assert_eq!(
        pnpm(
            &PNPM
                .replace("minimumReleaseAge: 10080\n", "")
                .replace("catalog:\n", "catalog:\n  minimumReleaseAge: 10080\n")
        ),
        failed(
            "the supply-chain configuration: pnpm-workspace.yaml lacks the top-level key \
             `minimumReleaseAge`"
        ),
        "the age moved under another key"
    );
    assert_eq!(
        pnpm("{packages: [], minimumReleaseAge: 10080, overrides: {}, catalog: {}}\n"),
        found(
            "the supply-chain configuration",
            &[
                not_accepted(
                    1,
                    "{packages: [], minimumReleaseAge: 10080, overrides: {}, catalog: {}}"
                ),
                "pnpm-workspace.yaml lacks the top-level key `packages`".to_owned(),
                "pnpm-workspace.yaml lacks the top-level key `minimumReleaseAge`".to_owned(),
                "pnpm-workspace.yaml lacks the top-level key `overrides`".to_owned(),
                "pnpm-workspace.yaml lacks the top-level key `catalog`".to_owned(),
            ]
        ),
        "a flow mapping"
    );
}

#[test]
fn refuses_pnpm_settings_outside_the_pnpm_workspace() {
    let package = |text: &str, npm_configs: &[String]| {
        crate::supply_chain_gate(
            &supply_texts(crate::DENY_CONFIG, crate::DEPENDABOT_CONFIG, PNPM, text),
            npm_configs,
        )
    };
    assert_eq!(
        package(r#"{"name": "x", "pnpm": {"overrides": {}}}"#, &[]),
        failed(
            "the supply-chain configuration: package.json holds a `pnpm` key, from which pnpm \
             reads settings: they go in pnpm-workspace.yaml, whose keys xtask checks"
        ),
        "settings in the root manifest"
    );
    for not_an_object in ["[]", "{"] {
        assert_eq!(
            package(not_an_object, &[]),
            failed("the supply-chain configuration: package.json is not a JSON object"),
            "{not_an_object}"
        );
    }
    assert_eq!(
        package(PACKAGE, &[".npmrc".to_owned(), "apps/x/.npmrc".to_owned()]),
        found(
            "the supply-chain configuration",
            &[".npmrc", "apps/x/.npmrc"].map(|path| {
                format!(
                    "{path}: an npm configuration file, from which pnpm reads settings \
                     (`minimum-release-age-exclude` …): they go in pnpm-workspace.yaml, whose \
                     keys xtask checks"
                )
            })
        ),
        "every npm configuration file"
    );
}
