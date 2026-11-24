//! The runnable hints gate: every hint, the runnable form of a remedy (`see`, a colon, a
//! space, then what to run, in parentheses), names a command a user can run as written (`CLAUDE.md`, "A hint is a command"), never a program
//! name that is no command of the repository (`audio-cli help`, where the binary is run
//! through Cargo) nor a document; and it is written in that one form, never in another case
//! or spacing the gate would not read. A remedy written in prose ("restore it from git") is
//! no hint this gate can read: where a fix turned one into a runnable hint, the prose is
//! retired (`retired-patterns.txt`).

use crate::{
    logical_text::{
        Language, Logical, SUBSTITUTION, code_blocks, code_literals, code_prose, line_at, logical,
        markdown_prose,
    },
    steps::{Failure, step_verdict},
    workspace::{Reading, reading_of},
};

/// What opens a hint: written in two pieces, so that this file holds no hint of its own.
pub const HINT_OPENING: &str = concat!("(see", ": ");

/// What opens the hint of a README that names a part's commands, each in code quotes:
/// written in two pieces, as [`HINT_OPENING`].
pub const README_HINT_OPENING: &str = concat!("(commands", ": ");

/// How a runnable hint starts: the commands a user of the repository runs.
///
/// From any directory of the root workspace (`cargo xtask` does not resolve in
/// `crates/visual-gpu`, a workspace of its own), a package's script from its package.
pub const RUNNABLE_PREFIXES: [&str; 6] = [
    "cargo xtask ",
    "cargo run ",
    "cargo update ",
    "pnpm ",
    "git ",
    "node ",
];

/// The words that open a hint, in any case (`see`, the README's `commands`, `command` and
/// `run`): an opening parenthesis, spaces, one of them, spaces and a colon open a hint
/// the gate reads only in its one form.
const OPENING_WORDS: [&str; 4] = ["see", "commands", "command", "run"];

/// Whether `command`, a hint's text after its opening, starts with an interpolation, whose
/// value is a command its package's tests follow: a template substitution
/// ([`SUBSTITUTION`]), a Rust format placeholder (`{…}`), or `${…}` written in a comment.
fn interpolated(command: &str) -> bool {
    command.starts_with(SUBSTITUTION) || command.starts_with('{') || command.starts_with("${")
}

/// `command` as a problem names it: each template substitution written `${…}`.
fn shown(command: &str) -> String {
    command.replace(SUBSTITUTION, "${…}")
}

/// The command of a hint, `rest` the text after its opening: up to the parenthesis that
/// closes it, the parentheses it opens and the interpolations it holds (`{…}`, `${…}`)
/// kept inside, or to the end of its text.
fn hint_command(rest: &str) -> &str {
    let (mut parentheses, mut braces) = (0_usize, 0_usize);
    let end = rest
        .char_indices()
        .find(|&(_, character)| {
            match character {
                '{' => braces = braces.saturating_add(1),
                '}' => braces = braces.saturating_sub(1),
                '(' if braces == 0 => parentheses = parentheses.saturating_add(1),
                ')' if braces == 0 => {
                    if parentheses == 0 {
                        return true;
                    }
                    parentheses = parentheses.saturating_sub(1);
                }
                _ => {}
            }
            false
        })
        .map_or(rest.len(), |(end, _)| end);
    rest.get(..end).unwrap_or(rest)
}

/// The first placeholder `command` holds, if any: an ellipsis, or a word in angle
/// brackets (`<version>`, a letter after the `<`, no space before the `>`), which a shell
/// reads as a redirection.
fn placeholder(command: &str) -> Option<&str> {
    if let Some(at) = command.find('…') {
        return command.get(at..at.saturating_add('…'.len_utf8()));
    }
    command.match_indices('<').find_map(|(at, _)| {
        let after = command.get(at.saturating_add(1)..)?;
        let end = after.find(|character: char| {
            character == '>' || character == '<' || character.is_whitespace()
        })?;
        (after.starts_with(|character: char| character.is_ascii_alphabetic())
            && after.get(end..)?.starts_with('>'))
        .then(|| command.get(at..at.saturating_add(end).saturating_add(2)))
        .flatten()
    })
}

