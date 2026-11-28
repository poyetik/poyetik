//! The functions a change touches, mutated whole.
//!
//! cargo-mutants' `--in-diff` keeps a mutant only when its own span touches a line the diff
//! adds, or one beside a line it deletes. A body moved into a new function, or a function whose
//! signature alone changed, keeps its lines as context, and none of its mutants is kept: the
//! code a refactor touched goes unmutated. So every function holding an affected line (its
//! signature included) is marked added whole, through the one interface `--in-diff` documents:
//! a section of the diff whose added lines are that function's lines, each replaced by itself,
//! so that its new text is the source's, as cargo-mutants checks it is.

use alloc::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::{changes::rust_file, steps::Failure};

/// The header git writes before each file's section of a diff.
const SECTION: &str = "diff --git ";

/// One file's section of a unified diff: the path of its new side (none for a file deleted)
/// and its text, from its header line to the next one.
#[derive(Debug, PartialEq, Eq)]
struct Section<'diff> {
    /// The path of its new side, without git's `b/`.
    path: Option<&'diff str>,
    /// Whether it adds the file whole (`--- /dev/null`).
    created: bool,
    /// Its text, every line ended by a line feed.
    text: &'diff str,
}

/// The sections of `diff`, in order: the text before the first header, if any, is none.
fn sections(diff: &str) -> Vec<Section<'_>> {
    let mut starts: Vec<usize> = diff
        .match_indices(SECTION)
        .map(|(at, _)| at)
        .filter(|&at| at == 0 || diff.get(..at).is_some_and(|before| before.ends_with('\n')))
        .collect();
    starts.push(diff.len());
    starts
        .windows(2)
        .filter_map(|bounds| {
            let [start, end] = *bounds else { return None };
            let text = diff.get(start..end)?;
            let path = text
                .lines()
                .find_map(|line| line.strip_prefix("+++ "))
                .and_then(|path| path.strip_prefix("b/"));
            let created = text.lines().any(|line| line == "--- /dev/null");
            Some(Section {
                path,
                created,
                text,
            })
        })
        .collect()
}

/// The paths of the Rust files `diff` changes in part (not added whole): those whose functions
/// may hold an affected line but not every line.
pub(crate) fn partly_changed(diff: &str) -> Vec<String> {
    sections(diff)
        .into_iter()
        .filter(|section| !section.created)
        .filter_map(|section| section.path)
        .filter(|path| rust_file(path))
        .map(str::to_owned)
        .collect()
}

/// The lines of the new side a section's hunks affect, as cargo-mutants 27 reads them
/// (`in_diff.rs`, `affected_lines`): each line added, the line before a deletion, and the line
/// after it.
fn affected(section: &str) -> BTreeSet<usize> {
    let mut lines = BTreeSet::new();
    // The line of the new side the next line of a hunk is, from the first hunk on: the
    // section's header lines (`--- a/…`, `+++ b/…`) come before it.
    let mut at: Option<usize> = None;
    let mut removed = false;
    for line in section.lines() {
        if let Some(header) = line.strip_prefix("@@ ") {
            at = Some(new_start(header).unwrap_or_default());
            removed = false;
            continue;
        }
        let (Some(at), Some(first)) = (at.as_mut(), line.chars().next()) else {
            continue;
        };
        match first {
            '-' => {
                if *at > 1 {
                    let _added = lines.insert(at.saturating_sub(1));
                }
                removed = true;
            }
            '+' => {
                let _added = lines.insert(*at);
                removed = false;
                *at = at.saturating_add(1);
            }
            ' ' => {
                if removed {
                    let _added = lines.insert(*at);
                    removed = false;
                }
                *at = at.saturating_add(1);
            }
            _ => {}
        }
    }
    lines
}

/// The first line of a hunk's new side, from its header after `@@ `: `-a,b +c,d @@`.
fn new_start(header: &str) -> Option<usize> {
    let new = header
        .split_whitespace()
        .find_map(|word| word.strip_prefix('+'))?;
    new.split(',').next()?.parse().ok()
}

