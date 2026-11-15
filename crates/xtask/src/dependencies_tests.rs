//! Tests of the dependency register, the pins of bit-defining dependencies and the audio library's
//! isolation.

use crate::tests::{CHANGES_MODE, METADATA, failed, fixture_metadata, found, passing, step_found};
use crate::{check_steps, run_steps};

#[test]
fn refuses_a_dependency_renamed() {
    let metadata = r#"{"packages":[{"name":"x","dependencies":[{"name":"audio-hash","kind":null,"path":"/r/crates/audio-hash","rename":"core"},{"name":"serde_json","kind":"dev","rename":"json"}]}]}"#;
    assert_eq!(
        crate::dependency_gate(metadata, &[], "x: serde_json (dev) -- reads JSON\n"),
        found(
            "direct dependencies must be reviewed in dependencies.txt and keep their names",
            &[
                "x depends on audio-hash renamed `core`: a dependency keeps its package's \
                 name, so that no name (`core`, `std` …) can stand for another crate"
                    .to_owned(),
                "x depends on serde_json renamed `json`: a dependency keeps its package's \
                 name, so that no name (`core`, `std` …) can stand for another crate"
                    .to_owned(),
            ]
        ),
        "a crate of the workspace and a third-party one, each renamed"
    );
}

#[test]
fn requires_every_direct_dependency_to_be_reviewed() {
    let reviewed = "x: serde_json (normal) -- reads JSON\n./: left-pad (devDependencies) -- old\n";
    let packages = vec![
        (
            "package.json".to_owned(),
            r#"{"devDependencies":{"left-pad":"1"}}"#.to_owned(),
        ),
        (
            "apps/a/package.json".to_owned(),
            r#"{"dependencies":{"@poyetik/audio-wasm":"workspace:*","react":"^19"},"peerDependencies":{"typescript":"~6"}}"#.to_owned(),
        ),
    ];
    let metadata = r#"{"packages":[{"name":"x","dependencies":[{"name":"audio-core","kind":null,"path":"/r","rename":null},{"name":"serde_json","kind":null},{"name":"rand","kind":"dev"}]},{"name":"y"}]}"#;
    assert_eq!(
        crate::dependency_gate(metadata, &packages, reviewed),
        found(
            "direct dependencies must be reviewed in dependencies.txt and keep their names",
            &[
                "not reviewed: .: left-pad (devDependencies)".to_owned(),
                "not reviewed: apps/a: react (dependencies)".to_owned(),
                "not reviewed: apps/a: typescript (peerDependencies)".to_owned(),
                "not reviewed: x: rand (dev)".to_owned(),
                "stale review: ./: left-pad (devDependencies)".to_owned(),
            ]
        ),
        "a new crate, an extra npm key in each field, a review of nothing; parts of the \
         workspace apart"
    );
    assert!(
        crate::dependency_gate("[", &[], "")
            .is_err_and(|failure| failure.summary.starts_with("invalid cargo metadata: ")),
        "unreadable metadata"
    );
    assert_eq!(
        crate::dependency_gate("{}", &[], ""),
        failed("cargo metadata lists no packages"),
        "metadata without packages"
    );
    assert_eq!(
        crate::dependency_gate(r#"{"packages":[{"dependencies":[]}]}"#, &[], ""),
        failed("cargo metadata lists a package without a name"),
        "a package without a name is refused, never read as \"\""
    );
    assert!(
        crate::dependency_gate(
            METADATA,
            &[("p/package.json".to_owned(), "{".to_owned())],
            ""
        )
        .is_err_and(|failure| failure
            .summary
            .starts_with("p/package.json: invalid package.json: ")),
        "an unreadable manifest"
    );
}

#[test]
fn reviews_the_dependencies_real_cargo_metadata_lists() {
    let (metadata, _) = fixture_metadata();
    let reviewed = "audio-core: libm (normal) -- r\naudio-testkit: libm (normal) -- r\n\
                    audio-testkit: sha2 (normal) -- r\naudio-boxes: libm (dev) -- r\n\
                    audio-cli: libm (normal) -- r\naudio-cli: serde (build) -- r\n\
                    audio-hash: serde_json (normal) -- r\naudio-hash: sha2 (normal) -- r\n\
                    visual-hash: serde_json (normal) -- r\nvisual-cli: serde_json (normal) -- r\n";
    assert_eq!(
        crate::dependency_gate(&metadata, &[], reviewed),
        failed(
            "direct dependencies must be reviewed in dependencies.txt and keep their names: audio-cli depends on \
             libm renamed `fast-math`: a dependency keeps its package's name, so that no name \
             (`core`, `std` …) can stand for another crate"
        ),
        "every kind reviewed, the crates of the workspace apart, a renamed one by its name, \
         and refused"
    );
}

/// What the pin gate says of a dependency `key` of the library `what` not reviewed as
/// bit-defining.
fn unmarked(key: &str, what: &str) -> String {
    format!(
        "{key}: its code runs in the {what}'s: review it in dependencies.txt with a \
         reason starting `bit-defining: `"
    )
}

#[test]
fn pins_the_bit_defining_dependencies_real_cargo_metadata_lists() {
    let (metadata, _) = fixture_metadata();
    let reviewed = "# audio-cli: libm (normal) -- bit-defining: in a comment\n\
                    audio-core: libm (normal) -- bit-defining: the math layer\n\
                    audio-testkit: libm (normal) -- the measurements\n\
                    audio-testkit: sha2 (normal) -- bit-defining: the fingerprints\n\
                    audio-hash: serde_json (normal) -- reads JSON\n\
                    audio-hash: sha2 (normal) -- bit-defining: the render hash\n\
                    visual-hash: serde_json (normal) -- reads JSON\n\
                    visual-cli: serde_json (normal) -- reads JSON\n";
    assert_eq!(
        crate::pin_gate(&metadata, reviewed),
        step_found(&[
            unpinned("audio-testkit: sha2 (normal)", "^0.11"),
            unmarked("audio-hash: serde_json (normal)", "audio library"),
            unpinned("audio-hash: serde_json (normal)", "^1"),
            unmarked("visual-hash: serde_json (normal)", "visual library"),
            unpinned("visual-hash: serde_json (normal)", "^1"),
            unmarked("visual-cli: serde_json (normal)", "conformance case reader"),
            unpinned("visual-cli: serde_json (normal)", "^1"),
        ]),
        "a reviewed one unpinned, one of each library and of a case reader neither reviewed \
         so nor pinned; the libraries' tests, another crate's and a comment apart"
    );
}

#[test]
fn reads_one_exact_version_only() {
    let gate = |req: &str| {
        crate::pin_gate(
            &format!(
                r#"{{"packages":[{{"name":"audio-core","dependencies":[{{"name":"libm","kind":"build","req":"{req}"}}]}}]}}"#
            ),
            "audio-core: libm (build) -- bit-defining: the math layer\n",
        )
    };
    assert_eq!(gate("=0.2.16"), Ok(()), "one version, a build dependency");
    assert_eq!(gate("=10.20.300"), Ok(()), "numbers of several digits");
    for req in [
        "0.2.16",
        "^0.2.16",
        ">=0.2.16",
        "=0.2",
        "=0.2.16.1",
        "=0.2.x",
        "=0..16",
        "=.2.16",
        "=0.2.",
        "=0.2.16-rc1",
        "= 0.2.16",
        "",
    ] {
        assert_eq!(
            gate(req),
            step_found(&[unpinned("audio-core: libm (build)", req)]),
            "{req}"
        );
    }
    let missing = crate::pin_gate(
        r#"{"packages":[{"name":"audio-core","dependencies":[{"name":"libm"}]}]}"#,
        "audio-core: libm (normal) -- reads\n",
    );
    assert_eq!(
        missing,
        step_found(&[
            unmarked("audio-core: libm (normal)", "audio library"),
            unpinned("audio-core: libm (normal)", "")
        ]),
        "no kind, no requirement"
    );
    assert_eq!(
        crate::pin_gate(
            r#"{"packages":[{"name":"audio-cli","dependencies":[{"name":"x","req":"1"}]}]}"#,
            "audio-cli: x (normal) --bit-defining: no space\naudio-cli: x (normal)  -- reads\n"
        ),
        Ok(()),
        "a crate outside the library, a reason not starting with the mark"
    );
    assert!(
        crate::pin_gate("not json", "")
            .is_err_and(|failure| failure.summary.starts_with("invalid cargo metadata: ")),
        "the metadata's error"
    );
}

/// What the library gate says of `library` depending on `name` (`kind`, at `path`).
fn outside_library(library: &str, name: &str, kind: &str, path: &str) -> String {
    format!(
        "{library} depends on {name} ({kind}, {path}): the audio library depends only on its \
         own crates (audio-core, audio-boxes, audio-patch, audio-hash, audio-host, \
         audio-fpenv, audio-wasm), its tests also on alloc-counter, audio-battery, audio-testkit; move what \
         it needs into the library, or the dependency out of it"
    )
}

/// What the library gate says of `library`, a crate of the visual library, depending on
/// `name` (`kind`, at `path`).
fn outside_visual_library(library: &str, name: &str, kind: &str, path: &str) -> String {
    format!(
        "{library} depends on {name} ({kind}, {path}): the visual library depends only on its \
         own crates (visual-core, visual-boxes, visual-hash, visual-host, visual-wasm), its tests also on \
         visual-testkit, alloc-counter; move what it needs into the library, or the \
         dependency out of it"
    )
}

/// What the library gate says of `test`, a test crate of the `library` named, its own
/// crates `own` and its test crates `tests`, depending on `name` (`kind`, at `path`).
fn outside_tests(
    test: &str,
    library: &str,
    own: &str,
    tests: &str,
    dependency: (&str, &str, &str),
) -> String {
    let (name, kind, path) = dependency;
    format!(
        "{test}, a crate of the {library}'s tests, depends on {name} ({kind}, {path}): it \
         depends, within the project, only on the {library}'s crates ({own}) and its other \
         test crates ({tests}); move what it needs into them, or the dependency out of it"
    )
}

/// What the library gate says of `tool`, a tool of the visual library, depending on
/// `name` (`kind`, at `path`).
fn outside_visual_tool(tool: &str, name: &str, kind: &str, path: &str) -> String {
    format!(
        "{tool}, a tool of the visual library, depends on {name} ({kind}, {path}): it \
         depends, within the project, only on the visual library's crates (visual-core, \
         visual-boxes, visual-hash, visual-host, visual-wasm), its tests also on visual-testkit, \
         alloc-counter; move what it needs into the library, or the dependency out of it"
    )
}

#[test]
fn keeps_the_library_apart_in_real_cargo_metadata() {
    let (metadata, root) = fixture_metadata();
    let cli = format!("{root}/crates/audio-cli");
    let core = format!("{root}/crates/audio-core");
    let hash = format!("{root}/crates/audio-hash");
    let patch = format!("{root}/crates/audio-patch");
    assert_eq!(
        crate::library_gate(&metadata),
        step_found(&[
            outside_library("audio-boxes", "audio-cli", "normal", &cli),
            outside_library("audio-wasm", "audio-cli", "dev", &cli),
            outside_tests(
                "audio-testkit",
                "audio library",
                "audio-core, audio-boxes, audio-patch, audio-hash, audio-host, audio-fpenv, audio-wasm",
                "alloc-counter, audio-battery, audio-testkit",
                (
                    "visual-core",
                    "normal",
                    &format!("{root}/crates/visual-core")
                ),
            ),
            outside_visual_library("visual-core", "audio-core", "normal", &core),
            outside_visual_library("visual-wasm", "audio-hash", "normal", &hash),
            outside_visual_tool("visual-cli", "audio-patch", "normal", &patch),
            outside_tests(
                "visual-testkit",
                "visual library",
                "visual-core, visual-boxes, visual-hash, visual-host, visual-wasm",
                "visual-testkit, alloc-counter",
                ("audio-core", "normal", &core),
            ),
        ]),
        "the native shell in the code and in the tests; the visual library, its module, its \
         tool and each library's test kit reaching into the other library; each library's test \
         crates in its tests and in each other, its own crates and third-party crates allowed"
    );
}

#[test]
fn refuses_a_library_that_reaches_outside_itself() {
    let packages = |core: &str| {
        let others = r#"{"name":"audio-boxes"},{"name":"audio-patch"},{"name":"audio-hash"},{"name":"audio-host"},{"name":"audio-fpenv"},{"name":"visual-core"},{"name":"visual-boxes"},{"name":"visual-hash"},{"name":"visual-host"},{"name":"visual-wasm"},{"name":"visual-cli"},{"name":"alloc-counter"},{"name":"audio-battery"},{"name":"audio-testkit"},{"name":"visual-testkit"},"#;
        format!(
            r#"{{"workspace_root":"/r","packages":[{others}{{"name":"audio-wasm"}},{{"name":"audio-core","dependencies":[{core}]}}]}}"#
        )
    };
    let dependency = |name: &str, kind: &str, path: &str| {
        format!(r#"{{"name":"{name}","kind":{kind},"path":"{path}"}}"#)
    };
    let gate = |core: &str| crate::library_gate(&packages(core));
    for allowed in [
        dependency("audio-hash", "null", "/r/crates/audio-hash"),
        dependency("audio-wasm", "\"build\"", "/r/bindings/audio-wasm"),
        dependency("audio-testkit", "\"dev\"", "/r/crates/audio-testkit"),
        dependency("alloc-counter", "\"dev\"", "/r/crates/alloc-counter"),
        r#"{"name":"serde","kind":null}"#.to_owned(),
    ] {
        assert_eq!(gate(&allowed), Ok(()), "{allowed}");
    }
    for (name, kind, path, said) in [
        ("audio-testkit", "null", "/r/crates/audio-testkit", "normal"),
        (
            "audio-testkit",
            "\"build\"",
            "/r/crates/audio-testkit",
            "build",
        ),
        ("xtask", "null", "/r/crates/xtask", "normal"),
        ("audio-hash", "null", "/r/apps/audio-hash", "normal"),
        ("audio-hash", "null", "/r/crates/audio-hashx", "normal"),
        (
            "audio-hash",
            "null",
            "/r/vendor/crates/audio-hash",
            "normal",
        ),
        (
            "audio-testkit",
            "\"dev\"",
            "/r/tooling/audio-testkit",
            "dev",
        ),
        ("player", "null", "/r/apps/player", "normal"),
    ] {
        assert_eq!(
            gate(&dependency(name, kind, path)),
            step_found(&[outside_library("audio-core", name, said, path)]),
            "{name} {kind} {path}"
        );
    }
    assert!(
        gate(r#"{"kind":null,"path":"/r/crates/audio-hash"}"#).is_err_and(|failure| {
            failure.summary == "cargo metadata lists a dependency without a name"
        }),
        "a dependency without a name"
    );
    assert_eq!(
        crate::library_gate(r#"{"workspace_root":"/r","packages":[{"name":"audio-core"}]}"#),
        step_found(
            &[
                "audio-boxes",
                "audio-patch",
                "audio-hash",
                "audio-host",
                "audio-fpenv",
                "audio-wasm",
                "alloc-counter",
                "audio-battery",
                "audio-testkit",
                "visual-core",
                "visual-boxes",
                "visual-hash",
                "visual-host",
                "visual-wasm",
                "visual-cli",
                "visual-testkit",
                "alloc-counter"
            ]
            .map(|name| {
                format!("{name}: not in the cargo metadata, so its dependencies cannot be checked")
            })
        ),
        "a crate of the library missing"
    );
    assert!(
        crate::library_gate("{}")
            .is_err_and(|failure| failure.summary == "cargo metadata lists no packages"),
        "the metadata's error"
    );
}

#[test]
fn fails_when_the_crates_cannot_be_listed() {
    let mut fake = passing();
    let _removed = fake
        .files
        .remove(&format!("capture {}", crate::metadata_command().line()));
    let failure =
        run_steps(&check_steps(CHANGES_MODE), &mut fake, &mut Vec::new()).expect_err("stops");
    assert!(
        failure.summary.contains("cannot list the crates"),
        "says what it could not do: {}",
        failure.summary
    );
}

#[test]
fn takes_a_crate_of_the_workspace_by_its_name_and_its_directory() {
    assert_eq!(
        crate::shared_tests_gate(
            r#"{"workspace_root":"/r","packages":[{"name":"audio-boxes","dependencies":[{"name":"audio-hash","kind":"dev","path":"/elsewhere/audio-hash"},{"name":"audio-patch","kind":"dev","path":"/r/vendor/crates/audio-patch"},{"name":"other","kind":"dev","path":"/r/crates/audio-patch"},{"name":"audio-testkit","kind":"dev","path":"/r/crates/audio-testkit"}]},{"name":"audio-testkit","dependencies":[{"name":"alloc-counter","kind":null,"path":"/r/crates/alloc-counter"}]}]}"#
        ),
        Ok(()),
        "a dependency is a crate of the workspace by its name and its directory both"
    );
    assert!(
        crate::shared_tests_gate(r#"{"workspace_root":"/r","packages":[{"name":"x","dependencies":[{"kind":"dev"}]}]}"#)
            .is_err_and(|failure| failure.summary == "cargo metadata lists a dependency without a name"),
        "a dependency without a name"
    );
}

#[test]
fn takes_for_shared_test_code_every_crate_only_tests_use() {
    let (real, _) = fixture_metadata();
    assert_eq!(
        crate::shared_tests_gate(&real),
        Ok(()),
        "the crates real cargo metadata reaches only through tests"
    );
    assert_eq!(crate::shared_tests_gate(METADATA), Ok(()), "the fake's");
    let package = |name: &str, dependencies: &[(&str, &str)]| {
        let listed: Vec<String> = dependencies
            .iter()
            .map(|(dependency, kind)| {
                format!(
                    r#"{{"name":"{dependency}","kind":{kind},"path":"/r/crates/{dependency}"}}"#
                )
            })
            .collect();
        format!(
            r#"{{"name":"{name}","dependencies":[{}]}}"#,
            listed.join(",")
        )
    };
    let metadata = |packages: &[String]| {
        format!(
            r#"{{"workspace_root":"/r","packages":[{}]}}"#,
            packages.join(",")
        )
    };
    assert_eq!(
        crate::shared_tests_gate(&metadata(&[
            package("audio-boxes", &[("audio-testkit", "\"dev\"")]),
            package("audio-core", &[("alloc-counter", "null")]),
            package("audio-testkit", &[("alloc-counter", "null")]),
            package("audio-cli", &[("audio-testkit", "null")]),
        ])),
        Ok(()),
        "shared test code that code depends on too widens more, never less"
    );
    assert_eq!(
        crate::shared_tests_gate(&metadata(&[
            package(
                "audio-boxes",
                &[("audio-testkit", "\"dev\""), ("audio-hash", "\"dev\"")]
            ),
            package("audio-testkit", &[("alloc-counter", "null")]),
        ])),
        failed(
            "audio-hash: only tests reach it, so a change to it can weaken the \
             tests of every crate using it: give it CrateRole::SharedTests"
        ),
        "a crate only tests reach, not taken for shared test code"
    );
    assert_eq!(
        crate::shared_tests_gate(&metadata(&[
            package("audio-boxes", &[("audio-testkit", "\"dev\"")]),
            package("audio-testkit", &[("alloc-counter", "\"build\"")]),
            package("other", &[("audio-testkit", "\"dev\"")]),
        ])),
        Ok(()),
        "through a crate of shared test code, any kind"
    );
    assert_eq!(
        crate::shared_tests_gate(&metadata(&[
            package("audio-boxes", &[("audio-hash", "\"dev\"")]),
            package("audio-hash", &[("audio-patch", "null")]),
            package("audio-wasm", &[("audio-patch", "\"dev\"")]),
        ])),
        step_found(&[
            "audio-hash: only tests reach it, so a change to it can weaken the tests of \
                 every crate using it: give it CrateRole::SharedTests"
                .to_owned(),
            "audio-patch: only tests reach it, so a change to it can weaken the tests of \
                 every crate using it: give it CrateRole::SharedTests"
                .to_owned(),
        ]),
        "reached through a crate only tests use, at any depth"
    );
    assert_eq!(
        crate::shared_tests_gate(&metadata(&[
            package("audio-boxes", &[("audio-hash", "\"dev\"")]),
            package("audio-core", &[("audio-patch", "null")]),
        ])),
        failed(
            "audio-hash: only tests reach it, so a change to it can weaken the \
             tests of every crate using it: give it CrateRole::SharedTests"
        ),
        "code depending on one crate takes no other out of the set"
    );
}

/// What the pin gate says of a dependency `key` required as `req`.
fn unpinned(key: &str, req: &str) -> String {
    format!(
        "{key} is required as `{req}`: pin it to one version, `=X.Y.Z`, in its Cargo.toml, so \
         that an update, which can move bits, is decided alone (spec/engine-design.md, \
         decision 29)"
    )
}
