//! The coverage gates, Rust and TypeScript, and the state of the tree they measure.

use alloc::collections::BTreeSet;

use crate::{
    actions::{git_output, read_file},
    runner::Runner,
    steps::{Failure, INCLUDE_IGNORED, verdict},
    workspace::{COVERAGE_LOCALITY, Coverage, Measured, NIGHTLY_TOOLCHAIN},
};

/// What the working tree holds that is not committed (the files git reports as changed or
/// untracked, ignored files apart, and the changes of the tracked ones), and what is
/// checked out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeState {
    /// `git status --porcelain --untracked-files=all`.
    pub status: String,
    /// `git diff --no-ext-diff --no-textconv HEAD`: the content of the changes, as they
    /// are (no external diff or text conversion of the user's).
    pub diff: String,
    /// `git rev-parse HEAD --symbolic-full-name HEAD`: the commit checked out, then its
    /// branch (`HEAD` when detached).
    pub head: String,
}

/// Checks that the checks left the working tree as they found it.
///
/// A tree may be dirty (a local check of uncommitted work), but the checks must not change
/// it: a command that rewrites a tracked file (a stale `Cargo.lock` an unlocked `cargo`
/// would update, a generated file) or leaves a file git does not ignore makes CI differ from
/// what was checked. Nor may what is checked out change (a commit made or a branch checked
/// out in the same checkout while the checks run): the steps after it would read another
/// commit than the one recorded, which a push would then send as checked.
///
/// # Errors
///
/// Returns every file whose state changed, a change of what is checked out, or that the
/// tree was not recorded.
pub fn tree_gate(before: Option<&TreeState>, after: &TreeState) -> Result<(), Failure> {
    let Some(before) = before else {
        return Err(Failure::from(
            "the working tree was not recorded before the checks".to_owned(),
        ));
    };
    let lines = |status: &str| -> BTreeSet<String> { status.lines().map(str::to_owned).collect() };
    let (old, new) = (lines(&before.status), lines(&after.status));
    let mut problems: Vec<String> = new
        .difference(&old)
        .map(|line| format!("`{line}` appeared"))
        .chain(
            old.difference(&new)
                .map(|line| format!("`{line}` went away")),
        )
        .collect();
    if problems.is_empty() && before.diff != after.diff {
        problems.push("the uncommitted changes of a tracked file changed".to_owned());
    }
    if before.head != after.head {
        let words = |head: &str| head.split_whitespace().collect::<Vec<_>>().join(" ");
        problems.push(format!(
            "what is checked out changed, from `{}` to `{}`: the checks read another commit than the one recorded",
            words(&before.head),
            words(&after.head)
        ));
    }
    verdict("the checks changed the working tree", problems)
}

/// Reads the state of the working tree.
pub(crate) fn tree_state(runner: &mut dyn Runner) -> Result<TreeState, Failure> {
    Ok(TreeState {
        status: git_output(
            runner,
            &["status", "--porcelain", "--untracked-files=all"],
            "read the work tree",
        )?,
        diff: git_output(
            runner,
            &["diff", "--no-ext-diff", "--no-textconv", "HEAD"],
            "read the work tree",
        )?,
        head: git_output(
            runner,
            &["rev-parse", "HEAD", "--symbolic-full-name", "HEAD"],
            "read the work tree",
        )?,
    })
}

/// Reads one `{ count, covered }` total of an llvm-cov JSON summary.
pub(crate) fn total(totals: Option<&serde_json::Value>, kind: &str) -> Result<(u64, u64), String> {
    let field = |name: &str| {
        totals
            .and_then(|value| value.get(kind))
            .and_then(|entry| entry.get(name))
            .and_then(serde_json::Value::as_u64)
    };
    match (field("count"), field("covered")) {
        (Some(count), Some(covered)) => Ok((count, covered)),
        _ => Err(format!("coverage report has no {kind} total")),
    }
}