/// The first and last lines of every function cargo-mutants lists a mutant in, by file, from
/// its `--list --json` output.
///
/// # Errors
///
/// Returns why the listing is not cargo-mutants' JSON: not an array, or a mutant without its
/// file.
pub(crate) fn function_spans(
    listing: &str,
) -> Result<BTreeMap<String, BTreeSet<(usize, usize)>>, String> {
    let value: Value = serde_json::from_str(listing)
        .map_err(|error| format!("cargo-mutants listed no JSON: {error}"))?;
    let Value::Array(mutants) = value else {
        return Err("cargo-mutants listed no array of mutants".to_owned());
    };
    let mut spans: BTreeMap<String, BTreeSet<(usize, usize)>> = BTreeMap::new();
    for mutant in &mutants {
        let file = mutant
            .get("file")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("cargo-mutants listed a mutant without its file: {mutant}"))?;
        let span = mutant
            .get("function")
            .and_then(|function| function.get("span"));
        let line = |end: &str| {
            span.and_then(|span| span.get(end))
                .and_then(|place| place.get("line"))
                .and_then(Value::as_u64)
                .and_then(|line| usize::try_from(line).ok())
        };
        // A mutant outside every function (a constant's value) has no span of one: its own
        // lines decide whether the diff keeps it.
        if let (Some(first), Some(last)) = (line("start"), line("end")) {
            let _added = spans
                .entry(file.to_owned())
                .or_default()
                .insert((first, last));
        }
    }
    Ok(spans)
}

/// The lines of `text` a section marks added: those `affected` and every line of each
/// function of `spans` holding one of them.
fn widened(
    affected: &BTreeSet<usize>,
    spans: &BTreeSet<(usize, usize)>,
) -> (BTreeSet<usize>, usize) {
    let touched: Vec<&(usize, usize)> = spans
        .iter()
        .filter(|&&(first, last)| affected.range(first..=last).next().is_some())
        .collect();
    let mut lines = affected.clone();
    for &&(first, last) in &touched {
        lines.extend(first..=last);
    }
    (lines, touched.len())
}

/// A section of `path` whose added lines are `lines` of `text`, each replaced by itself, in one
/// hunk per run of consecutive lines; a line past the end of `text` is left out.
fn self_section(path: &str, text: &str, lines: &BTreeSet<usize>) -> String {
    let source: Vec<&str> = text.lines().collect();
    let mut section = format!("{SECTION}a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n");
    let mut runs: Vec<(usize, Vec<&str>)> = Vec::new();
    for &line in lines {
        let Some(&content) = line.checked_sub(1).and_then(|index| source.get(index)) else {
            continue;
        };
        match runs.last_mut() {
            Some((first, run)) if first.saturating_add(run.len()) == line => run.push(content),
            _ => runs.push((line, vec![content])),
        }
    }
    for (first, run) in runs {
        let range = [first.to_string(), run.len().to_string()].join(",");
        for piece in ["@@ -", range.as_str(), " +", range.as_str(), " @@\n"] {
            section.push_str(piece);
        }
        for content in &run {
            section.push('-');
            section.push_str(content);
            section.push('\n');
        }
        for content in &run {
            section.push('+');
            section.push_str(content);
            section.push('\n');
        }
    }
    section
}

/// `diff` with every function holding an affected line of a file it changes in part marked
/// added whole, from cargo-mutants' `listing` of those files (`--list --json`) and their texts
/// read by `read`; and how many functions it marked. A section without a function touched is
/// kept as it is; every other section is too.
///
/// # Errors
///
/// Returns why the listing is not cargo-mutants' JSON ([`function_spans`]), or why a file
/// cannot be read.
pub(crate) fn widen(
    diff: &str,
    listing: &str,
    read: &mut dyn FnMut(&str) -> Result<String, Failure>,
) -> Result<(String, usize), Failure> {
    let spans = function_spans(listing).map_err(Failure::from)?;
    let none = BTreeSet::new();
    let mut widened_diff = String::new();
    let mut functions = 0_usize;
    for section in sections(diff) {
        let partial = section
            .path
            .filter(|path| !section.created && rust_file(path));
        let Some(path) = partial else {
            widened_diff.push_str(section.text);
            continue;
        };
        let (lines, touched) = widened(&affected(section.text), spans.get(path).unwrap_or(&none));
        if touched == 0 {
            widened_diff.push_str(section.text);
            continue;
        }
        functions = functions.saturating_add(touched);
        widened_diff.push_str(&self_section(path, &read(path)?, &lines));
    }
    Ok((widened_diff, functions))
}

#[cfg(test)]
#[path = "widened_tests.rs"]
mod tests;
