//! The history gates: pinned commits, the history of the branch, and the pre-push gate.

use alloc::collections::BTreeMap;

use crate::{
    actions::is_deletion,
    commits::{CI_MARKER_EXEMPT, SCOPES_SINCE, ci_marker, conventional},
    proof_record::{TreeView, unclean},
    release::CHANGELOG,
    steps::{Cmd, Dir, Failure, verdict},
};

/// The last commit the history gate does not hold to the changelog rule: every change of
/// behavior after it must update `CHANGELOG.md` in the same commit.
///
/// The rule itself is older (9a32301), but it was enforced only when the gate came, after
/// this commit: this commit and 81509cf, before it, added their entries in later commits,
/// and the history is not rewritten to move them.
pub const CHANGELOG_SINCE: &str = "a6614dfa18a3b69634f7f581f711d3f509bb5115";

/// The commits the history gate is pinned to, each with the constant naming it:
/// [`SCOPES_SINCE`], [`CHANGELOG_SINCE`], then each of [`CI_MARKER_EXEMPT`].
///
/// Each must be a commit of `main` ([`pins_gate`]): the repository merges by rebase, which
/// rewrites every commit of a branch, so a pin to a commit only a branch holds would name,
/// after the merge, a commit the history of `main` does not hold.
#[must_use]
pub fn history_pins() -> Vec<(&'static str, &'static str)> {
    [
        ("SCOPES_SINCE", SCOPES_SINCE),
        ("CHANGELOG_SINCE", CHANGELOG_SINCE),
    ]
    .into_iter()
    .chain(CI_MARKER_EXEMPT.map(|hash| ("CI_MARKER_EXEMPT", hash)))
    .collect()
}

/// Lists the history of `base`, one full hash per line: what [`pins_gate`] reads.
#[must_use]
pub fn pins_command(base: &str) -> Cmd {
    Cmd::new("git", &["log", "--format=%H", base], Dir::Root)
}

/// Checks that every pin of `pins` (a constant's name, its commit) is a commit of
/// `ancestors`, the history of `base`, the merge base of `HEAD` with `against`, one of
/// [`MAIN_BASES`](crate::workspace::MAIN_BASES) (the output of [`pins_command`]).
///
/// A pin a branch alone holds is refused on that branch, before the rebase merge rewrites
/// it ([`history_pins`]), and so is a pin no longer in the history at all.
///
/// # Errors
///
/// Returns one line per pin that is not a commit of `ancestors`, naming its constant.
pub fn pins_gate(
    pins: &[(&str, &str)],
    ancestors: &str,
    (base, against): (&str, &str),
) -> Result<(), Failure> {
    let ancestors: Vec<&str> = ancestors.lines().collect();
    let problems = pins
        .iter()
        .filter(|(_, hash)| !ancestors.contains(hash))
        .map(|(name, hash)| {
            format!(
                "{name} names {hash}, which is not in the history of {base} (merge base \
                 with {against}): pin a commit main holds"
            )
        })
        .collect();
    verdict(
        "the history gate is pinned to commits main does not hold",
        problems,
    )
}

/// The three history listings the history gate reads.
///
/// Every commit with its message, the files each commit after `since` touched (`since`
/// itself excluded; the check passes [`CHANGELOG_SINCE`]), records separated by `0x1e`,
/// and the commits after `scopes_since`, one per line (the check passes [`SCOPES_SINCE`]).
#[must_use]
pub fn history_commands(since: &str, scopes_since: &str) -> [Cmd; 3] {
    let git = |args: &[&str]| Cmd::new("git", args, Dir::Root);
    [
        git(&["log", "--format=%x1e%H%x1f%B", "HEAD"]),
        git(&[
            "log",
            "--format=%x1e%H",
            "--name-only",
            &format!("{since}..HEAD"),
        ]),
        git(&["log", "--format=%H", &format!("{scopes_since}..HEAD")]),
    ]
}

/// Whether a commit message announces a change of behavior: a `feat`, `fix` or `perf`
/// commit, or any breaking one (`!` or a `BREAKING CHANGE:` footer).
#[must_use]
pub fn changes_behavior(message: &str) -> bool {
    let header = message.lines().next().unwrap_or_default();
    let prefix = header.split_once(": ").map_or(header, |(prefix, _)| prefix);
    let kind = prefix.split(['(', '!']).next().unwrap_or_default();
    ["feat", "fix", "perf"].contains(&kind)
        || prefix.ends_with('!')
        || message
            .lines()
            .any(|line| line.starts_with("BREAKING CHANGE:"))
}

