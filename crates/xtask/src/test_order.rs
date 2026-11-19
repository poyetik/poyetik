//! The order of the tests in mutant runs: each Cargo workspace's slow group, run last.
//!
//! Each mutant's tests stop at their first failure ([`MUTANT_FAIL_FAST`](crate::MUTANT_FAIL_FAST)),
//! so the tests that run first decide how long a caught mutant takes: a mutant a fast test
//! catches waited behind every slow test nextest started before it (measured on the full
//! mutation run of 2026-11-18: 237 of audio-core's 780 mutants took about 100 s each, the
//! limiter's tests running before the 0.02 s test that caught them). Each workspace lists
//! its slow tests in its nextest configuration ([`CargoWorkspace::nextest_config`]), each
//! with a lower priority, so that nextest starts them after every other test.
//!
//! A priority only orders: every test still runs when none fails, so no verdict depends on
//! the order, only how long a run takes. The configuration holds nothing else, which
//! [`nextest_configs_gate`] proves; nextest itself refuses an entry naming a test binary
//! the workspace does not build, before any test runs. The group is kept current from the
//! measurements: each mutation run ends with [`order_suggestion`], from the times of its
//! unmutated baseline.

use alloc::collections::BTreeSet;

use crate::{
    steps::{Failure, step_verdict},
    workspace::CargoWorkspace,
};

/// The header of each entry of the slow group: one nextest override of the default profile,
/// the only profile cargo-mutants runs.
pub const OVERRIDE_HEADER: &str = "[[profile.default.overrides]]";

/// The priority of every test of the slow group: below the default 0, so that nextest
/// starts it after every other test of the run.
pub const SLOW_PRIORITY_LINE: &str = "priority = -1";

/// The least time, in milliseconds, a test of the slow group took in the baseline it was
/// chosen from: half a second.
///
/// Chosen by replaying the full run of 2026-11-18 (3,667 caught mutants, each one's first
/// failing test placed in the order nextest would follow on 10 threads, the times of its
/// unmutated baseline): alphabetical order, 7,988 s of tests until the first failure; slow
/// group from 2 s, 2,795 s; from 1 s, 2,178 s; from 0.5 s, 1,688 s (119 tests); from
/// 0.25 s, 1,355 s (145 tests). Half a second keeps most of the gain with a short list.
pub const SLOW_TEST_MILLIS: u64 = 500;

/// The least baseline time, in milliseconds, for which a test outside the slow group is
/// suggested into it: twice [`SLOW_TEST_MILLIS`].
///
/// With [`FAST_TEST_MILLIS`], a band around the group's threshold where no move is
/// suggested: the same test's time varies between runs with the load (the limiter's tests
/// took 58 s in a baseline and 86 s in a mutant's run), and a factor of two either way
/// keeps that noise from moving a test in and out at each run.
pub const SLOW_TEST_SUGGESTED_MILLIS: u64 = 1_000;

/// The baseline time, in milliseconds, under which a test of the slow group is suggested
/// out of it: half of [`SLOW_TEST_MILLIS`] (see [`SLOW_TEST_SUGGESTED_MILLIS`]).
pub const FAST_TEST_MILLIS: u64 = 250;

/// A test, as nextest names it: its test binary, then its name in that binary.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TestName {
    /// The binary's id (`audio-core`, `audio-testkit::plan_laws`).
    pub binary: String,
    /// The test's path in the binary (`limiter::tests::holds_the_ceiling_in_each_precision`).
    pub path: String,
}

/// Whether `name` can be a binary id or a test path of the slow group: ASCII letters,
/// digits, `_`, `-` and `:` only, at least one, so that it holds nothing a filter would
/// read as an operator.
fn plain(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_-:".contains(character))
}

/// The test the filter line `line` names exactly, from `filter = 'binary_id(=<binary>) &
/// test(=<test>)'`, or none.
fn filtered_test(line: &str) -> Option<TestName> {
    let inner = line
        .strip_prefix("filter = 'binary_id(=")?
        .strip_suffix(")'")?;
    let (binary, path) = inner.split_once(") & test(=")?;
    (plain(binary) && plain(path)).then(|| TestName {
        binary: binary.to_owned(),
        path: path.to_owned(),
    })
}

