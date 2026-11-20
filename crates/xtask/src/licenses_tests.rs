//! Tests of the license gates: the JavaScript dependencies' licenses, and each part's license
//! against its folder's.

use crate::tests::{LICENSE_TEXTS, README_LICENSE, SHIPPED, TOOLING, failed, step_found};
use crate::{Failure, license_gate};

#[test]
fn allows_strict_licenses_for_what_ships_and_wider_ones_for_tooling() {
    assert_eq!(license_gate(SHIPPED, TOOLING), Ok(()), "allowed");
    assert_eq!(
        license_gate(
            r#"{"MPL-2.0":[{"name":"axe-core"}],"MIT":[{"name":"react"},{"name":"react-dom"}]}"#,
            r#"{"GPL-3.0":[{"name":"bad"}],"LGPL-3.0-or-later":[{"name":"other-lgpl"},{"name":"@img/sharp-libvips-x"}],"CC0-1.0":[{"name":"data"}],"Apache-2.0":[{"name":"typescript"}]}"#
        ),
        failed(
            "licenses not allowed: 3 problems, listed above\n\
             axe-core is shipped under MPL-2.0\n\
             bad is under GPL-3.0\n\
             other-lgpl is under LGPL-3.0-or-later"
        ),
        "a tooling license shipped, a license allowed nowhere, an exception for its own package only"
    );
    assert_eq!(
        license_gate(
            SHIPPED,
            r#"{"MIT-0":[{"name":"@img/sharp-libvips-x"}],"Apache-2.0":[{"name":"typescript"}]}"#
        ),
        failed("licenses not allowed: @img/sharp-libvips-x is under MIT-0"),
        "an exception for its own license only"
    );
    assert!(
        license_gate("x", TOOLING).is_err_and(|error| error
            .summary
            .starts_with("invalid shipped license report: ")),
        "unreadable"
    );
    assert_eq!(
        license_gate(SHIPPED, "[]"),
        failed("invalid tooling license report: not an object"),
        "not an object"
    );
    for (shipped, all, why) in [
        (
            r#"{"MIT":{"x":{"name":"react"}}}"#,
            TOOLING,
            "invalid shipped license report: MIT does not list packages",
        ),
        (
            r#"{"MIT":[{"version":"1"}]}"#,
            TOOLING,
            "invalid shipped license report: a package under MIT has no name",
        ),
        (
            r#"{"MIT":["react"]}"#,
            TOOLING,
            "invalid shipped license report: a package under MIT has no name",
        ),
        ("{}", TOOLING, "invalid shipped license report: no package"),
        (
            r#"{"MIT":[{"name":"a"}]}"#,
            TOOLING,
            "invalid shipped license report: react, react-dom missing",
        ),
        (
            SHIPPED,
            r#"{"MIT":[{"name":"react"}]}"#,
            "invalid tooling license report: typescript missing",
        ),
    ] {
        assert_eq!(license_gate(shipped, all), failed(why), "{why}");
    }
}

