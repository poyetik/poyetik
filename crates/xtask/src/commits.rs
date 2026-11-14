//! Commit messages: their types, scopes and CI markers, and the commit-msg gate.

use crate::{
    parts::{Part, full_parts},
    steps::Failure,
    workspace::{PACKAGES, RUST_CRATES},
};

/// Commit types allowed by the Conventional Commits rule of the project.
pub const COMMIT_TYPES: [&str; 11] = [
    "feat", "fix", "docs", "test", "refactor", "perf", "build", "ci", "chore", "style", "revert",
];

/// The scopes a commit may name besides the crates and the packages ([`commit_scopes`]),
/// each with what it names.
pub const COMMIT_SCOPE_EXTRAS: [(&str, &str); 8] = [
    ("spec", "the specifications under spec/"),
    ("conformance", "the conformance corpus"),
    ("ci", "the CI workflow"),
    (
        "build",
        "the build files of the workspaces and the toolchains",
    ),
    ("release", "a version commit"),
    ("changelog", "CHANGELOG.md"),
    ("glossary", "GLOSSARY.md and vocabulary.txt"),
    ("deep-review", "the deep review skill"),
];

/// Every scope a commit may name, derived from the registries.
///
/// Each crate of [`RUST_CRATES`] by its name, each package of [`PACKAGES`] by the last
/// folder of its directory, or by the name of the crate that shares that directory
/// (`audio-wasm`, never `wasm`), then [`COMMIT_SCOPE_EXTRAS`]: one name per part, so that
/// a part is never named two ways.
#[must_use]
pub fn commit_scopes() -> Vec<&'static str> {
    let packages = PACKAGES.iter().map(|package| {
        RUST_CRATES
            .iter()
            .find(|krate| krate.dir == package.dir)
            .map_or_else(
                || package.dir.rsplit('/').next().unwrap_or(package.dir),
                |krate| krate.name,
            )
    });
    let mut scopes: Vec<&str> = RUST_CRATES.iter().map(|krate| krate.name).collect();
    for scope in packages.chain(COMMIT_SCOPE_EXTRAS.iter().map(|(scope, _)| *scope)) {
        if !scopes.contains(&scope) {
            scopes.push(scope);
        }
    }
    scopes
}

/// The last commit whose scope the history gate does not hold to [`commit_scopes`].
///
/// The scopes were free text up to it, and the published history is not rewritten. It is
/// the commit of `main` the branch that brought the rule started from, as every pin of the
/// history gate is a commit of `main` ([`history_pins`](crate::history::history_pins)).
pub const SCOPES_SINCE: &str = "9853e43382d0af0e44970c8304294d34e0d8ac2b";

/// Longest allowed commit header, in characters.
pub const COMMIT_HEADER_MAX: usize = 72;

/// Checks a commit message against Conventional Commits.
///
/// Expected header: `type(scope)!: summary`, scope and `!` optional, at most
/// [`COMMIT_HEADER_MAX`] characters, no trailing period, blank line before any body; a
/// scope is one of [`commit_scopes`].
/// `message` is the file git hands the commit-msg hook: its lines starting with `#` are
/// git's comments, removed before the commit is recorded, and are ignored. Every message
/// follows the convention, those git writes for merges and reverts included (`Merge …`,
/// `Revert "…"`): write `revert: …`. The marker [`CI_MARKER`] may only stand alone on a
/// line ([`ci_marker`]).
///
/// # Errors
///
/// Returns a message explaining the expected format, or that the message names
/// [`CI_MARKER`] other than alone on a line.
pub fn commit_msg_gate(message: &str) -> Result<(), Failure> {
    let text: Vec<&str> = message
        .lines()
        .filter(|line| !line.starts_with('#'))
        .collect();
    let text = text.join("\n");
    conventional(&text, true)?;
    ci_marker(&text).map_or(Ok(()), |problem| Err(Failure::from(problem)))
}

/// The marker asking CI to run on a push: the workflow runs the check when the last
/// commit's message contains it (see [`WORKFLOW_CONDITIONS`](crate::workflow::WORKFLOW_CONDITIONS)), every part of it.
pub const CI_MARKER: &str = "[ci]";

/// How the marker asking CI to run only some parts begins: `[ci <part> <part>…]`, the parts
/// named as [`Part::name`] does, [`ci_parts_marker`] giving the one form accepted.
pub const CI_PARTS_MARKER: &str = "[ci ";

