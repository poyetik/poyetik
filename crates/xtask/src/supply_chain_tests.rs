//! Tests of the supply-chain configuration xtask owns, and its gate.

use crate::tests::{PACKAGE, PNPM, failed, found};

/// The texts of the supply-chain files, in the order of `SUPPLY_CHAIN_FILES`.
/// The GPU workspace's policy is `gpu_deny_config()` there.
fn supply_texts(deny: &str, dependabot: &str, pnpm: &str, package: &str) -> [String; 5] {
    [
        deny.to_owned(),
        dependabot.to_owned(),
        pnpm.to_owned(),
        package.to_owned(),
        crate::gpu_deny_config(),
    ]
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
             minimumReleaseAge, overrides, catalog, auditConfig): a setting of pnpm is a change \
             of xtask"
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
    assert_eq!(
        package(
            PACKAGE,
            &[
                ".pnpmfile.cjs".to_owned(),
                "apps/x/.pnpmfile.mjs".to_owned()
            ]
        ),
        found(
            "the supply-chain configuration",
            &[".pnpmfile.cjs", "apps/x/.pnpmfile.mjs"].map(|path| {
                format!(
                    "{path}: a pnpm hooks file, whose code pnpm runs at every install, \
                     rewriting the manifests it resolves (`readPackage`) and its settings \
                     (`updateConfig`): a setting goes in pnpm-workspace.yaml, whose keys xtask \
                     checks"
                )
            })
        ),
        "every pnpm hooks file, in either module form"
    );
}

/// A reason of the audit exception register, as xtask accepts it: the advisory, then its
/// reason, continued on a hanging indent, citing the roadmap step that removes it.
const REASON: &str =
    "# GHSA-fc5q-39xc-vjpf (picomatch): development tools only,\n#   removed by #24 step 152.\n";

/// The audit exceptions of [`REASON`], as xtask accepts them.
const IGNORED: &str = "auditConfig:\n  ignoreGhsas:\n    - GHSA-fc5q-39xc-vjpf\n";

/// The line of [`IGNORED`] that lists the advisory.
const ITEM: &str = "    - GHSA-fc5q-39xc-vjpf\n";

/// The verdict of the supply-chain gate on `text`, xtask's other files as it requires them.
fn workspace(text: &str) -> Result<(), crate::steps::Failure> {
    crate::supply_chain_gate(
        &supply_texts(crate::DENY_CONFIG, crate::DEPENDABOT_CONFIG, text, PACKAGE),
        &[],
    )
}

#[test]
fn accepts_audit_exceptions_each_with_its_reason() {
    assert_eq!(
        workspace(&format!("{REASON}{IGNORED}{PNPM}")),
        Ok(()),
        "one advisory ignored with its reason, the keys after it read as before"
    );
    assert_eq!(
        workspace(&format!(
            "# GHSA-2222-3333-4444 (@scope/glob-2.x_y~z): a reason cited on its first line, #24 \
             step 1.\n{PNPM}\
             auditConfig:\n\n  ignoreGhsas:\n    - GHSA-2222-3333-4444\n\n{REASON}{ITEM}"
        )),
        Ok(()),
        "two advisories, a reason anywhere at column 0, blank lines in the list"
    );
}

#[test]
fn refuses_an_audit_exception_without_its_reason() {
    let unexplained = |at: usize, id: &str| {
        format!(
            "pnpm-workspace.yaml:{at}: `{id}` ignored by `pnpm audit` without its reason: \
             write `# {id} (<package>): <reason>, removed by #N step M` at column 0"
        )
    };
    assert_eq!(
        workspace(&format!(
            "{PNPM}{REASON}{IGNORED}    - GHSA-2222-3333-4444\n    - GHSA-2222-3333-4444\n"
        )),
        found(
            "the supply-chain configuration",
            &[
                "pnpm-workspace.yaml:18: `GHSA-2222-3333-4444` ignored a second time".to_owned(),
                unexplained(17, "GHSA-2222-3333-4444"),
            ]
        ),
        "an advisory without its reason, an advisory twice"
    );
    assert_eq!(
        workspace(&format!(
            "{PNPM}{REASON}overrides:\n    - GHSA-fc5q-39xc-vjpf\n"
        )),
        found(
            "the supply-chain configuration",
            &[
                "pnpm-workspace.yaml:14: `overrides` a second time".to_owned(),
                unlisted_reason(12),
            ]
        ),
        "a reason for an advisory the audit does not ignore, the same line under another key"
    );
    assert_eq!(
        workspace(&format!("{PNPM}{REASON}{REASON}{IGNORED}")),
        failed(
            "the supply-chain configuration: pnpm-workspace.yaml:14: the reason of \
             `GHSA-fc5q-39xc-vjpf` a second time"
        ),
        "a reason twice"
    );
}