#[test]
fn allows_tooling_every_license_allowed_for_what_ships() {
    for license in crate::SHIPPED_LICENSES {
        assert_eq!(
            license_gate(
                &format!(r#"{{"{license}":[{{"name":"react"}},{{"name":"react-dom"}}]}}"#),
                &format!(
                    r#"{{"{license}":[{{"name":"react"}},{{"name":"react-dom"}},{{"name":"typescript"}}]}}"#
                )
            ),
            Ok(()),
            "{license}: a shipped package is in the full report too"
        );
    }
}

/// Cargo metadata of two crates, `crates/lib` and `apps/app`, under the given licenses, the
/// first depending on the second (by a path, of `kind`) when `depends`.
fn license_metadata(lib: Option<&str>, app: &str, depends: Option<&str>) -> String {
    let license = lib.map_or(String::new(), |license| {
        format!(r#""license":"{license}","#)
    });
    let dependencies = depends.map_or(String::new(), |kind| {
        format!(r#","dependencies":[{{"name":"app","kind":"{kind}","path":"/r/apps/app"}},{{"name":"serde","kind":null}}]"#)
    });
    format!(
        r#"{{"workspace_root":"/r","packages":[{{{license}"name":"lib","manifest_path":"/r/crates/lib/Cargo.toml"{dependencies}}},{{"license":"{app}","name":"app","manifest_path":"/r/apps/app/Cargo.toml","dependencies":[{{"name":"lib","kind":null,"path":"/r/crates/lib"}}]}}]}}"#
    )
}

/// The license files and the license statements as [`LICENSE_TEXTS`] and [`README_LICENSE`]
/// hold them.
fn license_texts() -> Vec<(String, String)> {
    LICENSE_TEXTS
        .iter()
        .copied()
        .chain([("README.md", README_LICENSE)])
        .map(|(path, text)| (path.to_owned(), text.to_owned()))
        .collect()
}

/// A tooling package and an app, under the given licenses, the first depending on the
/// second in `field` (with `version`) when given.
fn license_packages(tool: &str, app: &str, depends: Option<(&str, &str)>) -> Vec<(String, String)> {
    let dependency = depends.map_or(String::new(), |(field, version)| {
        format!(r#", "{field}": {{"@poyetik/app": "{version}"}}"#)
    });
    vec![
        (
            "tooling/tool/package.json".to_owned(),
            format!(r#"{{"name": "@poyetik/tool", "license": "{tool}"{dependency}}}"#),
        ),
        (
            "apps/app/package.json".to_owned(),
            format!(
                r#"{{"name": "@poyetik/app", "license": "{app}", "dependencies": {{"@poyetik/tool": "workspace:*"}}}}"#
            ),
        ),
        (
            "package.json".to_owned(),
            r#"{"name": "poyetik", "license": "MIT OR Apache-2.0"}"#.to_owned(),
        ),
    ]
}

#[test]
fn tells_the_license_of_each_folder() {
    for (dir, license) in [
        (
            "apps/playground",
            "LicenseRef-PolyForm-Perimeter-1.0.1-Exports",
        ),
        ("apps/x/y", "LicenseRef-PolyForm-Perimeter-1.0.1-Exports"),
        ("crates/audio-core", "MIT OR Apache-2.0"),
        ("bindings/audio-wasm", "MIT OR Apache-2.0"),
        ("tooling/config", "MIT OR Apache-2.0"),
        ("spec", "MIT OR Apache-2.0"),
        (".", "MIT OR Apache-2.0"),
        ("apps", "MIT OR Apache-2.0"),
        ("xapps/y", "MIT OR Apache-2.0"),
    ] {
        assert_eq!(crate::license_of(dir), license, "{dir}");
    }
}

#[test]
fn accepts_open_parts_depending_on_open_parts() {
    let metadata = r#"{"workspace_root":"/r","packages":[{"license":"MIT OR Apache-2.0","name":"lib","manifest_path":"/r/crates/lib/Cargo.toml","dependencies":[{"name":"core","kind":null,"path":"/r/crates/core"}]},{"license":"MIT OR Apache-2.0","name":"core","manifest_path":"/r/crates/core/Cargo.toml"}]}"#;
    let packages = vec![
        (
            "tooling/tool/package.json".to_owned(),
            r#"{"name": "@poyetik/tool", "license": "MIT OR Apache-2.0", "devDependencies": {"@poyetik/config": "workspace:*"}}"#.to_owned(),
        ),
        (
            "tooling/config/package.json".to_owned(),
            r#"{"name": "@poyetik/config", "license": "MIT OR Apache-2.0"}"#.to_owned(),
        ),
    ];
    assert_eq!(
        crate::part_license_gate(metadata, &packages, &license_texts()),
        Ok(())
    );
}

#[test]
fn accepts_every_part_under_its_license_and_apps_using_open_parts() {
    let open = "MIT OR Apache-2.0";
    let app = "LicenseRef-PolyForm-Perimeter-1.0.1-Exports";
    assert_eq!(
        crate::part_license_gate(
            &license_metadata(Some(open), app, None),
            &license_packages(open, app, Some(("devDependencies", "^1.0.0"))),
            &license_texts(),
        ),
        Ok(())
    );
}

#[test]
fn refuses_a_part_under_another_license_than_its_folders() {
    let open = "MIT OR Apache-2.0";
    let app = "LicenseRef-PolyForm-Perimeter-1.0.1-Exports";
    assert_eq!(
        crate::part_license_gate(
            &license_metadata(None, open, None),
            &license_packages(app, "MIT", None),
            &license_texts(),
        ),
        step_found(&[
            "crates/lib: the crate lib is under no license, not MIT OR Apache-2.0".to_owned(),
            "apps/app: the crate app is under MIT OR Apache-2.0, not LicenseRef-PolyForm-Perimeter-1.0.1-Exports".to_owned(),
            "tooling/tool/package.json: @poyetik/tool is under LicenseRef-PolyForm-Perimeter-1.0.1-Exports, not MIT OR Apache-2.0"
                .to_owned(),
            "apps/app/package.json: @poyetik/app is under MIT, not LicenseRef-PolyForm-Perimeter-1.0.1-Exports".to_owned(),
        ])
    );
    let unlicensed = vec![(
        "package.json".to_owned(),
        r#"{"name": "poyetik"}"#.to_owned(),
    )];
    assert_eq!(
        crate::part_license_gate(
            &license_metadata(Some(open), app, None),
            &unlicensed,
            &license_texts()
        ),
        step_found(&[
            "./package.json: poyetik is under no license, not MIT OR Apache-2.0".to_owned()
        ])
    );
}

#[test]
fn refuses_an_open_part_depending_on_an_app() {
    let open = "MIT OR Apache-2.0";
    let app = "LicenseRef-PolyForm-Perimeter-1.0.1-Exports";
    for kind in ["normal", "dev", "build"] {
        assert_eq!(
            crate::part_license_gate(
                &license_metadata(Some(open), app, Some(kind)),
                &license_packages(open, app, None),
                &license_texts(),
            ),
            step_found(&["crates/lib: the crate lib, under MIT OR Apache-2.0, depends on apps/app, under LicenseRef-PolyForm-Perimeter-1.0.1-Exports"
                    .to_owned()]
            ),
            "{kind}"
        );
    }
    for field in [
        "dependencies",
        "devDependencies",
        "peerDependencies",
        "optionalDependencies",
    ] {
        assert_eq!(
            crate::part_license_gate(
                &license_metadata(Some(open), app, None),
                &license_packages(open, app, Some((field, "workspace:*"))),
                &license_texts(),
            ),
            step_found(&[format!(
                "tooling/tool/package.json: @poyetik/tool, under MIT OR Apache-2.0, depends on @poyetik/app ({field}), under LicenseRef-PolyForm-Perimeter-1.0.1-Exports"
            )]),
            "{field}"
        );
    }
}

#[test]
fn refuses_a_license_file_missing_or_not_starting_with_its_license() {
    let open = "MIT OR Apache-2.0";
    let app = "LicenseRef-PolyForm-Perimeter-1.0.1-Exports";
    let mut texts = license_texts();
    texts.retain(|(path, _)| path != "LICENSE-MIT");
    if let Some((_, text)) = texts.iter_mut().find(|(path, _)| path == "apps/LICENSE") {
        *text = "Copyright\n# PolyForm Perimeter License 1.0.1\n".to_owned();
    }
    assert_eq!(
        crate::part_license_gate(
            &license_metadata(Some(open), app, None),
            &license_packages(open, app, None),
            &texts,
        ),
        step_found(&[
            "LICENSE-MIT does not start with \"MIT License\"".to_owned(),
            "apps/LICENSE does not start with \"# PolyForm Perimeter License 1.0.1\"".to_owned(),
        ])
    );
}

#[test]
fn refuses_unreadable_manifests_of_the_license_gate() {
    let open = "MIT OR Apache-2.0";
    let app = "LicenseRef-PolyForm-Perimeter-1.0.1-Exports";
    let broken = vec![("apps/app/package.json".to_owned(), "{".to_owned())];
    assert!(
        crate::part_license_gate(
            &license_metadata(Some(open), app, None),
            &broken,
            &license_texts()
        )
        .is_err_and(|failure| failure
            .summary
            .starts_with("apps/app/package.json: invalid package.json")),
        "invalid package.json"
    );
    let no_manifest =
        r#"{"workspace_root":"/r","packages":[{"name":"lib","license":"MIT OR Apache-2.0"}]}"#;
    assert_eq!(
        crate::part_license_gate(no_manifest, &[], &license_texts()),
        Err(Failure::from(
            "cargo metadata gives lib no manifest".to_owned()
        ))
    );
}

#[test]
fn refuses_an_audit_exception_of_a_package_an_app_ships() {
    let shipped = r#"{"MIT": [{"name": "react"}, {"name": "react-dom"}, {"name": "picomatch"}]}"#;
    let tooling = r#"{"MIT": [{"name": "react"}, {"name": "react-dom"}]}"#;
    let register = "# GHSA-fc5q-39xc-vjpf (picomatch): development tools only, removed by \
                    #24 step 152.\n# GHSA-2222-3333-4444 (@scope/glob): development tools \
                    only, removed by #24 step 1.\nauditConfig:\n  ignoreGhsas:\n    - \
                    GHSA-fc5q-39xc-vjpf\n    - GHSA-2222-3333-4444\n";
    assert_eq!(
        crate::licenses::shipped_exceptions_gate(shipped, register),
        Err(Failure::from(
            "pnpm audit's exceptions: `GHSA-fc5q-39xc-vjpf` (picomatch) is ignored by `pnpm \
             audit` in a package an app ships (pnpm-workspace.yaml:1): an exception covers \
             development tools only"
                .to_owned()
        )),
        "a shipped package"
    );
    assert_eq!(
        crate::licenses::shipped_exceptions_gate(tooling, register),
        Ok(()),
        "development tools only"
    );
    let both = r#"{"MIT": [{"name": "react"}, {"name": "react-dom"}, {"name": "picomatch"}],
        "ISC": [{"name": "@scope/glob"}]}"#;
    assert!(
        crate::licenses::shipped_exceptions_gate(both, register).is_err_and(|failure| {
            failure.summary == "pnpm audit's exceptions: 2 problems, listed above"
                && failure.details.len() == 2
                && failure
                    .details
                    .iter()
                    .any(|detail| detail.contains("(@scope/glob)"))
        }),
        "each shipped package, whatever its license"
    );
    assert!(
        crate::licenses::shipped_exceptions_gate("[]", register).is_err_and(|failure| failure
            .summary
            .starts_with("invalid shipped license report")),
        "a report of another shape refused"
    );
}

#[test]
fn refuses_a_license_statement_naming_another_identifier_or_waiving_notices() {
    // A renamed identifier left behind in the prose (finding C7-10 of the third deep review
    // of visual block 2, #34) reads as another license, and the README once said an export
    // ships every part free of any notice (C4-1).
    let open = "MIT OR Apache-2.0";
    let app = "LicenseRef-PolyForm-Perimeter-1.0.1-Exports";
    let gate = |texts: &[(String, String)]| {
        crate::part_license_gate(
            &license_metadata(Some(open), app, None),
            &license_packages(open, app, None),
            texts,
        )
    };
    let mut stale = license_texts();
    for (path, text) in &mut stale {
        if path == "CLAUDE.md" {
            *text = text.replace(app, "LicenseRef-PolyForm-Perimeter-1.0.1");
        }
        if path == "README.md" {
            *text = format!(
                "{}\n## Versioning\n\nthird-party libraries keep their own notices\n",
                text.replace("third-party libraries keep their own notices", "x")
                    .replace("notices and noncompete", "any condition")
            );
        }
    }
    assert_eq!(
        gate(&stale),
        step_found(&[
            "README.md's License section does not say notices and noncompete".to_owned(),
            "README.md's License section does not say third-party libraries keep their own \
             notices"
                .to_owned(),
            "CLAUDE.md does not name `LicenseRef-PolyForm-Perimeter-1.0.1-Exports`".to_owned(),
            "CLAUDE.md names LicenseRef-PolyForm-Perimeter-1.0.1, not the apps' license \
             LicenseRef-PolyForm-Perimeter-1.0.1-Exports"
                .to_owned(),
        ])
    );
    let mut missing = license_texts();
    missing.retain(|(path, _)| path != "README.md");
    for (path, text) in &mut missing {
        if path == "CLAUDE.md" {
            *text = text.replace(open, "MIT");
        }
    }
    assert_eq!(
        gate(&missing),
        step_found(&[
            "README.md is missing".to_owned(),
            "CLAUDE.md does not name `MIT OR Apache-2.0`".to_owned(),
        ])
    );
    let mut unsectioned = license_texts();
    for (path, text) in &mut unsectioned {
        if path == "README.md" {
            *text = text.replace("## License", "## Licenses");
        }
    }
    assert_eq!(
        gate(&unsectioned),
        step_found(
            &crate::licenses::EXPORTS_NOTICES
                .map(|words| format!("README.md's License section does not say {words}"))
        )
    );
}

#[test]
fn reads_each_license_ref_up_to_its_end() {
    assert_eq!(
        crate::licenses::license_refs("`LicenseRef-A-1.0`, LicenseRef-B. (LicenseRef-C-2.0)"),
        ["LicenseRef-A-1.0", "LicenseRef-B", "LicenseRef-C-2.0"]
    );
    assert_eq!(
        crate::licenses::license_refs("LicenseRef-D"),
        ["LicenseRef-D"]
    );
    assert_eq!(
        crate::licenses::license_refs("no identifier"),
        Vec::<&str>::new()
    );
}
