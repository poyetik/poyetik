//! The exact errors gate: a test asserts the whole of an error it expects, never only that
//! there is one, or how it starts, so that another error never passes for it.

use crate::{
    rust_source::{RustSource, Token, close_of},
    steps::{Failure, step_verdict},
    test_code::is_test_code,
};

/// The calls that tell only that a result is an error, or let a closure judge it by a part.
const VAGUE: [&str; 2] = ["is_err", "is_err_and"];

/// The calls that take an error out of a result.
const TAKEN: [&str; 2] = ["expect_err", "unwrap_err"];

/// The calls that compare a part of a text.
const PARTIAL: [&str; 3] = ["starts_with", "ends_with", "contains"];

/// The functions of test code that judge an error by a part of it, each with its file and why
/// its whole text cannot be asserted there.
///
/// Its words are another program's, which a version of it decides, or the rest of the error is
/// asserted beside.
pub const INEXACT_ERRORS: [(&str, &str, &str); 11] = [
    (
        "crates/audio-cli/src/lib_tests.rs",
        "check_names_no_refused_command_while_the_battery_differs",
        "each case fails with its own count of problems: the lines said, with no refused \
         command, are asserted beside",
    ),
    (
        "crates/audio-host/src/engine_tests.rs",
        "tells_a_lookahead_pending_while_it_limits_then_taken_at_the_rest_after",
        "the refusal above the maximum is one step of a run; its whole text is asserted by \
         `changes_its_limiter_live_and_refuses_without_one` in the same file",
    ),
    (
        "crates/visual-host/src/view_tests.rs",
        "refuses_a_square_without_the_heap_and_keeps_every_message_whole",
        "the result is taken while the allocations are counted; the message is compared whole \
         beside, from the same refusal",
    ),
    (
        "crates/visual-testkit/src/laws_tests.rs",
        "refuses_a_box_wrong_along_any_input_of_the_contract",
        "the law that refuses each fault is asserted by its name; the case of the matrix it \
         reports after it depends on the sampled cases",
    ),
    (
        "crates/xtask/src/cli_tests.rs",
        "reads_the_record_real_git_holds_at_a_commit",
        "git's own words, which its version decides",
    ),
    (
        "crates/xtask/src/cli_tests.rs",
        "resolves_a_revision_and_proposes_the_upstream_with_real_git",
        "git's own words, which its version decides",
    ),
    (
        "crates/xtask/src/lib_tests.rs",
        "reports",
        "the helper that looks for one problem among those of a gate",
    ),
    (
        "crates/xtask/src/runner_tests.rs",
        "system_runner_lists_the_tracked_typescript_files",
        "git's own words, which its version decides",
    ),
    (
        "crates/xtask/src/runner_tests.rs",
        "system_runner_captures_output_and_failures",
        "git's own words, which its version decides",
    ),
    (
        "crates/xtask/src/steps_tests.rs",
        "rustdoc_checks_the_error_code_of_the_fixture_under_the_variable",
        "the command line holds the scratch directory; what rustdoc refused is asserted beside",
    ),
    (
        "crates/xtask/src/workflow_tests.rs",
        "refuses_every_workflow_but_xtask_s",
        "each probe is refused at the line its edit made differ; what the gate says of each \
         rule is asserted whole by the tests beside",
    ),
];

/// Requires the test code of every crate (`tests/` and `*_tests.rs` files) to assert an error
/// whole.
///
/// It is compared, never only tested for (`is_err`), judged by a closure (`is_err_and`), nor
/// taken (`expect_err`, `unwrap_err`) then compared by a part in the same chain of calls
/// (`starts_with`, `ends_with`, `contains`), but in the functions of [`INEXACT_ERRORS`].
///
/// **Review only**: an error held in a variable, then compared by a part; a pattern that
/// binds nothing (`Err(_)`).
///
/// # Errors
///
/// Returns every such call with its file, its line and its function, and every reviewed
/// function that makes none, in a directory whose test code is read.
pub fn exact_errors_gate(sources: &[RustSource]) -> Result<(), Failure> {
    reviewed_errors_gate(sources, &INEXACT_ERRORS)
}

/// [`exact_errors_gate`] with `reviewed` for its reviewed functions, so that its tests can
/// review functions of their own.
pub(crate) fn reviewed_errors_gate(
    sources: &[RustSource],
    reviewed: &[(&str, &str, &str)],
) -> Result<(), Failure> {
    let mut problems = Vec::new();
    let mut used = vec![false; reviewed.len()];
    for source in sources {
        let path = source.path();
        if !is_test_code(source) {
            continue;
        }
        let mut function = "";
        for (at, token) in source.tokens.iter().enumerate() {
            if token.is_ident("fn") {
                function = source
                    .tokens
                    .get(at.saturating_add(1))
                    .map_or("", |name| name.text.as_str());
            }
            let Some(call) = vague_call(&source.tokens, at) else {
                continue;
            };
            let entry = reviewed
                .iter()
                .position(|&(file, name, _)| file == path && name == function);
            match entry.and_then(|index| used.get_mut(index)) {
                Some(use_of) => *use_of = true,
                None => problems.push(format!(
                    "{path}:{}: `{function}` judges an error by `{call}`: compare it whole, or \
                     review the function in INEXACT_ERRORS with why its text cannot be known \
                     (crates/xtask/src/exact_errors.rs)",
                    token.line
                )),
            }
        }
    }
    // Only of the directories whose test code is read, so that the tests that read a file or
    // two need none of the reviewed functions.
    let dir = |path: &str| path.rsplit_once('/').map(|(dir, _)| dir.to_owned());
    let read: Vec<_> = sources
        .iter()
        .filter(|source| is_test_code(source))
        .map(|source| dir(source.path()))
        .collect();
    problems.extend(
        reviewed
            .iter()
            .zip(used)
            .filter(|&(&(file, _, _), used)| !used && read.contains(&dir(file)))
            .map(|(&(file, name, _), _)| {
                format!(
                    "{file}: `{name}`, reviewed in INEXACT_ERRORS, judges no error by a part: \
                     remove it (crates/xtask/src/exact_errors.rs)"
                )
            }),
    );
    step_verdict(problems)
}

/// The call of `tokens` at `at` that judges an error by a part, if it is one: a method of
/// [`VAGUE`], or a method of [`PARTIAL`] in the chain that follows a method of [`TAKEN`].
fn vague_call(tokens: &[Token], at: usize) -> Option<String> {
    let called = tokens.get(at)?;
    let method = at
        .checked_sub(1)
        .and_then(|before| tokens.get(before))
        .is_some_and(|before| before.is_punct('.'));
    if !method {
        return None;
    }
    if VAGUE.iter().any(|name| called.is_ident(name)) {
        return Some(called.text.clone());
    }
    let taken = TAKEN.iter().find(|name| called.is_ident(name))?;
    let mut next = at.saturating_add(1);
    loop {
        match tokens.get(next..).unwrap_or_default() {
            [open, ..] if open.opens('(') => next = close_of(tokens, next).saturating_add(1),
            [dot, name, ..] if dot.is_punct('.') => {
                if let Some(partial) = PARTIAL.iter().find(|word| name.is_ident(word)) {
                    return Some(format!("{taken}(…).{partial}"));
                }
                next = next.saturating_add(2);
            }
            _ => return None,
        }
    }
}

#[cfg(test)]
#[path = "exact_errors_tests.rs"]
mod tests;
