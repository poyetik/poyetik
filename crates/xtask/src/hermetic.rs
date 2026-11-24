//! The hermetic git gate: every place that starts a process is listed, and the lists of git's
//! variables every copy holds are xtask's, so that no test reads the machine's git configuration
//! or an enclosing git process's variables.

use core::ops::Range;

use crate::{
    logical_text::prose_texts,
    runner::{HERMETIC_GIT, LOCAL_GIT_VARIABLES},
    rust_source::{RustSource, Token, TokenKind, close_of},
    steps::{Failure, step_verdict},
};

/// What a place that starts a process ([`PROCESS_STARTS`], [`SCRIPT_STARTS`]) may start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartKind {
    /// Git, or a shell that runs it, in a test's directory: it sets the variables of
    /// [`HERMETIC_GIT`] and removes those of [`LOCAL_GIT_VARIABLES`].
    HermeticGit,
    /// Git on the repository itself, which keeps the user's configuration
    /// (`safe.directory`, for one) and removes the variables of [`LOCAL_GIT_VARIABLES`].
    Repository,
    /// No git.
    NotGit,
}

/// The functions of Rust code that start a process (a file, a function), each with what it
/// may start and why, the binary and the test program of a crate's own tests apart.
pub const PROCESS_STARTS: [(&str, &str, StartKind, &str); 8] = [
    (
        "crates/audio-cli/tests/binary.rs",
        "shell",
        StartKind::HermeticGit,
        "the shell that follows a restore hint, a git command",
    ),
    (
        "crates/visual-cli/tests/binary.rs",
        "shell",
        StartKind::HermeticGit,
        "the shell that follows a restore hint, a git command",
    ),
    (
        "crates/visual-gpu/src/bench.rs",
        "system",
        StartKind::NotGit,
        "the program that names the operating system's version",
    ),
    (
        "crates/visual-gpu/tests/binary.rs",
        "shell",
        StartKind::HermeticGit,
        "the shell that follows a restore hint, a git command",
    ),
    (
        "crates/xtask/src/lib_tests.rs",
        "hermetic_git_command",
        StartKind::HermeticGit,
        "the git of xtask's unit tests",
    ),
    (
        "crates/xtask/src/runner.rs",
        "command",
        StartKind::Repository,
        "every command of the checks, whose tests give it their git's variables",
    ),
    (
        "crates/xtask/src/steps_tests.rs",
        "the_mutant_runner_limits_what_it_starts_on_linux",
        StartKind::NotGit,
        "the program that limits a mutant's memory, and a shell's `ulimit`",
    ),
    (
        "crates/xtask/tests/binary.rs",
        "git_command",
        StartKind::HermeticGit,
        "the git of the hooks' tests",
    ),
];

/// The TypeScript and JavaScript files that start a process, each with what it may start
/// and why.
pub const SCRIPT_STARTS: [(&str, StartKind, &str); 9] = [
    (
        "apps/roadmap/test/bin.test.ts",
        StartKind::NotGit,
        "the roadmap's command",
    ),
    (
        "bindings/audio-wasm/test/benchmarks/cli.test.ts",
        StartKind::NotGit,
        "the benchmarks' command",
    ),
    (
        "bindings/audio-wasm/test/scripts.test.ts",
        StartKind::NotGit,
        "the package's scripts",
    ),
    (
        "bindings/visual-wasm/test/scripts.test.ts",
        StartKind::NotGit,
        "the package's scripts",
    ),
    (
        "tooling/config/test/linted.test.ts",
        StartKind::Repository,
        "the files the repository tracks, and a scratch repository of another owner",
    ),
    (
        "tooling/oracle/test/bin.test.ts",
        StartKind::NotGit,
        "the oracle's command",
    ),
    (
        "tooling/wasm-build/src/io.ts",
        StartKind::NotGit,
        "cargo and the tools of the WebAssembly build",
    ),
    (
        "tooling/wasm-build/src/testing/scripts.ts",
        StartKind::NotGit,
        "a package's scripts, for their tests",
    ),
    (
        "tooling/wasm-build/test/bench.test.ts",
        StartKind::HermeticGit,
        "the git that follows a restore hint",
    ),
];

/// The file that holds [`PROCESS_STARTS`] and [`SCRIPT_STARTS`]: in the tree that holds it,
/// every one of their rows is current.
pub const STARTS_FILE: &str = "crates/xtask/src/hermetic.rs";