/// Checks that an llvm-cov JSON summary of `coverage` reports 100% line, function and
/// branch coverage of every file it measures.
///
/// For a library, every file of the report belongs to the crate (`<dir>/src/`) and at least
/// one is measured; for a binary, only its `src/main.rs` is read, and it must be there.
///
/// # Errors
///
/// Returns every file not fully covered with its counts and the command showing the lines,
/// every file of another crate, or why the report cannot be read.
pub fn coverage_gate(report: &str, coverage: Coverage) -> Result<(), Failure> {
    let krate = coverage.krate;
    let json: serde_json::Value = serde_json::from_str(report)
        .map_err(|error| Failure::from(format!("invalid coverage report: {error}")))?;
    let files = json
        .pointer("/data/0/files")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| Failure::from("coverage report has no files".to_owned()))?;
    let own = format!("/{}/src/", krate.dir);
    let mut problems = Vec::new();
    let mut measured = 0_usize;
    let mut incomplete = false;
    for file in files {
        let name = file
            .get("filename")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let inside = match (name.split_once(&own), coverage.measured) {
            (Some((_, inside)), Measured::Library) => inside,
            (Some((_, "main.rs")), Measured::Binary) => "main.rs",
            (None, Measured::Library) => {
                problems.push(format!("{name} is not a file of {}", krate.name));
                continue;
            }
            (_, Measured::Binary) => continue,
        };
        measured = measured.saturating_add(1);
        let mut missing = Vec::new();
        for kind in ["lines", "functions", "branches"] {
            let (count, covered) = total(file.get("summary"), kind)?;
            if covered != count {
                missing.push(format!("{kind} {covered}/{count}"));
            }
        }
        if !missing.is_empty() {
            incomplete = true;
            problems.push(format!(
                "{}/src/{inside}: {}",
                krate.dir,
                missing.join(", ")
            ));
        }
    }
    if measured == 0 {
        problems.push(format!("no file of {} measured", krate.name));
    }
    let context = match coverage.measured {
        Measured::Library => format!("{} is not covered 100% by its own unit tests", krate.name),
        Measured::Binary => format!("the {} binary is not covered 100% by its tests", krate.name),
    };
    verdict(&context, problems).map_err(|mut failure| {
        if incomplete {
            let lib = if coverage.measured == Measured::Library {
                " --lib"
            } else {
                ""
            };
            failure.details.push(format!(
                "the lines: cargo {NIGHTLY_TOOLCHAIN} llvm-cov --manifest-path {}/Cargo.toml{lib} --branch --show-missing-lines -- {INCLUDE_IGNORED}",
                krate.dir
            ));
        }
        failure
    })
}

/// Measures [`COVERAGE_LOCALITY`], whose integration test reaches a function its unit tests
/// do not, and requires the coverage gate to refuse its `src/lib.rs`.
pub(crate) fn coverage_locality(runner: &mut dyn Runner) -> Result<(), Failure> {
    let coverage = Coverage {
        krate: COVERAGE_LOCALITY,
        measured: Measured::Library,
    };
    let report = measure(runner, coverage)?;
    let refused = format!("{}/src/lib.rs: ", COVERAGE_LOCALITY.dir);
    match coverage_gate(&report, coverage) {
        Err(failure) if failure.summary.contains(&refused) => Ok(()),
        Err(failure) => Err(Failure::from(format!(
            "the coverage gate refused {} for another reason: {}",
            COVERAGE_LOCALITY.dir, failure.summary
        ))),
        Ok(()) => Err(Failure::from(format!(
            "the coverage gate accepted {}: a path reached only by an integration test counts as covered",
            COVERAGE_LOCALITY.dir
        ))),
    }
}

/// Runs a coverage measurement and reads its report.
pub(crate) fn measure(runner: &mut dyn Runner, coverage: Coverage) -> Result<String, Failure> {
    runner
        .run(&Coverage::clean_command())
        .map_err(|failure| Failure::from(failure.to_string()))?;
    runner
        .run(&coverage.command())
        .map_err(|failure| Failure::from(failure.to_string()))?;
    read_file(runner, &coverage.report())
}

/// Checks the coverage summary of a TypeScript package (Vitest `json-summary`).
///
/// 100% of lines, statements, functions and branches. Lines, statements and functions must
/// be measured (a report of nothing fails); branches may number zero, in code without any.
///
/// # Errors
///
/// Returns a message naming the package: the summary is not a valid coverage summary,
/// or it lists every kind not fully covered.
pub fn ts_coverage_gate(package: &str, summary: &str) -> Result<(), String> {
    let json: serde_json::Value = serde_json::from_str(summary)
        .map_err(|error| format!("{package}: invalid coverage summary: {error}"))?;
    let mut missing = Vec::new();
    for kind in ["lines", "statements", "functions", "branches"] {
        let field = |name: &str| {
            json.get("total")
                .and_then(|total| total.get(kind))
                .and_then(|entry| entry.get(name))
                .and_then(serde_json::Value::as_u64)
        };
        match (field("total"), field("covered")) {
            // Code without a branch has none to cover; lines, statements and functions
            // must still be measured, so an empty report still fails.
            (Some(0), _) if kind == "branches" => {}
            (Some(0), _) => missing.push(format!("{kind} not measured")),
            (Some(count), Some(covered)) if covered == count => {}
            (Some(count), Some(covered)) => missing.push(format!("{kind} {covered}/{count}")),
            _ => missing.push(format!("no {kind} total")),
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{package}: coverage is not 100%: {}",
            missing.join(", ")
        ))
    }
}