/// Whether `message` is exactly the one GitHub writes for the merge it checks out to run a
/// pull request: `Merge <40 hex> into <40 hex>`, in lowercase, alone (as `git log` writes
/// it, followed by newlines).
pub(crate) fn synthetic_merge(message: &str) -> bool {
    let hash = |text: &str| {
        text.len() == 40
            && text
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    };
    // `git log` ends the message with one newline more than it holds: none or one.
    message
        .trim_end_matches('\n')
        .strip_prefix("Merge ")
        .and_then(|line| line.split_once(" into "))
        .is_some_and(|(head, base)| hash(head) && hash(base))
}

/// Checks the history of commits.
///
/// Every commit message, as recorded, follows the convention ([`commit_msg_gate`](crate::commits::commit_msg_gate)), its
/// scope held to [`commit_scopes`](crate::commits::commit_scopes) only in the commits after [`SCOPES_SINCE`] (`scoped`),
/// and every commit since [`CHANGELOG_SINCE`] that changes behavior touches `CHANGELOG.md`.
///
/// One commit is not held to the convention: the first of `messages`, the commit checked
/// out, when its message is exactly the one GitHub writes for the merge it checks out to
/// run a pull request (`Merge <commit> into <commit>`). That merge is never recorded (the
/// repository allows rebase merges only), so the same message anywhere else is refused.
///
/// `messages` and `files` are the outputs of [`history_commands`].
///
/// # Errors
///
/// Returns one line per commit at fault, naming it by its short hash.
pub fn history_gate(messages: &str, files: &str, scoped: &str) -> Result<(), Failure> {
    let scoped: Vec<&str> = scoped.lines().collect();
    let short = |hash: &str| hash.chars().take(7).collect::<String>();
    let touched: BTreeMap<&str, Vec<&str>> = files
        .split('\u{1e}')
        .filter_map(|record| {
            let mut lines = record.lines();
            lines
                .next()
                .filter(|hash| !hash.is_empty())
                .map(|hash| (hash, lines.filter(|line| !line.is_empty()).collect()))
        })
        .collect();
    let mut problems = Vec::new();
    let records = messages.split('\u{1e}').filter(|record| !record.is_empty());
    for (index, record) in records.enumerate() {
        let (hash, message) = record.split_once('\u{1f}').unwrap_or((record, ""));
        if index == 0 && synthetic_merge(message) {
            continue;
        }
        if let Err(failure) = conventional(message, scoped.contains(&hash)) {
            problems.push(format!("commit {}: {}", short(hash), failure.summary));
        }
        if let Some(problem) = ci_marker(message).filter(|_| !CI_MARKER_EXEMPT.contains(&hash)) {
            problems.push(format!("commit {}: {problem}", short(hash)));
        }
        let unrecorded = touched
            .get(hash)
            .is_some_and(|paths| !paths.contains(&CHANGELOG));
        if unrecorded && changes_behavior(message) {
            problems.push(format!(
                "commit {} changes behavior but not {CHANGELOG}",
                short(hash)
            ));
        }
    }
    verdict("the history breaks the commit rules", problems)
}

/// Checks that the pre-push hook validates exactly what is pushed.
///
/// The working tree (`view`) has nothing uncommitted, nor hidden from git status or by a
/// filter ([`unclean`]), and every commit pushed (`pushed`, each object the
/// hook received peeled to its commit) is `head`, the commit checked out. A deleted branch (all
/// zeros) pushes nothing.
///
/// # Errors
///
/// Returns what to commit or check out first.
pub fn pre_push_gate(head: &str, view: &TreeView, pushed: &[String]) -> Result<(), Failure> {
    let mut problems = Vec::new();
    if !view.status.trim().is_empty() {
        problems.push(format!(
            "the working tree has changes not committed, which the check would include: {}",
            view.status
                .lines()
                .map(str::trim)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    } else if let Some(reason) = unclean(view) {
        problems.push(format!(
            "the working tree may differ from what is committed, which the check would \
             include: {reason}"
        ));
    }
    for commit in pushed {
        if !is_deletion(commit) && commit != head.trim() {
            problems.push(format!(
                "pushing {commit}, but {} is checked out: check out the pushed commit, then push again",
                head.trim()
            ));
        }
    }
    verdict("the push cannot be checked", problems)
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;
