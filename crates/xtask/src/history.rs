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

/// The last commit the history gate does not hold to the breaking rule: every commit after
/// it that adds a breaking entry to `CHANGELOG.md` ([`BREAKING_ENTRY`]) carries `!` in its
/// header ([`breaking_bang_gate`]).
///
/// The version commit of 0.6.0 on `main`: the rule came after it, and the commits of `main`
/// before it are not rewritten.
pub const BREAKING_SINCE: &str = "8ff0722ec0273e176d23865891563e8bbe8a1c41";

/// How a changelog entry of a breaking change begins, alone on its line.
pub const BREAKING_ENTRY: &str = "- **Breaking";

/// The commits the history gate is pinned to, each with the constant naming it:
/// [`SCOPES_SINCE`], [`CHANGELOG_SINCE`], [`BREAKING_SINCE`], then each of
/// [`CI_MARKER_EXEMPT`].
///
/// Each must be a commit of `main` ([`pins_gate`]): the repository merges by rebase, which
/// rewrites every commit of a branch, so a pin to a commit only a branch holds would name,
/// after the merge, a commit the history of `main` does not hold.
#[must_use]
pub fn history_pins() -> Vec<(&'static str, &'static str)> {
    [
        ("SCOPES_SINCE", SCOPES_SINCE),
        ("CHANGELOG_SINCE", CHANGELOG_SINCE),
        ("BREAKING_SINCE", BREAKING_SINCE),
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

/// Lists every commit after `since` with what it changes in `CHANGELOG.md`, what
/// [`breaking_bang_gate`] reads.
///
/// Records separated by `0x1e`, each its full hash, `0x1f`, its header, then the lines of
/// its diff of the changelog with no context (the check passes [`BREAKING_SINCE`]), read as
/// text whatever an attribute says (`--text`, `--no-textconv`), never `Binary files …
/// differ`.
#[must_use]
pub fn breaking_log_command(since: &str) -> Cmd {
    Cmd::new(
        "git",
        &[
            "log",
            "--format=%x1e%H%x1f%s",
            "--patch",
            "--unified=0",
            "--no-color",
            "--no-ext-diff",
            "--text",
            "--no-textconv",
            &format!("{since}..HEAD"),
            "--",
            CHANGELOG,
        ],
        Dir::Root,
    )
}

/// How each line of a diff [`breaking_bang_gate`] reads opens: a diff's own header, its
/// mode and index lines, a hunk, an added or a removed line (the file's two names among
/// them), the end of a file with no line break; a blank line, the one after a header, is
/// read too.
const DIFF_LINES: [&str; 10] = [
    "diff --git ",
    "new file mode ",
    "deleted file mode ",
    "old mode ",
    "new mode ",
    "index ",
    "@@ ",
    "+",
    "-",
    "\\ No newline at end of file",
];

/// Checks that every commit adding a breaking entry to the changelog says it breaks.
///
/// A commit whose diff of `CHANGELOG.md` (`log`, the output of [`breaking_log_command`])
/// adds more lines beginning with [`BREAKING_ENTRY`] than it removes carries `!` in its
/// header (`type(scope)!: summary`); one that rewords an entry, or a version commit moving
/// entries under a release heading, adds as many as it removes. It counts lines in each
/// commit's own diff: a commit that merges two breaking entries into one and adds a new one
/// at once passes unmarked, kept out by one logical change per commit; what an entry says
/// breaks is judged against the last version by the names it cites (`breaking_names_gate`),
/// and its spelling by `breaking_spellings_gate`. A line of a diff it does not know (a
/// diff's own header, mode, index or hunk line, an added or removed line, the end of a file
/// with no line break, a blank line) is refused, never skipped, so that a diff git printed
/// in another form (`Binary files … differ`) cannot pass vacuously.
///
/// # Errors
///
/// Returns one line per commit at fault, naming it by its short hash, and one per line of a
/// diff it cannot read.
pub fn breaking_bang_gate(log: &str) -> Result<(), Failure> {
    let mut problems = Vec::new();
    for record in log.split('\u{1e}').filter(|record| !record.is_empty()) {
        let (hash, rest) = record.split_once('\u{1f}').unwrap_or((record, ""));
        let mut lines = rest.lines();
        let header = lines.next().unwrap_or_default();
        let short: String = hash.chars().take(7).collect();
        problems.extend(
            lines
                .clone()
                .filter(|line| {
                    !line.is_empty() && !DIFF_LINES.iter().any(|known| line.starts_with(known))
                })
                .map(|line| {
                    format!(
                        "commit {short}: its diff of {CHANGELOG} holds a line the gate cannot \
                         read: `{line}`"
                    )
                }),
        );
        let (added, removed) = lines.fold((0_usize, 0_usize), |(added, removed), line| match line
            .split_at_checked(1)
        {
            Some(("+", entry)) if entry.starts_with(BREAKING_ENTRY) => {
                (added.saturating_add(1), removed)
            }
            Some(("-", entry)) if entry.starts_with(BREAKING_ENTRY) => {
                (added, removed.saturating_add(1))
            }
            _ => (added, removed),
        });
        let (_, marked) = header_prefix(header);
        if added > removed && !marked {
            problems.push(format!(
                "commit {short}: it adds a breaking entry to {CHANGELOG}, so its header carries \
                 `!` (`type(scope)!: summary`)"
            ));
        }
    }
    verdict("the history breaks the breaking rule", problems)
}

/// The prefix of a commit header, its type and scope (`type(scope)`, before its `: `; the
/// whole header without one), and whether it marks a breaking change, a `!` ending that
/// prefix (a header without `: ` marks none): the one reading of the mark, for the
/// commit-msg check, the bang gate and [`changes_behavior`].
pub(crate) fn header_prefix(header: &str) -> (&str, bool) {
    header
        .split_once(": ")
        .map_or((header, false), |(prefix, _)| {
            prefix
                .strip_suffix('!')
                .map_or((prefix, false), |kept| (kept, true))
        })
}

/// Whether a commit message announces a change of behavior: a `feat`, `fix` or `perf`
/// commit, or any breaking one (`!` or a `BREAKING CHANGE:` footer).
#[must_use]
pub fn changes_behavior(message: &str) -> bool {
    let header = message.lines().next().unwrap_or_default();
    let (prefix, breaks) = header_prefix(header);
    let kind = prefix.split('(').next().unwrap_or_default();
    ["feat", "fix", "perf"].contains(&kind)
        || breaks
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
/// The working tree (`view`) has nothing uncommitted, nor hidden from git status, by a
/// filter or by bytes git does not show ([`unclean`]), and every commit pushed (`pushed`, each object the
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
