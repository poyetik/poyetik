//! The WGSL gate: every tracked shader validates with the WGSL proof suite's `wgsl-test`,
//! naga as wgpu runs it, but the invalid fixtures of its tests, each of which must stay
//! refused.

use crate::{
    actions::{list_tracked, read_file},
    runner::{Exit, Failed, Runner},
    steps::{Cmd, Dir, Failure, verdict},
    workspace::ApartWorkspace,
};

#[cfg(test)]
#[path = "wgsl_tests.rs"]
mod tests;

/// The shaders invalid on purpose, fixtures of `wgsl-test`'s own tests, each with where
/// naga's validator refuses it (`<line>:<column>`).
///
/// They prove that it refuses them. [`wgsl_gate`] requires each to be tracked and refused
/// by the validator at that place, so that the list holds no stale entry, and no fixture
/// turned valid, or refused at another stage or place (a typo turning it into a parse
/// error), passes unseen: each proves that the validation runs, not only the parser.
pub const INVALID_SHADERS: [(&str, &str); 1] =
    [("tooling/wgsl/wgsl-test/tests/fixtures/invalid.wgsl", "4:12")];

/// The files of the project the WGSL tools hold a copy of, each copy with its original.
///
/// The suite depends on no other part, and its mutants are tested in copies of its own
/// workspace alone, so its tests cannot read the project's files: they read copies, which
/// [`copies_gate`] keeps identical, so that each fact keeps one source.
pub const COPIES: [(&str, &str); 1] = [(
    "tooling/wgsl/wgsl-test/tests/data/safe-message-vectors.json",
    "spec/safe-message-vectors.json",
)];

/// Checks that each copy of [`COPIES`] holds its original's bytes exactly: `texts` holds
/// the text of each copy and of its original, in the order of [`COPIES`].
///
/// # Errors
///
/// Returns each copy that differs from its original.
pub fn copies_gate(texts: &[(String, String)]) -> Result<(), Failure> {
    let problems = COPIES
        .iter()
        .zip(texts)
        .filter(|(_, (copy, original))| copy != original)
        .map(|((copy, original), _)| format!("{copy} differs from {original}, which it copies"))
        .collect();
    verdict(
        "every copy the WGSL tools hold identical to the project's file",
        problems,
    )
}

/// Reads each copy of [`COPIES`] and its original, and runs [`copies_gate`].
///
/// # Errors
///
/// Returns why a file cannot be read, and each copy that differs from its original.
pub(crate) fn wgsl_copies(runner: &mut dyn Runner) -> Result<(), Failure> {
    let texts = COPIES
        .iter()
        .map(|(copy, original)| Ok((read_file(runner, copy)?, read_file(runner, original)?)))
        .collect::<Result<Vec<_>, Failure>>()?;
    copies_gate(&texts)
}

/// The command validating `files` with `wgsl-test`, built from the WGSL tools workspace at
/// its locked versions.
#[must_use]
pub fn wgsl_command(files: &[&str]) -> Cmd {
    let mut args = vec!["run"];
    args.extend(ApartWorkspace::Wgsl.manifest_args());
    args.extend([
        "--package",
        "wgsl-test",
        "--locked",
        "--quiet",
        "--",
        "validate",
    ]);
    args.extend(files);
    Cmd::new("cargo", &args, Dir::Root)
}

/// Checks every tracked WGSL shader (`*.wgsl`).
///
/// The shaders but [`INVALID_SHADERS`] are validated together ([`wgsl_command`]), each
/// problem said on its own line by `wgsl-test`; each of [`INVALID_SHADERS`] is validated
/// alone, and must be refused as `wgsl-test` refuses a shader: exit code 1, its one
/// problem line naming it, naga's validator and the place listed.
///
/// It sees shaders as files only: a shader written as a Rust string literal is no `.wgsl`
/// file, and escapes it. The project keeps each shader in a file of its crate's `src/`,
/// which a build loads (`include_str!`) and the loaded-files gate requires tracked.
///
/// # Errors
///
/// Returns why the shaders cannot be listed, the failure of the validation of the valid
/// ones, and each invalid fixture untracked, accepted or refused otherwise.
pub fn wgsl_gate(runner: &mut dyn Runner) -> Result<(), Failure> {
    let tracked = list_tracked(runner, &["*.wgsl"], "the WGSL shaders")?;
    let valid: Vec<&str> = tracked
        .iter()
        .map(String::as_str)
        .filter(|&shader| {
            !INVALID_SHADERS
                .iter()
                .any(|&(invalid, _)| invalid == shader)
        })
        .collect();
    let mut problems = Vec::new();
    if !valid.is_empty()
        && let Err(failure) = runner.run(&wgsl_command(&valid))
    {
        problems.push(failure.to_string());
    }
    for (shader, at) in INVALID_SHADERS {
        if !tracked.iter().any(|path| path == shader) {
            problems.push(format!(
                "{shader}: listed among the invalid shaders (INVALID_SHADERS) but not tracked"
            ));
            continue;
        }
        let ran = runner.run_for_stdout(&wgsl_command(&[shader]));
        let said = ran.stdout.starts_with(&format!("{shader}:{at}: invalid: "))
            && ran.stdout.lines().count() == 1;
        match ran.ended {
            Err(failure)
                if said
                    && matches!(
                        failure.failed,
                        Failed::Ended {
                            exit: Exit::Code(1_i32),
                            ..
                        }
                    ) => {}
            Err(failure) => problems.push(format!(
                "{shader}: an invalid shader not refused as wgsl-test refuses one: {failure}"
            )),
            Ok(()) => problems.push(format!(
                "{shader}: listed among the invalid shaders (INVALID_SHADERS), but valid"
            )),
        }
    }
    verdict(
        "every WGSL shader valid, the invalid fixtures refused",
        problems,
    )
}