/// Commits published before the marker was confined to a line of its own, whose message
/// names it in a sentence: their history is not rewritten.
pub const CI_MARKER_EXEMPT: [&str; 1] = ["431b8a141a99759fb5c2a5adefd4fc551c43276c"];

/// The parts the workflow's matrix can run: every part of the full check but the final
/// one, which runs only after all of them ([`workflow_text`](crate::workflow::workflow_text)).
#[must_use]
pub fn matrix_parts() -> Vec<Part> {
    full_parts()
        .into_iter()
        .filter(|part| *part != Part::Final)
        .collect()
}

/// The marker asking CI to run `parts`, the one form of it [`ci_marker`] accepts: `[ci]`
/// for every part of the matrix, else `[ci <part> <part>…]`, the parts in the order of
/// [`matrix_parts`], separated by one space.
#[must_use]
pub fn ci_parts_marker(parts: &[Part]) -> String {
    let named: Vec<String> = matrix_parts()
        .into_iter()
        .filter(|part| parts.contains(part))
        .map(Part::name)
        .collect();
    if named.len() == matrix_parts().len() || named.is_empty() {
        CI_MARKER.to_owned()
    } else {
        format!("{CI_PARTS_MARKER}{}]", named.join(" "))
    }
}

/// Reads parts named by `names`, separated by white space, in any order: each a part of
/// [`matrix_parts`] by its name, each once; given back in the order of [`matrix_parts`].
///
/// # Errors
///
/// Returns why a name is none, or is repeated, or that no part is named.
pub fn named_parts(names: &str) -> Result<Vec<Part>, String> {
    let known = matrix_parts();
    let mut chosen: Vec<Part> = Vec::new();
    for name in names.split_whitespace() {
        let part = known
            .iter()
            .copied()
            .find(|part| part.name() == name)
            .ok_or_else(|| {
                format!("unknown part \"{name}\" (`cargo xtask list --full --parts` lists them, final apart)")
            })?;
        if chosen.contains(&part) {
            return Err(format!("part \"{name}\" named twice"));
        }
        chosen.push(part);
    }
    if chosen.is_empty() {
        return Err("no part named".to_owned());
    }
    Ok(known
        .into_iter()
        .filter(|part| chosen.contains(part))
        .collect())
}

/// The parts the marker of `line` asks for, if `line` is a marker in its one accepted form
/// ([`ci_parts_marker`]), read in any case as GitHub reads it: every part for `[ci]`.
pub(crate) fn marker_parts(line: &str) -> Option<Vec<Part>> {
    // Uppercased, as `ci_marker` reads a marker: `ı` and `i` both give `I`.
    let line = line.trim().to_uppercase();
    if line == CI_MARKER.to_uppercase() {
        return Some(matrix_parts());
    }
    let names = line
        .strip_prefix(&CI_PARTS_MARKER.to_uppercase())?
        .strip_suffix(']')?
        .to_lowercase();
    let parts = named_parts(&names).ok()?;
    (ci_parts_marker(&parts).to_uppercase() == line).then_some(parts)
}

/// Whether `line` names a marker, well formed or not: it holds `[ci]` or `[ci ` in any case,
/// which GitHub's `contains` finds ([`WORKFLOW_CONDITIONS`](crate::workflow::WORKFLOW_CONDITIONS)).
pub(crate) fn names_a_marker(line: &str) -> bool {
    let line = line.to_uppercase();
    [CI_MARKER, CI_PARTS_MARKER]
        .iter()
        .any(|marker| line.contains(&marker.to_uppercase()))
}

/// Why `message` names a CI marker other than alone on a line in its one form, if it does.
///
/// A marker is `[ci]` or `[ci <part>…]` ([`ci_parts_marker`]). GitHub only offers `contains` to read a commit message, so a marker in a sentence (a
/// commit describing the rule, say) would start CI unasked: it is refused anywhere but on
/// a line of its own, the one way to ask for CI. GitHub's `contains` ignores case (`[CI]`
/// starts CI too), so a marker is read in any case. A marker naming parts names known parts
/// of the matrix, each once, in their order ([`ci_parts_marker`]), and every marker line of
/// a message asks for the same parts.
#[must_use]
pub fn ci_marker(message: &str) -> Option<String> {
    let markers: Vec<&str> = message
        .lines()
        .filter(|line| names_a_marker(line))
        .collect();
    if markers.iter().any(|line| marker_parts(line).is_none()) {
        return Some(format!(
            "invalid commit message: {CI_MARKER} asks CI to run, and `[ci <part> <part>…]` some parts of it (`cargo xtask list --full --parts` names them, in their order, final apart); write one alone on a line, or none at all"
        ));
    }
    let mut asked = markers.iter().filter_map(|line| marker_parts(line));
    let first = asked.next();
    asked
        .any(|parts| Some(parts) != first)
        .then(|| "invalid commit message: its CI markers ask for different parts".to_owned())
}