/// The Node.js module whose functions start a process.
const SCRIPT_MODULE: &str = "child_process";

/// The directory of the crate that declares both lists of git's variables, whose code names
/// them without a copy.
const LISTS_CRATE: &str = "crates/xtask/";

/// Checks every place that starts a process and every copy of git's variables.
///
/// `sources` are the tracked code files (path and text), and the gate denies by default:
/// - a Rust function that names `Command::new` (its tokens, so neither a comment nor a
///   string is read) is a row of [`PROCESS_STARTS`], but one that starts the binary
///   (`env!("CARGO_BIN_EXE_…")`) or the test program (`std::env::current_exe()`) of its own
///   crate; a TypeScript or JavaScript file that holds the name `child_process` is a row of
///   [`SCRIPT_STARTS`]; no row is stale in the tree that holds the tables ([`STARTS_FILE`]);
///   and `Command` is never renamed (`Command as`);
/// - the function (the file, for a script) of a row names what its kind requires
///   ([`StartKind`]): its file's own copy of the lists, or xtask's in xtask's crate;
/// - every declaration of `HERMETIC_GIT` or `LOCAL_GIT_VARIABLES`, whatever its file starts,
///   holds xtask's entries ([`HERMETIC_GIT`], [`LOCAL_GIT_VARIABLES`]) in their order: an
///   array of literals in Rust, and in a script the one form the formatter writes (`script_list`).
///
/// Not seen: a process started by a dependency or by another program the tests start, or by a
/// package script; a module name a script computes; git given to a row of
/// [`StartKind::NotGit`]; a variable set after the lists in a row's function.
///
/// # Errors
///
/// Returns every such place and copy, naming its file, and every stale row, or why a Rust
/// file does not lex.
pub fn hermetic_git_gate(sources: &[(String, String)]) -> Result<(), Failure> {
    let (mut problems, mut held) = (Vec::new(), Vec::new());
    for (path, text) in sources {
        if std::path::Path::new(path)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("rs"))
        {
            // Only a file holding one of these words holds a token the rules read.
            if ["Command", "HERMETIC_GIT", "LOCAL_GIT_VARIABLES"]
                .iter()
                .any(|word| text.contains(word))
            {
                let source = RustSource::parse(path, text)?;
                rust_starts(&source, &mut problems, &mut held);
                problems.extend(rust_lists(&source));
            }
        } else {
            script_starts(path, text, &mut problems, &mut held);
        }
    }
    if sources.iter().any(|(path, _)| path == STARTS_FILE) {
        let rows = PROCESS_STARTS
            .iter()
            .map(|&(path, function, _, _)| (path, function))
            .chain(SCRIPT_STARTS.iter().map(|&(path, _, _)| (path, "")));
        problems.extend(
            rows.filter(|&(path, function)| {
                !held
                    .iter()
                    .any(|(file, name)| file == path && name == function)
            })
            .map(|(path, function)| {
                let place = if function.is_empty() {
                    path.to_owned()
                } else {
                    format!("{path}: `{function}`")
                };
                format!("{place}: starts no process any more: remove its row ({STARTS_FILE})")
            }),
        );
    }
    step_verdict(problems)
}

/// What a start of `kind` lacks, given whether its code names the two lists, said as a
/// problem of it.
fn lacking(kind: StartKind, hermetic: bool, local: bool) -> Option<String> {
    let lacks: Vec<&str> = [
        (
            kind == StartKind::HermeticGit && !hermetic,
            "sets no variable of HERMETIC_GIT",
        ),
        (
            kind != StartKind::NotGit && !local,
            "removes no variable of LOCAL_GIT_VARIABLES",
        ),
    ]
    .into_iter()
    .filter_map(|(lacks, what)| lacks.then_some(what))
    .collect();
    (!lacks.is_empty()).then(|| {
        format!(
            "{}: name both lists, the file's own copy or xtask's in xtask \
             (crates/xtask/src/runner.rs), so that the git it starts reads neither the \
             machine's configuration nor an enclosing git process's",
            lacks.join(" and ")
        )
    })
}