#[test]
fn refuses_a_reason_that_cites_no_step_saying_what_it_did_not_read() {
    let no_step = |at: usize, unread: &str| {
        format!(
            "pnpm-workspace.yaml:{at}: the reason of `GHSA-fc5q-39xc-vjpf` cites no roadmap \
             step (`#N step M`) that removes the exception (a reason is continued on the \
             lines right under it opening with `#  `){unread}"
        )
    };
    for (reason, unread) in [
        (
            "# GHSA-fc5q-39xc-vjpf (picomatch): development tools only.\n",
            "",
        ),
        (
            "# GHSA-fc5q-39xc-vjpf (picomatch): development tools only,\n# removed by #24 step 152.\n",
            "; line 13, `# removed by #24 step 152.`, is not read as its continuation",
        ),
        (
            "# GHSA-fc5q-39xc-vjpf (picomatch): development tools only,\n#  still read,\n#removed by #24 \
             step 152.\n",
            "; line 14, `#removed by #24 step 152.`, is not read as its continuation",
        ),
        (
            "# GHSA-fc5q-39xc-vjpf (picomatch): development tools only, removed by #24.\n",
            "",
        ),
    ] {
        assert_eq!(
            workspace(&format!("{PNPM}{reason}{IGNORED}")),
            failed(format!(
                "the supply-chain configuration: {}",
                no_step(12, unread)
            )),
            "{reason}"
        );
    }
    assert_eq!(
        workspace(&format!(
            "{PNPM}# GHSA-fc5q-39xc-vjpf (picomatch): development tools only.\n# GHSA-2222-3333-4444 (anymatch): \
             another, removed by #24 step 1.\nauditConfig:\n  ignoreGhsas:\n    - \
             GHSA-fc5q-39xc-vjpf\n    - GHSA-2222-3333-4444\n"
        )),
        failed(format!(
            "the supply-chain configuration: {}",
            no_step(12, "")
        )),
        "the next reason is not a continuation left unread"
    );
    let malformed = |at: usize, line: &str| {
        format!(
            "pnpm-workspace.yaml:{at}: `{line}`, where xtask reads a reason as `# GHSA-… \
             (<package>): <reason>`, continued on lines opening with `#  `"
        )
    };
    for line in [
        "# GHSA-fc5q-39xc-vjpf",
        "# GHSA-fc5q-39xc-vjpf:",
        "# GHSA-fc5q-39xc-vjpf: ",
        "# GHSA-fc5q-39xc-vjpf development tools only, removed by #24 step 152.",
        "# GHSA-fc5q-39xc-vjp: development tools only, removed by #24 step 152.",
        "# GHSA-fc5q-39xc-vjp (picomatch): development tools only, removed by #24 step 152.",
        "# GHSA-FC5Q-39XC-VJPF (picomatch): development tools only, removed by #24 step 152.",
        "# GHSA-fc5q-39xc-vjpf: development tools only, removed by #24 step 152.",
        "# GHSA-fc5q-39xc-vjpf (picomatch) development tools only, removed by #24 step 152.",
        "# GHSA-fc5q-39xc-vjpf (): development tools only, removed by #24 step 152.",
        "# GHSA-fc5q-39xc-vjpf (pico match): development tools only, removed by #24 step 152.",
        "# GHSA-fc5q-39xc-vjpf (Picomatch): development tools only, removed by #24 step 152.",
        "# GHSA-fc5q-39xc-vjpf (@Scope/glob): development tools only, removed by #24 step 152.",
        "# GHSA-fc5q-39xc-vjpf (@scope/Glob): development tools only, removed by #24 step 152.",
        "# GHSA-fc5q-39xc-vjpf (@scope): development tools only, removed by #24 step 152.",
        "# GHSA-fc5q-39xc-vjpf(picomatch): development tools only, removed by #24 step 152.",
        "# GHSA-fc5q-39xc-vjpf (picomatch: development tools only, removed by #24 step 152.",
    ] {
        assert_eq!(
            workspace(&format!("{PNPM}{line}\n{REASON}{IGNORED}")),
            failed(format!(
                "the supply-chain configuration: {}",
                malformed(12, line)
            )),
            "{line}"
        );
    }
}