/// The slow group of the nextest configuration `config`, read from `path`: every test it
/// names, in order.
///
/// The configuration holds comments, blank lines and entries only, each entry exactly three
/// lines: [`OVERRIDE_HEADER`], a filter naming one test exactly (`filter =
/// 'binary_id(=<binary>) & test(=<test>)'`) and [`SLOW_PRIORITY_LINE`]; the entries in
/// strictly increasing order of binary then test, so that none is listed twice. Nothing
/// else: no other setting of nextest in the file (retries, timeouts, threads, a default
/// filter) can change which tests run or what they decide. Its `NEXTEST_*` environment
/// variables are not cleared from the mutant runs yet (roadmap #24 step 164).
///
/// # Errors
///
/// Returns each line that breaks that form, with its number.
pub fn slow_group(path: &str, config: &str) -> Result<Vec<TestName>, Failure> {
    let mut lines = config
        .lines()
        .zip(1_usize..)
        .filter(|(line, _)| !line.is_empty() && !line.starts_with('#'));
    let mut group: Vec<TestName> = Vec::new();
    let mut problems = Vec::new();
    while let Some((header, number)) = lines.next() {
        if header != OVERRIDE_HEADER {
            problems.push(format!(
                "{path}:{number}: `{header}`, where an entry starts with `{OVERRIDE_HEADER}`: \
                 the file holds the slow group only"
            ));
            continue;
        }
        let filter = lines.next();
        let priority = lines.next();
        match (filter, priority) {
            (Some((filter, at)), Some((priority, then))) => {
                let named = filtered_test(filter);
                if named.is_none() {
                    problems.push(format!(
                        "{path}:{at}: `{filter}`, where the entry names one test exactly: \
                         `filter = 'binary_id(=<binary>) & test(=<test>)'`"
                    ));
                }
                if priority != SLOW_PRIORITY_LINE {
                    problems.push(format!(
                        "{path}:{then}: `{priority}`, where the entry ends with \
                         `{SLOW_PRIORITY_LINE}`"
                    ));
                }
                if let Some(named) = named {
                    if group.last().is_some_and(|last| *last >= named) {
                        problems.push(format!(
                            "{path}:{at}: {} {} out of order: the entries are sorted by \
                             binary then test, each once",
                            named.binary, named.path
                        ));
                    }
                    group.push(named);
                }
            }
            _ => problems.push(format!(
                "{path}:{number}: an entry without its filter and priority lines"
            )),
        }
    }
    step_verdict(problems).map(|()| group)
}

/// The time `text` (`12.345`, seconds with nextest's three decimals) says, in milliseconds,
/// or none.
fn millis(text: &str) -> Option<u64> {
    let (seconds, thousandths) = text.split_once('.')?;
    let digits = |part: &str| part.bytes().all(|byte| byte.is_ascii_digit());
    if seconds.is_empty() || thousandths.len() != 3 || !digits(seconds) || !digits(thousandths) {
        return None;
    }
    seconds
        .parse::<u64>()
        .ok()?
        .checked_mul(1_000)?
        .checked_add(thousandths.parse::<u64>().ok()?)
}

/// The time of every test that passed in the nextest output `log`, in milliseconds.
///
/// `log` is cargo-mutants' log of its unmutated baseline, read from its lines `PASS [
/// <seconds>s] (<n>/<m>) <binary> <test>`; every other line is skipped, so that a log
/// nextest wrote in another form yields fewer times, never wrong ones.
#[must_use]
pub fn baseline_times(log: &str) -> Vec<(TestName, u64)> {
    log.lines()
        .filter_map(|line| {
            let rest = line.trim_start().strip_prefix("PASS [")?;
            let (seconds, rest) = rest.split_once("s] (")?;
            let (_, rest) = rest.split_once(") ")?;
            let mut words = rest.split_whitespace();
            let (binary, path) = (words.next()?, words.next()?);
            if words.next().is_some() {
                return None;
            }
            Some((
                TestName {
                    binary: binary.to_owned(),
                    path: path.to_owned(),
                },
                millis(seconds.trim_start())?,
            ))
        })
        .collect()
}