/// The functions of `tokens`, each with its name and the range of its body, the outer before
/// those inside it.
fn functions(tokens: &[Token]) -> Vec<(&str, Range<usize>)> {
    tokens
        .iter()
        .zip(tokens.iter().skip(1))
        .enumerate()
        .filter(|(_, (keyword, name))| keyword.is_ident("fn") && name.kind == TokenKind::Ident)
        .filter_map(|(at, (_, name))| {
            let mut depth = 0_usize;
            // The first brace, or the semicolon of a declaration, outside every group.
            let (open, _) = tokens.iter().enumerate().skip(at).find(|(_, token)| {
                let found = depth == 0 && (token.opens('{') || token.is_punct(';'));
                match token.kind {
                    TokenKind::Open => depth = depth.saturating_add(1),
                    TokenKind::Close => depth = depth.saturating_sub(1),
                    TokenKind::Ident
                    | TokenKind::Punct { .. }
                    | TokenKind::Literal
                    | TokenKind::Doc { .. } => {}
                }
                found
            })?;
            Some((name.text.as_str(), open..close_of(tokens, open)))
        })
        .collect()
}

/// Whether the tokens after `Command::new` start the binary or the test program of the
/// crate that holds them.
fn starts_itself(after: &[Token]) -> bool {
    let text: String = after
        .iter()
        .take(12)
        .map(|token| token.text.as_str())
        .collect();
    text.starts_with("(env!(\"CARGO_BIN_EXE_") || text.starts_with("(std::env::current_exe()")
}

/// Adds to `problems` those of the processes `source` starts, and to `held` the functions
/// that start one, each with its file.
fn rust_starts(source: &RustSource, problems: &mut Vec<String>, held: &mut Vec<(String, String)>) {
    let (path, tokens) = (source.path(), source.tokens.as_slice());
    let functions = functions(tokens);
    let declares = |list: &str| path.starts_with(LISTS_CRATE) || !lists_of(tokens, list).is_empty();
    for (at, token) in tokens
        .iter()
        .enumerate()
        .filter(|(_, token)| token.is_ident("Command"))
    {
        let after = tokens.get(at.saturating_add(1)..).unwrap_or_default();
        let line = token.line;
        if after.first().is_some_and(|next| next.is_ident("as")) {
            problems.push(format!(
                "{path}:{line}: renames `Command`, so that the processes it starts are seen by \
                 no gate: name it as it is"
            ));
        }
        let [first, second, new, rest @ ..] = after else {
            continue;
        };
        if !(first.is_punct(':') && second.is_punct(':') && new.is_ident("new"))
            || starts_itself(rest)
        {
            continue;
        }
        let (function, body) = functions
            .iter()
            .rfind(|(_, body)| body.contains(&at))
            .map_or(("", 0..0), |(name, body)| (*name, body.clone()));
        held.push((path.to_owned(), function.to_owned()));
        let row = PROCESS_STARTS
            .iter()
            .find(|&&(file, name, _, _)| file == path && name == function);
        let Some(&(_, _, kind, _)) = row else {
            problems.push(format!(
                "{path}:{line}: `{function}` starts a process and is no row of PROCESS_STARTS: \
                 add it, with what it may start and why ({STARTS_FILE})"
            ));
            continue;
        };
        let names = |list: &str| {
            declares(list)
                && tokens
                    .get(body.clone())
                    .unwrap_or_default()
                    .iter()
                    .any(|word| word.is_ident(list))
        };
        if let Some(lacks) = lacking(kind, names("HERMETIC_GIT"), names("LOCAL_GIT_VARIABLES")) {
            problems.push(format!("{path}:{line}: `{function}` {lacks}"));
        }
    }
}

/// The indices in `tokens` of the name of every declaration of `list` (`const` or
/// `static`).
fn lists_of(tokens: &[Token], list: &str) -> Vec<usize> {
    tokens
        .iter()
        .zip(tokens.iter().skip(1))
        .enumerate()
        .filter(|(_, (keyword, name))| {
            (keyword.is_ident("const") || keyword.is_ident("static")) && name.is_ident(list)
        })
        .map(|(at, _)| at.saturating_add(1))
        .collect()
}

