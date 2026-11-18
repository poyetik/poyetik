//! Poyetik test bench.
//!
//! [`check_laws`] runs a box in isolation and checks the laws every box must respect;
//! [`check_registry`] checks a whole registry. Box crates call them on every registered
//! box type, so a new box cannot escape them. [`measure`] gives the objective measurements
//! of a signal (`spec/measurements.md`), by which every box proves its audio quality.
//! The parameter sets and special inputs the laws try are the fingerprint battery's
//! (`audio-battery`), as is the shared test [`matrix`], re-exported here.
//!
//! Every law is checked in both precisions (binary64 and binary32, [`matrix::PRECISIONS`])
//! and at every sample rate of the shared [`matrix`], with the
//! parameters at their minimum, default and maximum, at a value away from the default
//! (see `audio_battery::param_sets`), and at `-0.0` and the smallest normal binary32 value (each
//! where the range accepts it, else the default), given as `audio_core::Params`, as a plan gives
//! them.
//!
//! The laws:
//! - **metadata**: names and descriptions are present, port and parameter names are
//!   unique (a parameter's range is finite and holds its default by construction:
//!   `audio_core::ParamSpec::new`);
//! - **determinism**: two fresh instances fed the same inputs produce bit-identical
//!   outputs, the second with its outputs set to a NaN on entry to every block where the
//!   first has +0: a box writes every sample of every outlet, whatever they hold on entry
//!   (`audio_core::Object::process`); the long run gives them a NaN on entry too, so that a
//!   box that stops writing once fed silence leaves a NaN the finite law sees;
//! - **finite output**: never NaN or infinity, for random inputs in `[-1, 1]`, the
//!   special inputs of the [`matrix`] and a long run (which reveals late overflow, not
//!   slow drift: it has no reference);
//! - **no subnormal output**: every output sample is zero or normal, in every one of those
//!   runs (`spec/engine-design.md`, decision 11: a box flushes its outputs with
//!   [`audio_core::Sample::flush`]). A box's recursive state is out of the laws' reach until box
//!   state can be read (roadmap #12 step 28): each box documents and tests how its own
//!   state stays free of subnormals;
//! - **no heap operation** while processing: no allocation, reallocation or release
//!   (measured by [`CountingAllocator`], which the test binary must install as its
//!   global allocator).

#![forbid(unsafe_code)]
#![doc(test(attr(forbid(unsafe_code))))]

extern crate alloc;

mod laws;
pub mod measure;
pub mod runner;

pub use alloc_counter::{
    CountingAllocator, NotCounting, heap_bytes_during, heap_operations_during,
};
pub use audio_battery::matrix;
pub use laws::{check_laws, check_metadata, check_registry};

use audio_core::host::Raw;
use audio_core::{Binary64, Detector, Sample, SampleRate};

/// `value`, a literal of a test written in binary64, as a sample of the precision `S`
/// (rounded to it when `S` is binary32): the one conversion the unit tests of the laws and
/// the plan laws share.
#[must_use]
pub fn sample_of<S: Sample>(value: f64) -> S {
    S::from_binary64(Binary64::from_raw(value))
}

/// Frames in `milliseconds` at `rate`, rounded to the nearest, at least one.
///
/// The tests' own count of a duration (a limiter's lookahead, its latency), counting whole
/// frames until the next would pass the exact duration less half a frame, apart from the
/// code under test. The one copy of the tests outside audio-core, whose own tests cannot
/// depend on this crate and keep their twin of the same name in its limiter's tests.
///
/// # Panics
///
/// For a duration that is not a number, is negative, or holds more frames than a `u32`
/// counts, refused before any count: no test asks for one, and a count made up for it would
/// size a test's work wrongly.
#[must_use]
pub fn frames_of_duration(milliseconds: f64, rate: SampleRate) -> usize {
    let exact = milliseconds * f64::from(rate.hz()) / 1_000.0_f64;
    assert!(
        exact >= 0.0_f64 && exact <= f64::from(u32::MAX),
        "a duration of {milliseconds} ms at {} Hz: not a number, negative or past the count",
        rate.hz()
    );
    // Every count whose next half frame is still within the duration: the count itself.
    (0_u32..=u32::MAX)
        .take_while(|count| f64::from(*count) + 0.5_f64 <= exact)
        .count()
        .max(1)
}

/// The frames `detector` looks behind its input, half its taps.
///
/// Written apart as literals (6, 16 and 64): the tests' own count of a limiter's latency. A
/// detector added to [`Detector::ALL`] stops this from compiling until its delay is written
/// here.
#[must_use]
pub fn detector_delay(detector: Detector) -> usize {
    let [standard, precise, _mastering] = Detector::ALL;
    if detector == standard {
        6
    } else if detector == precise {
        16
    } else {
        64
    }
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