#[test]
fn accepts_audit_settings_in_their_one_form() {
    let refused = |at: usize, line: &str| {
        format!(
            "pnpm-workspace.yaml:{at}: `{line}`, not a line xtask accepts under `auditConfig` \
             (`  ignoreGhsas:`, then one `    - GHSA-…` per advisory): a setting of pnpm is a \
             change of xtask"
        )
    };
    assert_eq!(
        workspace(&format!(
            "{PNPM}{REASON}auditConfig:\n    - GHSA-fc5q-39xc-vjpf\n  ignoreGhsas:\n{ITEM}"
        )),
        failed(format!(
            "the supply-chain configuration: {}",
            refused(15, "    - GHSA-fc5q-39xc-vjpf")
        )),
        "an advisory before the list"
    );
    let empty = "pnpm-workspace.yaml: `auditConfig` ignores no advisory: remove the key, \
                 absent while the register of exceptions is empty";
    assert_eq!(
        workspace(&format!(
            "{PNPM}{REASON}auditConfig: {{ignoreGhsas: [GHSA-fc5q-39xc-vjpf]}}\n"
        )),
        found(
            "the supply-chain configuration",
            &[
                "pnpm-workspace.yaml:14: `auditConfig: {ignoreGhsas: [GHSA-fc5q-39xc-vjpf]}`, \
                 where xtask accepts `auditConfig:` alone, its advisories on the lines under it"
                    .to_owned(),
                unlisted_reason(12),
                empty.to_owned(),
            ]
        ),
        "a flow mapping"
    );
    assert_eq!(
        workspace(&format!("{PNPM}auditConfig:\n  ignoreGhsas:\n")),
        failed(format!("the supply-chain configuration: {empty}")),
        "an empty list"
    );
    assert_eq!(
        workspace(&format!("{PNPM}{REASON}{IGNORED}{IGNORED}")),
        found(
            "the supply-chain configuration",
            &[
                "pnpm-workspace.yaml:17: `auditConfig` a second time".to_owned(),
                refused(18, "  ignoreGhsas:"),
                "pnpm-workspace.yaml:19: `GHSA-fc5q-39xc-vjpf` ignored a second time".to_owned(),
            ]
        ),
        "the key twice"
    );
}

