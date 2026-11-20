//! The runnable hints gate: every hint written in the runnable form, `(see:` and what to run,
//! names a command a user can run as written (`CLAUDE.md`, "A hint is a command"), never a
//! program name that is no command of the repository (`audio-cli help`, where the binary is
//! run through Cargo) nor a document. A remedy written in prose ("restore it from git") is
//! no hint this gate can read: where a fix turned one into a runnable hint, the prose is
//! retired (`retired-patterns.txt`).

use crate::steps::{Failure, step_verdict};

/// The tracked files whose hints the gate reads (git pathspecs): the code of Rust and
/// TypeScript, where every message is written, tests included.
pub const HINT_SOURCES: [&str; 6] = ["*.rs", "*.ts", "*.tsx", "*.mts", "*.js", "*.mjs"];

/// What opens a hint: written in two pieces, so that this file holds no hint of its own.
pub const HINT_OPENING: &str = concat!("(see", ": ");

/// How a runnable hint starts: the commands a user of the repository runs, from its root or
/// from the package the tool belongs to.
pub const RUNNABLE_PREFIXES: [&str; 5] = ["cargo xtask ", "cargo run ", "pnpm ", "git ", "node "];

/// A template interpolation after a hint's opening (`${…}`): its value is a command, which
/// the tests of its package follow.
const INTERPOLATION: &str = "${";

/// The text a hint starting at `rest`, after its opening, names: `rest` itself, or for a
/// Rust string continued by `\` at the end of its line, the next line from its first
/// character that is not a space.
fn hint_text<'text>(rest: &'text str, next: Option<&'text str>) -> &'text str {
    if rest == "\\" {
        next.map_or("", str::trim_start)
    } else {
        rest
    }
}

/// Checks that every hint of `sources` (each with its path, the tracked files of
/// [`HINT_SOURCES`]) names a runnable command.
///
/// A hint is [`HINT_OPENING`] wherever it is in a line, but where a backslash precedes it
/// (a regular expression matching hints); it must go on with one of [`RUNNABLE_PREFIXES`],
/// or with a template interpolation, whose command its package's tests run. A Rust string
/// continued at the end of the line, the opening then a backslash, is read on at the next
/// line.
///
/// # Errors
///
/// Returns every hint that names something else, with its file, its line and what it names.
pub fn runnable_hints_gate(sources: &[(String, String)]) -> Result<(), Failure> {
    let mut problems = Vec::new();
    for (path, text) in sources {
        let lines: Vec<&str> = text.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            for (at, _) in line.match_indices(HINT_OPENING) {
                if line.get(..at).is_some_and(|before| before.ends_with('\\')) {
                    continue;
                }
                let rest = line
                    .get(at.saturating_add(HINT_OPENING.len())..)
                    .unwrap_or("");
                let hint = hint_text(rest, lines.get(index.saturating_add(1)).copied());
                let runnable = hint.starts_with(INTERPOLATION)
                    || RUNNABLE_PREFIXES
                        .iter()
                        .any(|prefix| hint.starts_with(prefix));
                if !runnable {
                    let named: String = hint
                        .chars()
                        .take_while(|&character| character != ')')
                        .collect();
                    problems.push(format!(
                        "{path}:{}: the hint `{HINT_OPENING}{named})` names no command a user can run: \
                         write the exact command, starting with one of {}",
                        index.saturating_add(1),
                        RUNNABLE_PREFIXES.map(str::trim_end).join(", ")
                    ));
                }
            }
        }
    }
    step_verdict(problems)
}

#[cfg(test)]
#[path = "hints_tests.rs"]
mod tests;