/// The git commands of a hint that take paths, each anchored ([`anchored`]), all a hint may
/// name that change a file or the index: a restore from the last commit, never from the index,
/// which may hold the damage staged; the preflight's, of a file a stopped mutation run left,
/// from the index, which keeps what was staged; and an addition, which loses nothing.
const GIT_WITH_PATHS: [&str; 5] = [
    "git checkout HEAD --",
    "git restore --worktree --",
    "git add --",
    "git add --force --",
    "git add --renormalize --",
];

/// The other git commands of a hint: those that read a path from where they run, whatever its
/// form, started at the repository's root, then reads.
const GIT_OTHERS: [&str; 6] = [
    "git -C \"$(git rev-parse --show-toplevel)\" update-index",
    "git -C \"$(git rev-parse --show-toplevel)\" check-attr",
    "git diff",
    "git log",
    "git rev-parse",
    "git show",
];

/// The commands that write the files only a tool writes, the column "Written by" of
/// `spec/README.md`'s table, each without its directory (a test holds them equal): a hint
/// that blesses names one of them, never a tool's own `bless`.
const BLESS_COMMANDS: [&str; 3] = [
    "cargo xtask bless",
    "cargo xtask bless-benchmarks",
    "cargo run -p visual-cli -- bless",
];

/// What follows `form` in `command`, if it starts with those words.
fn continued<'text>(command: &'text str, form: &str) -> Option<&'text str> {
    command
        .strip_prefix(form)
        .filter(|rest| rest.is_empty() || rest.starts_with(' '))
}

/// Whether `word`, a path a hint names, runs from any directory: absolute, after `root`
/// (git's `:/`, or the repository's root as a shell finds it), quoted for a shell or not, or
/// an interpolation whose value its package's tests follow.
fn anchored(word: &str, root: &str) -> bool {
    let unquoted = word.strip_prefix(['\'', '"']).unwrap_or(word);
    unquoted.starts_with(root) || unquoted.starts_with('/') || interpolated(unquoted)
}

/// The words of `text` as a shell splits them at its spaces, a space inside quotes kept in
/// its word.
fn shell_words(text: &str) -> Vec<&str> {
    let mut quoted = None;
    let mut words = Vec::new();
    let mut start = 0_usize;
    for (at, character) in text.char_indices() {
        if quoted == Some(character) {
            quoted = None;
        } else if quoted.is_none() && ['\'', '"'].contains(&character) {
            quoted = Some(character);
        } else if character == ' ' && quoted.is_none() {
            words.push(text.get(start..at).unwrap_or_default());
            start = at.saturating_add(1);
        }
    }
    words.push(text.get(start..).unwrap_or_default());
    words.retain(|word| !word.is_empty());
    words
}

/// Why `word`, a path `command` names, is refused.
fn relative(word: &str) -> String {
    format!(
        "names `{word}` from where it runs, which fails from another directory: write the path \
         from the repository's root (`:/` for git, `\"$(git rev-parse --show-toplevel)/…\"` for \
         another command) or absolute"
    )
}

/// What is wrong with `command`, a git command of a hint, if anything: one that is none of
/// those a hint may name, so that no hint runs one that can lose work but the restores it
/// names; a path that does not run from any directory, or none in a hint `closed`, whose
/// literal goes on past its paths.
fn git_problem(command: &str, closed: bool) -> Option<String> {
    if !command.starts_with("git ") {
        return None;
    }
    if let Some(paths) = GIT_WITH_PATHS
        .iter()
        .find_map(|form| continued(command, form))
    {
        let words = shell_words(paths);
        return if words.is_empty() {
            closed.then(|| "names no path".to_owned())
        } else {
            words
                .into_iter()
                .find(|word| !anchored(word, ":/"))
                .map(relative)
        };
    }
    let other = GIT_OTHERS
        .iter()
        .any(|form| continued(command, form).is_some());
    (!other).then(|| {
        format!(
            "is no git command a hint may name: write one of {}, then its paths, or one of {}",
            GIT_WITH_PATHS.join(", "),
            GIT_OTHERS.join(", ")
        )
    })
}

