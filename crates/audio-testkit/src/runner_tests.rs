//! Tests of the property test runner's fixed configuration.

use std::process::Command;

use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::{Config, RngAlgorithm, RngSeed, TestRunner};

use super::{config, fork, runner};

/// Set in the environment of the child process of `ignores_the_proptest_environment`.
const CHILD: &str = "ENGINE_TESTKIT_PROPTEST_CHILD";

/// Checks every field of `config(7)` ([`config`]), the environment could set any.
fn assert_pinned(pinned: &Config) {
    assert_eq!(pinned.cases, 7, "cases");
    assert_eq!(pinned.max_local_rejects, 65_536, "local rejects");
    assert_eq!(pinned.max_global_rejects, 1_024, "global rejects");
    assert_eq!(
        pinned.max_flat_map_regens, 1_000_000,
        "flat map regenerations"
    );
    assert!(pinned.failure_persistence.is_none(), "no failure file");
    assert_eq!(pinned.max_shrink_time, 0, "no shrink time limit");
    assert_eq!(pinned.max_shrink_iters, 1_024, "shrink iterations");
    assert_eq!(pinned.max_default_size_range, 100, "default size range");
    assert_eq!(pinned.verbose, 0, "quiet");
    assert_eq!(pinned.rng_algorithm, RngAlgorithm::ChaCha, "algorithm");
    assert_eq!(pinned.rng_seed, RngSeed::Fixed(0), "seed");
}

#[test]
fn pins_every_field_the_environment_could_set() {
    assert_pinned(&config(7));
}

/// The child side of `ignores_the_proptest_environment`; does nothing in a normal run.
#[test]
fn child_reads_the_configuration_under_a_hostile_environment() {
    if std::env::var_os(CHILD).is_some() {
        assert_pinned(&config(7));
    }
}

#[test]
fn ignores_the_proptest_environment() {
    // This test binary again, running only the child test, with every variable
    // proptest reads set to a value other than the pinned one.
    let output = Command::new(std::env::current_exe().expect("the test binary"))
        .args([
            "--exact",
            "runner::tests::child_reads_the_configuration_under_a_hostile_environment",
            "--test-threads=1",
        ])
        .env(CHILD, "1")
        .env("PROPTEST_CASES", "3")
        .env("PROPTEST_MAX_LOCAL_REJECTS", "3")
        .env("PROPTEST_MAX_GLOBAL_REJECTS", "3")
        .env("PROPTEST_MAX_FLAT_MAP_REGENS", "3")
        .env("PROPTEST_MAX_SHRINK_TIME", "3")
        .env("PROPTEST_MAX_SHRINK_ITERS", "3")
        .env("PROPTEST_MAX_DEFAULT_SIZE_RANGE", "3")
        .env("PROPTEST_VERBOSE", "2")
        .env("PROPTEST_RNG_ALGORITHM", "xs")
        .env("PROPTEST_RNG_SEED", "3")
        .output()
        .expect("the test binary runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}");
    assert!(stdout.contains("1 passed"), "the child test ran: {stdout}");
}

#[test]
fn draws_the_same_values_at_every_run_and_new_ones_after() {
    let draw = |runner: &mut TestRunner| {
        (0_u64..u64::MAX)
            .new_tree(runner)
            .expect("a value")
            .current()
    };
    let (mut first, mut second) = (runner(1), runner(1));
    let start = draw(&mut first);
    assert_eq!(start, draw(&mut second), "same seed, same value");
    assert_ne!(start, draw(&mut first), "the stream goes on");
    assert_eq!(runner(3).config().cases, 3, "the cases are passed on");
}

/// The values `runner` tries in one run of a property that always holds.
fn tried(runner: &mut TestRunner) -> Vec<u64> {
    let values = core::cell::RefCell::new(Vec::new());
    runner
        .run(&(0_u64..u64::MAX), |value| {
            values.borrow_mut().push(value);
            Ok(())
        })
        .expect("a property that holds");
    values.into_inner()
}

#[test]
fn runs_its_cases_once_and_each_fork_runs_new_ones() {
    let mut parent = runner(3);
    let first = tried(&mut parent);
    assert_eq!(first.len(), 3, "the cases of the first run");
    assert_eq!(
        tried(&mut parent),
        Vec::<u64>::new(),
        "a second run tries none"
    );
    let forked = tried(&mut fork(&mut parent, 3));
    let again = tried(&mut fork(&mut parent, 3));
    assert_eq!([forked.len(), again.len()], [3, 3], "each fork its cases");
    assert!(
        forked != first && again != forked && again != first,
        "new values: {first:?}, {forked:?}, {again:?}"
    );
    let mut replayed = runner(3);
    assert_eq!(tried(&mut replayed), first, "the same first run");
    assert_eq!(
        tried(&mut fork(&mut replayed, 3)),
        forked,
        "the same at every run"
    );
    assert_eq!(
        fork(&mut parent, 5).config(),
        &config(5),
        "the configuration"
    );
}