/// `millis` milliseconds as seconds, with three decimals.
fn seconds(millis: u64) -> String {
    format!(
        "{}.{:03}",
        millis.div_euclid(1_000),
        millis.rem_euclid(1_000)
    )
}

/// What a mutation run suggests of the slow group `group` of `config`, from the `times` of
/// its baseline.
///
/// One line per test to move, after a line saying how many and what they cost; nothing
/// when the group is as measured.
///
/// Into the group, a test outside it that took [`SLOW_TEST_SUGGESTED_MILLIS`] or more; out
/// of it, a test of the group that took under [`FAST_TEST_MILLIS`], or that no longer
/// runs: absent from a baseline that ran its binary (a baseline runs only the tests of the
/// packages a partial run mutates, so the group's other binaries are not judged). The
/// gain stated is the time of the tests to move in, which a mutant a faster test catches
/// may wait behind.
#[must_use]
pub fn order_suggestion(
    config: &str,
    group: &[TestName],
    times: &[(TestName, u64)],
) -> Vec<String> {
    let listed: BTreeSet<&TestName> = group.iter().collect();
    let measured: BTreeSet<&str> = times.iter().map(|(name, _)| name.binary.as_str()).collect();
    let measured_tests: BTreeSet<&TestName> = times.iter().map(|(name, _)| name).collect();
    let mut moves = Vec::new();
    let mut gain: u64 = 0;
    for (name, millis) in times {
        let slow = listed.contains(name);
        if !slow && *millis >= SLOW_TEST_SUGGESTED_MILLIS {
            gain = gain.saturating_add(*millis);
            moves.push(format!(
                "    run last: {} {} ({} s)",
                name.binary,
                name.path,
                seconds(*millis)
            ));
        } else if slow && *millis < FAST_TEST_MILLIS {
            moves.push(format!(
                "    run with the others: {} {} ({} s)",
                name.binary,
                name.path,
                seconds(*millis)
            ));
        }
    }
    for name in group {
        if measured.contains(name.binary.as_str()) && !measured_tests.contains(name) {
            moves.push(format!(
                "    remove, no longer run: {} {}",
                name.binary, name.path
            ));
        }
    }
    if moves.is_empty() {
        return moves;
    }
    let count = moves.len();
    let tests = if count == 1 { "test" } else { "tests" };
    let mut lines = vec![format!(
        "  test order: {count} {tests} to move in the slow group of {config} (moving them in \
         saves up to {} s a mutant a faster test catches waits; a test slower than measured \
         under load may rather need speeding up):",
        seconds(gain)
    )];
    lines.extend(moves);
    lines
}

/// Checks the nextest configurations of the repository.
///
/// The tracked files named like one (`tracked`, every path ending with `nextest.toml`) are
/// exactly those of [`CargoWorkspace::ALL`], and each of `configs` (path, text) holds only
/// its slow group ([`slow_group`]).
///
/// # Errors
///
/// Returns every nextest configuration elsewhere, every one missing, and every line of one
/// that holds anything but its slow group.
pub fn nextest_configs_gate(
    configs: &[(String, String)],
    tracked: &[String],
) -> Result<(), Failure> {
    let expected: BTreeSet<&str> = CargoWorkspace::ALL
        .iter()
        .map(|workspace| workspace.nextest_config())
        .collect();
    let mut problems: Vec<String> = tracked
        .iter()
        .filter(|path| !expected.contains(path.as_str()))
        .map(|path| {
            format!(
                "{path}: a nextest configuration outside a workspace's own \
                 (`.config/nextest.toml` at its root), which no gate reads"
            )
        })
        .collect();
    problems.extend(
        expected
            .iter()
            .filter(|path| !tracked.iter().any(|tracked| tracked == *path))
            .map(|path| {
                format!("{path}: missing, or not tracked by git: the workspace's slow group")
            }),
    );
    for (path, text) in configs {
        if let Err(failure) = slow_group(path, text) {
            if failure.details.is_empty() {
                problems.push(failure.summary);
            } else {
                problems.extend(failure.details);
            }
        }
    }
    step_verdict(problems)
}

#[cfg(test)]
#[path = "test_order_tests.rs"]
mod tests;