/// What is wrong with the paths of `command`, a hint's command other than git's, if anything:
/// a word holding `/` that does not run from any directory ([`anchored`], or a package's name,
/// `@`), but after `pnpm --filter <package> exec`, which runs in the package.
fn path_problem(command: &str) -> Option<String> {
    let words = shell_words(command);
    if command.starts_with("git ")
        || matches!(words.as_slice(), ["pnpm", "--filter", _, "exec", ..])
    {
        return None;
    }
    words
        .into_iter()
        .find(|word| {
            word.contains('/')
                && !word.starts_with('@')
                && !anchored(word, "$(git rev-parse --show-toplevel)/")
        })
        .map(relative)
}

/// What is wrong with the bless of `command`, a hint's command, if anything: one that is none
/// of [`BLESS_COMMANDS`], whole or with its options. A command wholly interpolated is its
/// tool's, whose tests follow it.
fn bless_problem(command: &str) -> Option<String> {
    let blesses = command
        .split(' ')
        .any(|word| word == "bless" || word.starts_with("bless-"));
    let written = BLESS_COMMANDS
        .iter()
        .any(|written| continued(command, written).is_some());
    (blesses && !written).then(|| {
        format!(
            "names a bless that is none of the commands writing a file only a tool writes \
             (spec/README.md): {}",
            BLESS_COMMANDS.join(", ")
        )
    })
}

/// The commands of `text`, a run of string literals, written bare in parentheses: a runnable
/// prefix right after one, which opens no hint the gate reads.
fn bare_commands(text: &str) -> Vec<(usize, &str)> {
    text.match_indices('(')
        .filter(|(at, _)| !text.get(..*at).is_some_and(|before| before.ends_with('$')))
        .filter_map(|(at, _)| {
            let rest = text.get(at.saturating_add(1)..)?;
            RUNNABLE_PREFIXES
                .iter()
                .any(|prefix| rest.starts_with(prefix))
                .then(|| (at, hint_command(rest)))
        })
        .collect()
}

/// The openings of a hint in `text` written otherwise than `accepted`, the one form of its
/// file: an opening parenthesis, spaces, one of [`OPENING_WORDS`] in any case, spaces, then
/// a colon and the character after it; each with its offset, as written up to that
/// character; none where a backslash precedes it (a regular expression matching hints).
fn other_openings<'text>(text: &'text str, accepted: &str) -> Vec<(usize, &'text str)> {
    text.match_indices('(')
        .filter(|(at, _)| !text.get(..*at).is_some_and(|before| before.ends_with('\\')))
        .filter_map(|(at, _)| {
            let after = text.get(at.saturating_add(1)..)?.trim_start_matches(' ');
            let rest = OPENING_WORDS.iter().find_map(|word| {
                after
                    .get(..word.len())
                    .filter(|written| written.eq_ignore_ascii_case(word))
                    .and_then(|_| after.get(word.len()..))
            })?;
            let colon = rest.trim_start_matches(' ').strip_prefix(':')?;
            let next = colon.chars().next().map_or(0, char::len_utf8);
            let end = text.len().saturating_sub(colon.len()).saturating_add(next);
            text.get(at..end)
                .filter(|written| *written != accepted)
                .map(|written| (at, written))
        })
        .collect()
}

