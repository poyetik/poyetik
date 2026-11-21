//! The runnable hints gate: every hint, the runnable form of a remedy (`see`, a colon, a
//! space, then what to run, in parentheses), names a command a user can run as written (`CLAUDE.md`, "A hint is a command"), never a program
//! name that is no command of the repository (`audio-cli help`, where the binary is run
//! through Cargo) nor a document; and it is written in that one form, never in another case
//! or spacing the gate would not read. A remedy written in prose ("restore it from git") is
//! no hint this gate can read: where a fix turned one into a runnable hint, the prose is
//! retired (`retired-patterns.txt`).

use crate::steps::{Failure, step_verdict};

/// What opens a hint: written in two pieces, so that this file holds no hint of its own.
pub const HINT_OPENING: &str = concat!("(see", ": ");

/// What opens the hint of a README that names a part's commands, each in code quotes:
/// written in two pieces, as [`HINT_OPENING`].
pub const README_HINT_OPENING: &str = concat!("(commands", ": ");

/// How a runnable hint starts: the commands a user of the repository runs, from its root or
/// from the package the tool belongs to.
pub const RUNNABLE_PREFIXES: [&str; 6] = [
    "cargo xtask ",
    "cargo run ",
    "cargo update ",
    "pnpm ",
    "git ",
    "node ",
];

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

/// The openings of a hint in `line` written otherwise than [`HINT_OPENING`]: an opening
/// parenthesis, the word `see` in any case, spaces, then a colon and anything but one
/// space (a capital, a space before the colon, none after it), each as written up to its
/// colon and the character after it; none where a backslash precedes it (a regular
/// expression matching hints).
fn other_openings(line: &str) -> Vec<&str> {
    line.match_indices('(')
        .filter(|(at, _)| !line.get(..*at).is_some_and(|before| before.ends_with('\\')))
        .filter_map(|(at, _)| {
            let after = line.get(at.saturating_add(1)..)?;
            let rest = after
                .get(..3)
                .filter(|word| word.eq_ignore_ascii_case("see"))
                .and_then(|_| after.get(3..))?;
            let colon = rest.trim_start_matches(' ').strip_prefix(':')?;
            let next = colon.chars().next().map_or(0, char::len_utf8);
            let end = line.len().saturating_sub(colon.len()).saturating_add(next);
            line.get(at..end)
                .filter(|written| !written.starts_with(HINT_OPENING))
        })
        .collect()
}

/// Checks that every hint of `sources` (each with its path, the tracked files of
/// [`CODE_SOURCES`](crate::workspace::CODE_SOURCES), where every message is written, tests
/// included, and every `README.md`) names a runnable command.
///
/// A README names a part's commands in its own hint, [`README_HINT_OPENING`] then each
/// command in code quotes (`readme_hints`): each starts with one of [`RUNNABLE_PREFIXES`].
/// A README holds no message, so the hints of code are not read there.
///
/// A hint is [`HINT_OPENING`] wherever it is in a line, but where a backslash precedes it
/// (a regular expression matching hints); it must go on with one of [`RUNNABLE_PREFIXES`],
/// or with a template interpolation, whose command its package's tests run. A Rust string
/// continued at the end of the line, the opening then a backslash, is read on at the next
/// line. A hint opened in another case or spacing (a capital, a space before the colon, none
/// after it) is refused, so that no hint escapes the gate by its form.
///
/// # Errors
///
/// Returns every hint that names something else, with its file, its line and what it names,
/// and every hint opened otherwise.
pub fn runnable_hints_gate(sources: &[(String, String)]) -> Result<(), Failure> {
    let mut problems = Vec::new();
    for (path, text) in sources {
        if path == "README.md" || path.ends_with("/README.md") {
            problems.extend(readme_hints(path, text));
            continue;
        }
        let lines: Vec<&str> = text.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            problems.extend(other_openings(line).into_iter().map(|written| {
                format!(
                    "{path}:{}: a hint opened `{written}`: write it `{HINT_OPENING}<command>)`, \
                     the one form the hints gate reads",
                    index.saturating_add(1)
                )
            }));
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

/// The problems of the hints of a README, `text` at `path`: each [`README_HINT_OPENING`]
/// goes on with code spans up to the parenthesis that closes it, each one of the part's
/// commands as it is run, starting with one of [`RUNNABLE_PREFIXES`] (`audio-cli help` is
/// a program no shell has; `cargo run -p audio-cli -- help` runs it), and names one at
/// least.
fn readme_hints(path: &str, text: &str) -> Vec<String> {
    let mut problems = Vec::new();
    for (index, line) in text.lines().enumerate() {
        for (at, _) in line.match_indices(README_HINT_OPENING) {
            let rest = line
                .get(at.saturating_add(README_HINT_OPENING.len())..)
                .unwrap_or("");
            let mut quoted = false;
            let end = rest
                .char_indices()
                .find(|&(_, character)| {
                    quoted ^= character == '`';
                    character == ')' && !quoted
                })
                .map_or(rest.len(), |(end, _)| end);
            let hint = rest.get(..end).unwrap_or(rest);
            let commands: Vec<&str> = hint.split('`').skip(1).step_by(2).collect();
            let unrunnable = commands.iter().filter(|command| {
                !RUNNABLE_PREFIXES
                    .iter()
                    .any(|prefix| command.starts_with(prefix))
            });
            let number = index.saturating_add(1);
            problems.extend(unrunnable.map(|command| {
                format!(
                    "{path}:{number}: the command `{command}` of a README's hint is no command a \
                     user can run: write it as it is run, starting with one of {}",
                    RUNNABLE_PREFIXES.map(str::trim_end).join(", ")
                )
            }));
            if commands.is_empty() {
                problems.push(format!(
                    "{path}:{number}: a README's hint `{README_HINT_OPENING}{hint})` names no \
                     command: name each in code quotes"
                ));
            }
        }
    }
    problems
}

#[cfg(test)]
#[path = "hints_tests.rs"]
mod tests;