/// The problems of the lists of git's variables `source` declares.
fn rust_lists(source: &RustSource) -> Vec<String> {
    let tokens = source.tokens.as_slice();
    let own = [
        (
            "HERMETIC_GIT",
            HERMETIC_GIT
                .iter()
                .flat_map(|&pair| <[&str; 2]>::from(pair))
                .collect::<Vec<_>>(),
        ),
        ("LOCAL_GIT_VARIABLES", LOCAL_GIT_VARIABLES.to_vec()),
    ];
    own.into_iter()
        .flat_map(|(list, entries)| {
            lists_of(tokens, list)
                .into_iter()
                .filter(move |&at| literals(tokens, at).as_ref() != Some(&entries))
                .map(move |at| {
                    let line = tokens.get(at).map_or(0, |name| name.line);
                    format!("{}:{line}: {}", source.path(), unlike(list))
                })
        })
        .collect()
}

/// The strings of the array of literals the declaration named at `at` in `tokens` is given,
/// or `None` when it is given anything else.
fn literals(tokens: &[Token], at: usize) -> Option<Vec<&str>> {
    let before = tokens
        .iter()
        .skip(at)
        .take_while(|token| !token.is_punct('='))
        .count();
    let open = at.saturating_add(before).saturating_add(1);
    let _array = tokens.get(open).filter(|token| token.opens('['))?;
    tokens
        .get(open..close_of(tokens, open))
        .unwrap_or_default()
        .iter()
        .filter(|token| {
            !matches!(token.kind, TokenKind::Open | TokenKind::Close) && !token.is_punct(',')
        })
        .map(|token| {
            token
                .text
                .strip_prefix('"')
                .and_then(|text| text.strip_suffix('"'))
        })
        .collect()
}

/// What is wrong with a copy of `list` that is not xtask's.
fn unlike(list: &str) -> String {
    format!(
        "`{list}` is not xtask's (crates/xtask/src/runner.rs): declare the same entries, in the \
         same order, as literals"
    )
}

/// The one form of a copy of `list` in TypeScript, every run of white space one space: the
/// declaration as the formatter writes it, `/dev/null` as Node.js's `devNull`.
fn script_list(list: &str) -> String {
    if list == "HERMETIC_GIT" {
        let pairs: Vec<String> = HERMETIC_GIT
            .iter()
            .map(|&(name, value)| match value {
                "/dev/null" => format!("\"{name}\": devNull,"),
                text => format!("\"{name}\": \"{text}\","),
            })
            .collect();
        format!(
            "const HERMETIC_GIT: Readonly<Record<string, string>> = {{ {} }};",
            pairs.join(" ")
        )
    } else {
        let entries = LOCAL_GIT_VARIABLES.join("\", \"");
        format!("const LOCAL_GIT_VARIABLES: readonly string[] = [ \"{entries}\", ];")
    }
}

/// Adds to `problems` those of the script `text` at `path`: its lists of git's variables,
/// and the processes it starts, then to `held` its path if it starts one.
fn script_starts(
    path: &str,
    text: &str,
    problems: &mut Vec<String>,
    held: &mut Vec<(String, String)>,
) {
    let code: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut declares = |list: &str| {
        let declared = |within: &str| {
            ["const", "let", "var"]
                .iter()
                .any(|keyword| within.contains(&format!("{keyword} {list}")))
        };
        let form = script_list(list);
        if !declared(&code) {
            return false;
        }
        // One declaration, and it is the form: none other is left once it is taken out, nor is
        // it prose.
        let alike = code.contains(&form)
            && !declared(&code.replacen(&form, "", 1))
            && !prose_texts(path, text)
                .iter()
                .any(|prose| declared(&prose.text.0));
        if !alike {
            problems.push(format!("{path}: {}", unlike(list)));
        }
        true
    };
    let (hermetic, local) = (declares("HERMETIC_GIT"), declares("LOCAL_GIT_VARIABLES"));
    if !text.contains(SCRIPT_MODULE) {
        return;
    }
    held.push((path.to_owned(), String::new()));
    let Some(&(_, kind, _)) = SCRIPT_STARTS.iter().find(|&&(file, _, _)| file == path) else {
        problems.push(format!(
            "{path}: holds `{SCRIPT_MODULE}` and is no row of SCRIPT_STARTS: add it, with what it \
             may start and why ({STARTS_FILE})"
        ));
        return;
    };
    if let Some(lacks) = lacking(kind, hermetic, local) {
        problems.push(format!("{path}: {lacks}"));
    }
}

#[cfg(test)]
#[path = "hermetic_tests.rs"]
mod tests;