/// Whether `path` is a test file of Rust (`*_tests.rs`, or under a `tests/` directory),
/// whose literals quote commands as the messages they check name them.
fn rust_test(path: &str) -> bool {
    path.ends_with("_tests.rs") || path.starts_with("tests/") || path.contains("/tests/")
}

/// The code spans of `text`, a run of string literals, that name a command
/// ([`RUNNABLE_PREFIXES`]): a remedy in code quotes, which the gate cannot read as a hint.
fn quoted_commands(text: &str) -> Vec<(usize, &str)> {
    let mut spans = Vec::new();
    let mut opened = None;
    for (at, character) in text.char_indices() {
        if character != '`' {
            continue;
        }
        match opened.take() {
            None => opened = Some(at.saturating_add(1)),
            Some(start) => {
                let span = text.get(start..at).unwrap_or_default();
                if RUNNABLE_PREFIXES
                    .iter()
                    .any(|prefix| span.starts_with(prefix))
                {
                    spans.push((start, span));
                }
            }
        }
    }
    spans
}

/// Whether `path` is a README's, whose hints open in their own form.
fn readme(path: &str) -> bool {
    path == "README.md" || path.ends_with("/README.md")
}

/// The language of the code at `path`.
fn language_of(path: &str) -> Language {
    if reading_of(path) == Reading::Rust {
        Language::Rust
    } else {
        Language::Script
    }
}

/// The logical texts the hints of the file at `path` are read in ([`crate::logical_text`]),
/// and the one opening of a hint there: a README's paragraphs, whose hints open with
/// [`README_HINT_OPENING`]; the comment paragraphs and string runs of code, a template
/// substitution kept as [`SUBSTITUTION`] ([`code_literals`]), and the markup of an Astro
/// page, read as code blocks, whose hints open with [`HINT_OPENING`].
fn hint_texts(path: &str, text: &str) -> (Vec<Logical>, &'static str) {
    if readme(path) {
        return (markdown_prose(path, text), README_HINT_OPENING);
    }
    let language = language_of(path);
    let mut texts: Vec<Logical> = code_prose(text, language)
        .into_iter()
        .filter(|prose| !prose.literal)
        .map(|prose| prose.text)
        .collect();
    texts.extend(code_literals(text, language));
    let astro = std::path::Path::new(path)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("astro"));
    if astro {
        texts.extend(code_blocks(text).iter().map(|block| logical(block)));
    }
    (texts, HINT_OPENING)
}

