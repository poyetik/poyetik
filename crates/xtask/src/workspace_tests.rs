//! Tests of the workspace as the checks see it: its crates, packages, targets, paths and modes.

use core::num::NonZeroUsize;

use crate::tests::{CHANGES_MODE, FAKE_JOBS};
use crate::{
    Action, Cmd, Dir, Mode, NATIVE_TARGETS, PACKAGES, PORTABILITY_TARGETS, RustCrate, check_steps,
};

#[test]
fn builds_and_lints_for_every_target() {
    assert_eq!(PORTABILITY_TARGETS.len(), 4, "four portability targets");
    assert_eq!(
        NATIVE_TARGETS,
        ["aarch64-apple-darwin", "x86_64-unknown-linux-gnu"],
        "both architectures of the native shell"
    );
}

#[test]
fn runs_half_the_threads_as_mutation_jobs() {
    for (threads, jobs) in [(1, 1), (2, 1), (3, 1), (4, 2), (5, 2), (6, 3), (16, 8)] {
        assert_eq!(
            crate::mutation_jobs(NonZeroUsize::new(threads).expect("not zero")).get(),
            jobs,
            "{threads} threads"
        );
    }
}

#[test]
fn measures_each_crate_by_its_own_unit_tests() {
    let lines: Vec<String> = check_steps(CHANGES_MODE)
        .into_iter()
        .filter_map(|step| {
            if let Action::Coverage(coverage) = step.action {
                Some(coverage.command().line())
            } else {
                None
            }
        })
        .collect();
    let mut expected: Vec<String> = crate::RUST_CRATES
        .iter()
        .map(|krate| {
            format!(
                "cargo +nightly-2026-11-01 llvm-cov --manifest-path {}/Cargo.toml --lib --locked \
                 --branch --json --summary-only --output-path target/coverage/{}.json -- --include-ignored",
                krate.dir, krate.name
            )
        })
        .collect();
    expected.extend(
        ["visual-cli", "audio-cli", "xtask"]
            .map(|name| format!(
                "cargo +nightly-2026-11-01 llvm-cov --manifest-path crates/{name}/Cargo.toml \
                 --locked --branch --json --summary-only --output-path target/coverage/{name}-main.json -- --include-ignored"
            )),
    );
    assert_eq!(
        lines, expected,
        "each library by its unit tests alone, each binary's main.rs by its crate's tests"
    );
}

#[test]
fn empties_stale_build_outputs_before_the_builds_that_write_them() {
    let steps = check_steps(CHANGES_MODE);
    assert_eq!(
        crate::build_outputs(),
        vec![
            "bindings/audio-wasm/dist".to_owned(),
            "bindings/visual-wasm/dist".to_owned()
        ],
        "the outputs of tsc, which never removes a stale one"
    );
    for package in PACKAGES {
        for output in package.outputs {
            let path = format!("{}/{output}", package.dir);
            let emptied = steps.iter().position(
                |step| matches!(&step.action, Action::FreshDirs(paths) if paths.contains(&path)),
            );
            let built = steps
                .iter()
                .position(|step| step.name.starts_with(&format!("{}: ", package.step)));
            assert!(
                emptied.is_some() && emptied < built,
                "{path} is emptied before {} runs",
                package.step
            );
        }
    }
}

#[test]
fn builds_the_engine_package_before_every_package() {
    assert_eq!(
        crate::ENGINE_PACKAGE.dir,
        "bindings/audio-wasm",
        "the engine in the browser"
    );
    assert!(
        crate::WASM_MODULE.starts_with("bindings/audio-wasm/dist/"),
        "its build writes the module"
    );
    for (mode, first_user) in [
        (Mode::Full { jobs: FAKE_JOBS }, PACKAGES[0].step),
        (CHANGES_MODE, "Playground checks"),
    ] {
        let steps = check_steps(mode);
        let built = steps.iter().position(|step| {
            step.action
                == Action::Run(Cmd::new(
                    "pnpm",
                    &["run", "build"],
                    Dir::Package("bindings/audio-wasm"),
                ))
        });
        let emptied = steps
            .iter()
            .position(|step| step.action == Action::FreshDirs(crate::build_outputs()));
        let first = steps
            .iter()
            .position(|step| step.name.starts_with(&format!("{first_user}: ")));
        assert!(
            emptied < built && built < first && emptied.is_some(),
            "{mode:?}: built after its outputs are emptied, before {first_user}"
        );
    }
    assert_eq!(
        PACKAGES[3].needs,
        [crate::Setup::EngineBuild],
        "the playground uses it"
    );
}

