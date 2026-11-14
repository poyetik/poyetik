//! Seeded property-test runners that read nothing from the environment.

use proptest::test_runner::{Config, RngAlgorithm, RngSeed, TestRng, TestRunner};

/// A proptest configuration running `cases` cases.
///
/// `Config::default()` reads `PROPTEST_*` environment variables; every field it reads is
/// set here explicitly, so that the environment never changes what a run does, not even
/// how a failure is shrunk or reported. Failures are not persisted to files either.
#[must_use]
pub fn config(cases: u32) -> Config {
    Config {
        cases,
        max_local_rejects: 65_536,
        max_global_rejects: 1_024,
        max_flat_map_regens: 1_000_000,
        failure_persistence: None,
        max_shrink_time: 0,
        max_shrink_iters: 1_024,
        max_default_size_range: 100,
        verbose: 0,
        rng_algorithm: RngAlgorithm::ChaCha,
        rng_seed: RngSeed::Fixed(0),
        ..Config::default()
    }
}

/// A runner of `cases` cases with [`config`] and a fixed seed: the same cases, in the
/// same order, at every run. Each [`TestRunner::run`] on it continues the random stream,
/// so successive runs try new cases.
#[must_use]
pub fn runner(cases: u32) -> TestRunner {
    TestRunner::new_with_rng(
        config(cases),
        TestRng::deterministic_rng(RngAlgorithm::ChaCha),
    )
}

#[cfg(test)]
#[path = "runner_tests.rs"]
mod tests;