/// The parts a run of the workflow checks.
///
/// Read from its event (`github.event_name`), the message of the last commit pushed (on a
/// push) and the parts given by hand (a manual run, white-space separated, empty for every
/// part): every part for a pull request and for a
/// manual run given none; on a push, those its marker asks for ([`ci_marker`]), none
/// without one.
///
/// # Errors
///
/// Returns why the parts given by hand or the marker cannot be read, or that the event is
/// not one the workflow runs on.
pub fn ci_parts(event: &str, message: &str, input: &str) -> Result<Vec<Part>, String> {
    match event {
        "pull_request" => Ok(matrix_parts()),
        "workflow_dispatch" if input.trim().is_empty() => Ok(matrix_parts()),
        "workflow_dispatch" => named_parts(input),
        "push" => {
            if let Some(problem) = ci_marker(message) {
                return Err(problem);
            }
            Ok(message.lines().find_map(marker_parts).unwrap_or_default())
        }
        other => Err(format!("the workflow does not run on \"{other}\"")),
    }
}

/// The outputs of the workflow's first job, as GitHub reads them from `$GITHUB_OUTPUT`.
///
/// `parts`, the names of `parts` as a JSON array (the matrix of the check), and `full`,
/// whether they are every part of the matrix (only then does the final part run).
#[must_use]
pub fn ci_outputs(parts: &[Part]) -> [String; 2] {
    let names: Vec<String> = parts
        .iter()
        .map(|part| format!("\"{}\"", part.name()))
        .collect();
    [
        format!("parts=[{}]", names.join(",")),
        format!("full={}", parts == matrix_parts().as_slice()),
    ]
}

/// Checks a recorded commit message, as it is, against the convention of
/// [`commit_msg_gate`], its scope held to [`commit_scopes`] when `registered`.
pub(crate) fn conventional(message: &str, registered: bool) -> Result<(), Failure> {
    let mut lines = message.lines();
    let header = lines.next().unwrap_or("");
    let fail = |reason: &str| {
        Err(Failure {
            summary: format!("invalid commit message: {reason}"),
            details: vec![format!(
                "expected: <type>(<scope>)!: <summary>, type one of {}",
                COMMIT_TYPES.join(", ")
            )],
            usage: false,
        })
    };
    let Some((prefix, summary)) = header.split_once(": ") else {
        return fail("missing \"<type>: \" prefix");
    };
    let prefix = prefix.strip_suffix('!').unwrap_or(prefix);
    let (kind, scope) = match prefix.split_once('(') {
        Some((kind, rest)) => (kind, rest.strip_suffix(')')),
        None => (prefix, Some("x")),
    };
    if !COMMIT_TYPES.contains(&kind) {
        return fail(&format!("unknown type \"{kind}\""));
    }
    let scope_ok = scope.is_some_and(|name| {
        !name.is_empty()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    });
    if !scope_ok {
        return fail("the scope must be lowercase letters, digits or hyphens");
    }
    let scopes = commit_scopes();
    if let Some(name) =
        scope.filter(|name| registered && prefix.contains('(') && !scopes.contains(name))
    {
        return fail(&format!(
            "unknown scope \"{name}\": name a crate, a package or a scope of xtask's \
             COMMIT_SCOPE_EXTRAS ({})",
            scopes.join(", ")
        ));
    }
    if summary.trim().is_empty() || summary.ends_with('.') {
        return fail("the summary must be non-empty and must not end with a period");
    }
    if header.chars().count() > COMMIT_HEADER_MAX {
        return fail(&format!(
            "the header is longer than {COMMIT_HEADER_MAX} characters"
        ));
    }
    if lines.next().is_some_and(|line| !line.trim().is_empty()) {
        return fail("a blank line must separate the header from the body");
    }
    Ok(())
}

#[cfg(test)]
#[path = "commits_tests.rs"]
mod tests;