#[test]
fn refuses_each_kind_of_line_under_audit_settings_saying_what_it_is() {
    let refused = |at: usize, line: &str| {
        format!(
            "pnpm-workspace.yaml:{at}: `{line}`, not a line xtask accepts under `auditConfig` \
             (`  ignoreGhsas:`, then one `    - GHSA-…` per advisory): a setting of pnpm is a \
             change of xtask"
        )
    };
    let comment = |line: &str| {
        format!(
            "pnpm-workspace.yaml:17: `{line}`, a comment under `auditConfig`: a reason is \
             written at column 0, `# <id> (<package>): <reason>`, continued on lines opening \
             with `#  `"
        )
    };
    let not_github = |line: &str| {
        format!(
            "pnpm-workspace.yaml:17: `{line}`, an advisory id not as GitHub writes it: \
             `GHSA-` then three groups of four characters of `23456789cfghjmpqrvwx`, lower \
             case, joined by `-`"
        )
    };
    for (line, problem) in [
        (
            "    - GHSA-FC5Q-39XC-VJPF",
            not_github("    - GHSA-FC5Q-39XC-VJPF"),
        ),
        (
            "    - \"GHSA-fc5q-39xc-vjpf\"",
            not_github("    - \"GHSA-fc5q-39xc-vjpf\""),
        ),
        (
            "    - GHSA-fc5q-39xc-vjp",
            not_github("    - GHSA-fc5q-39xc-vjp"),
        ),
        (
            "    - GHSA-fc5q-39xc-vjpff",
            not_github("    - GHSA-fc5q-39xc-vjpff"),
        ),
        (
            "    - GHSA-fc5q-39xc-vjp1",
            not_github("    - GHSA-fc5q-39xc-vjp1"),
        ),
        (
            "    - GHSA-fc5q-39xc_vjpf",
            not_github("    - GHSA-fc5q-39xc_vjpf"),
        ),
        (
            "    - GHSA-fc5q-39x-cvjpf",
            not_github("    - GHSA-fc5q-39x-cvjpf"),
        ),
        (
            "    - GHSB-fc5q-39xc-vjpf",
            not_github("    - GHSB-fc5q-39xc-vjpf"),
        ),
        ("    - CVE-2099-0001", not_github("    - CVE-2099-0001")),
        ("  ignoreGhsas:", refused(17, "  ignoreGhsas:")),
        ("  ignoreCves:", refused(17, "  ignoreCves:")),
        ("  # A comment.", comment("  # A comment.")),
        (
            "    - GHSA-fc5q-39xc-vjpf # picomatch",
            comment("    - GHSA-fc5q-39xc-vjpf # picomatch"),
        ),
        (
            "   ",
            "pnpm-workspace.yaml:17: a line of spaces under `auditConfig`: leave it empty"
                .to_owned(),
        ),
    ] {
        assert_eq!(
            workspace(&format!("{PNPM}{REASON}{IGNORED}{line}\n")),
            failed(format!("the supply-chain configuration: {problem}")),
            "{line}"
        );
    }
}

/// The problem of the reason of [`REASON`] at line `at` when `auditConfig` ignores nothing.
fn unlisted_reason(at: usize) -> String {
    format!(
        "pnpm-workspace.yaml:{at}: the reason of `GHSA-fc5q-39xc-vjpf`, which `auditConfig` \
         does not ignore: an exception's reason goes with it"
    )
}

#[test]
fn derives_the_gpu_workspace_policy_from_the_root_one() {
    let gpu = crate::gpu_deny_config();
    assert!(gpu.starts_with(crate::GPU_DENY_HEADER), "its own header");
    assert!(
        gpu.ends_with(crate::GPU_DENY_EXCEPTIONS),
        "its exceptions last"
    );
    assert!(
        !gpu.contains("crate = \"libm\"") && crate::DENY_CONFIG.contains("crate = \"libm\""),
        "without the root's libm ban"
    );
    let shared = |text: &str| {
        text.lines()
            .filter(|line| !line.starts_with('#') && !line.contains("libm") && !line.is_empty())
            .take_while(|line| !line.starts_with("[[licenses.exceptions]]"))
            .filter(|line| !matches!(*line, "[[bans.features]]" | "exact = true" | "allow = []"))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        shared(&gpu),
        shared(crate::DENY_CONFIG),
        "every other line the root's"
    );
    let mut texts = supply_texts(crate::DENY_CONFIG, crate::DEPENDABOT_CONFIG, PNPM, PACKAGE);
    texts[4] = format!("{gpu}# an exception added\n");
    let refused = crate::supply_chain_gate(&texts, &[]).expect_err("refused");
    assert!(
        refused.summary.contains("crates/visual-gpu/deny.toml:")
            && refused.summary.contains("`gpu_deny_config`"),
        "names the GPU workspace's file and what xtask derives: {refused:?}"
    );
}