/// Checks that every hint of `sources` (each with its path, the tracked files of
/// [`code_sources`](crate::workspace::code_sources), where every message is written, tests
/// included, and every `README.md`) names a runnable command.
///
/// Each file is read whole through the one reader of logical text (`hint_texts`): a hint
/// wrapped over lines, a Rust string continued by `\` or a TypeScript one joined by `+`,
/// reads as one, and its command runs to the parenthesis that closes it.
///
/// In code, a hint is [`HINT_OPENING`] but where a backslash precedes it (a regular
/// expression matching hints); its command starts with one of [`RUNNABLE_PREFIXES`], or
/// with an interpolation, whose command its package's tests run. A README names a part's
/// commands in its own hint, [`README_HINT_OPENING`] then each command in code quotes
/// (`readme_hints`), each starting with one of [`RUNNABLE_PREFIXES`]. A hint opened in
/// another form (another case, a space after the parenthesis or before the colon, none after
/// it, or the other kind's opening, `command` included) is refused, so that no hint escapes
/// the gate by its form; and so is a hint holding a placeholder (`placeholder`), which a
/// user cannot run as written; a git command out of a closed list (`git_problem`), so that no
/// hint loses work or restores from the index; a path read from where the hint runs, which
/// fails from another directory (`path_problem`); and a bless that is none of the commands
/// `spec/README.md` says write the files (`bless_problem`). A command wholly interpolated is
/// read by none of these: its package's tests follow it. A message (a string literal, outside
/// Rust's tests) that names a command in code quotes (`quoted_commands`, Rust's alone) or bare
/// in parentheses (`bare_commands`) is refused too: a remedy is a hint, which the gate reads.
///
/// # Errors
///
/// Returns every hint that names something else, with its file, its line and what it names,
/// and every hint opened otherwise.
pub fn runnable_hints_gate(sources: &[(String, String)]) -> Result<(), Failure> {
    let mut problems = Vec::new();
    for (path, text) in sources {
        let language = language_of(path);
        if !readme(path) && !rust_test(path) {
            for (words, spans) in code_literals(text, language) {
                let quoted = if language == Language::Rust {
                    quoted_commands(&words)
                } else {
                    Vec::new()
                };
                for (commands, how) in [
                    (quoted, "in code quotes"),
                    (bare_commands(&words), "bare in parentheses"),
                ] {
                    for (at, named) in commands {
                        let command = shown(named);
                        problems.push(format!(
                            "{path}:{}: a message names the command `{command}` {how}: write \
                             it as a hint, `{HINT_OPENING}{command})`, which the hints gate reads",
                            line_at(&spans, at)
                        ));
                    }
                }
            }
        }
        let (texts, accepted) = hint_texts(path, text);
        for (words, spans) in &texts {
            let line = |offset| line_at(spans, offset);
            problems.extend(other_openings(words, accepted).into_iter().map(
                |(at, written)| {
                    format!(
                        "{path}:{}: a hint opened `{written}`: write it `{accepted}<command>)`, \
                         the one form the hints gate reads here",
                        line(at)
                    )
                },
            ));
            if accepted == README_HINT_OPENING {
                problems.extend(readme_hints(path, words, &line));
                continue;
            }
            for (at, _) in words.match_indices(HINT_OPENING) {
                if words.get(..at).is_some_and(|before| before.ends_with('\\')) {
                    continue;
                }
                let rest = words
                    .get(at.saturating_add(HINT_OPENING.len())..)
                    .unwrap_or("");
                let command = hint_command(rest);
                let runnable = interpolated(command)
                    || RUNNABLE_PREFIXES
                        .iter()
                        .any(|prefix| command.starts_with(prefix));
                if let Some(written) = placeholder(command) {
                    problems.push(format!(
                        "{path}:{}: the hint `{HINT_OPENING}{})` holds the placeholder \
                         `{written}`: name the exact value, one hint per value",
                        line(at),
                        shown(command),
                    ));
                }
                if runnable {
                    let closed = command.len() < rest.len();
                    for problem in [
                        git_problem(command, closed),
                        path_problem(command),
                        bless_problem(command),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        problems.push(format!(
                            "{path}:{}: the hint `{HINT_OPENING}{})` {problem}",
                            line(at),
                            shown(command),
                        ));
                    }
                } else {
                    problems.push(format!(
                        "{path}:{}: the hint `{HINT_OPENING}{})` names no command a user can run: \
                         write the exact command, starting with one of {}",
                        line(at),
                        shown(command),
                        RUNNABLE_PREFIXES.map(str::trim_end).join(", ")
                    ));
                }
            }
        }
    }
    step_verdict(problems)
}

/// The problems of the hints of `text`, a paragraph of the README at `path` whose lines
/// `line` numbers by offset: each [`README_HINT_OPENING`] goes on with code spans up to the
/// parenthesis that closes it, each one of the part's commands as it is run, starting with
/// one of [`RUNNABLE_PREFIXES`] (`audio-cli help` is a program no shell has;
/// `cargo run -p audio-cli -- help` runs it), and names one at least.
fn readme_hints(path: &str, text: &str, line: &dyn Fn(usize) -> usize) -> Vec<String> {
    let mut problems = Vec::new();
    for (at, _) in text.match_indices(README_HINT_OPENING) {
        let rest = text
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
        let number = line(at);
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
    problems
}

#[cfg(test)]
#[path = "hints_tests.rs"]
mod tests;