#[test]
fn tells_the_files_of_a_crate_by_its_directory() {
    let core = RustCrate {
        name: "audio-core",
        dir: "crates/audio-core",
        binary: false,
        test_data: &[],
        roles: &[],
    };
    for (path, held) in [
        ("crates/audio-core/src/lib.rs", Some("src/lib.rs")),
        ("crates/audio-core/", Some("")),
        ("crates/audio-core", None),
        ("crates/audio-core-x/a.rs", None),
        ("crates/audio-corea.rs", None),
        ("x/crates/audio-core/a.rs", None),
        ("crates/engine-cor/a.rs", None),
    ] {
        assert_eq!(core.holds(path), held, "{path}");
    }
}

#[test]
fn derives_each_list_of_crates_from_their_roles() {
    use crate::CrateRole;
    // Each list a fact written here, so that a role gained or lost is a decision of this
    // test too.
    for (role, crates, what) in [
        (
            CrateRole::Library,
            &[
                "audio-core",
                "audio-boxes",
                "audio-patch",
                "audio-hash",
                "audio-fpenv",
                "audio-wasm",
            ][..],
            "the audio library",
        ),
        (
            CrateRole::LibraryTests,
            &["alloc-counter", "audio-battery", "audio-testkit"][..],
            "the counting allocator, the fingerprint battery and the test kit",
        ),
        (
            CrateRole::VisualLibrary,
            &["visual-core", "visual-boxes", "visual-hash", "visual-wasm"][..],
            "the visual library and its module",
        ),
        (
            CrateRole::VisualTool,
            &["visual-cli"][..],
            "the visual library's command line",
        ),
        (
            CrateRole::VisualLibraryTests,
            &["visual-testkit", "alloc-counter"][..],
            "the visual test kit and the counting allocator",
        ),
        (
            CrateRole::Libm,
            &["audio-core", "audio-testkit"][..],
            "the engine core and the test kit",
        ),
        (
            CrateRole::MathLayer,
            &["audio-core", "audio-boxes"][..],
            "the engine core and the boxes",
        ),
        (
            CrateRole::SharedTests,
            &[
                "visual-testkit",
                "alloc-counter",
                "audio-battery",
                "audio-testkit",
            ][..],
            "the counting allocator, the fingerprint battery and the test kit",
        ),
    ] {
        assert_eq!(crate::crates_in(role), crates, "{what}");
    }
}

/// The crates of the libraries not built for every portability target, and why: each
/// needs what a bare-metal target has not.
const NOT_PORTABLE: [(&str, &str); 3] = [
    (
        "audio-fpenv",
        "sets the floating-point mode of the native CPU it runs on",
    ),
    (
        "audio-wasm",
        "the engine's WebAssembly interface: std, built for WebAssembly by its package",
    ),
    (
        "visual-wasm",
        "the visual library's WebAssembly interface: std, built for WebAssembly by its package",
    ),
];

#[test]
fn builds_every_crate_of_the_libraries_for_every_portability_target_but_its_shells() {
    let libraries: Vec<&str> = crate::LIBRARIES
        .iter()
        .flat_map(|library| crate::crates_in(library.crates))
        .collect();
    let shells: Vec<&str> = NOT_PORTABLE.iter().map(|(name, _)| *name).collect();
    let portable: Vec<&str> = libraries
        .iter()
        .copied()
        .filter(|name| !shells.contains(name))
        .collect();
    assert_eq!(
        crate::PORTABILITY_PACKAGES.to_vec(),
        portable,
        "every crate of a library, but its shells, and nothing else, in the order of the crates"
    );
    for (shell, why) in NOT_PORTABLE {
        assert!(
            libraries.contains(&shell),
            "{shell} ({why}) is a crate of a library"
        );
    }
}
