//! What the unit tests of several modules of the crate share.

/// Runs each generic test in both precisions, each instantiation a test of its own: every
/// instantiation of the generic code is covered on its own.
macro_rules! in_both_precisions {
    ($($test:ident),* $(,)?) => {
        /// The tests in binary64, the default precision.
        mod binary64 {
            $(
                #[test]
                fn $test() {
                    super::$test::<crate::Binary64>();
                }
            )*
        }
        /// The tests in binary32.
        mod binary32 {
            $(
                #[test]
                fn $test() {
                    super::$test::<crate::Binary32>();
                }
            )*
        }
    };
}

pub(crate) use in_both_precisions;

/// The larger of two numbers, the first when they compare equal: `f64::max` is banned
/// (`clippy.toml`), since it may return either zero when they are `+0` and `-0`, a sign that
/// would differ between targets.
pub(crate) fn larger(first: f64, second: f64) -> f64 {
    if second > first { second } else { first }
}

#[test]
fn takes_the_larger_number_and_the_first_of_equal_ones() {
    assert_eq!(
        larger(1.0_f64, 2.0_f64).to_bits(),
        2.0_f64.to_bits(),
        "second"
    );
    assert_eq!(
        larger(2.0_f64, 1.0_f64).to_bits(),
        2.0_f64.to_bits(),
        "first"
    );
    assert_eq!(
        larger(0.0_f64, -0.0_f64).to_bits(),
        0.0_f64.to_bits(),
        "+0 first"
    );
    assert_eq!(
        larger(-0.0_f64, 0.0_f64).to_bits(),
        (-0.0_f64).to_bits(),
        "-0 first"
    );
}

/// The margin the limiter adds to every need above the ceiling, at `frames` of maximum
/// lookahead: `64 × frames × 2^-53` (`spec/measurements.md`, "The true-peak limiter"),
/// computed apart.
pub(crate) fn margin_of(frames: usize) -> f64 {
    64.0_f64 * f64::from(u32::try_from(frames).expect("small")) * libm::exp2(-53.0_f64)
}

/// The control path's rounding bound at a lookahead of `frames`, `(43 frames + 3) × 2^-53`
/// (`spec/measurements.md`, "The true-peak limiter"), computed apart.
pub(crate) fn rounding_of(frames: usize) -> f64 {
    (43.0_f64 * f64::from(u32::try_from(frames).expect("small")) + 3.0_f64) * libm::exp2(-53.0_f64)
}

/// How `detector` reads a constant, per unit of it: the largest of its candidates, the
/// interval's own samples (gain 1) and each phase of its interpolator (the sum of its taps;
/// `spec/measurements.md`, "The true-peak limiter"), computed from its coefficients once it
/// lies within 2^-40 of the value stated apart: 1 + 13/8192 for the standard detector, whose
/// outer phases sum BS.1770's taps to that, and 1 for the windowed ones, whose phases sum to
/// 1 within a few ulps.
pub(crate) fn constant_reading(detector: crate::Detector) -> f64 {
    let computed = detector
        .coefficients()
        .chunks_exact(detector.taps())
        .map(|phase| {
            phase
                .iter()
                .fold(0.0_f64, |sum, tap| sum + crate::host::Raw::raw(*tap))
        })
        .fold(1.0_f64, larger);
    let stated = match detector {
        crate::Detector::Standard => 1.0_f64 + 13.0_f64 / 8_192.0_f64,
        crate::Detector::Precise | crate::Detector::Mastering => 1.0_f64,
    };
    assert!(
        (computed - stated).abs() <= libm::exp2(-40.0_f64),
        "{detector:?} reads a constant {computed} times, {stated} stated"
    );
    computed
}

/// The steady output of a constant `level` above every need held before it, through a
/// limiter of `detector` with `maximum` frames of maximum lookahead under the ceiling of
/// amplitude `ceiling`, once its window and smoothing hold that constant's need alone, and
/// how far the computed output may lie from it.
///
/// The need is `1 - c / (g·level) + E` (`g` the detector's reading of a constant,
/// [`constant_reading`]; `E` the margin, [`margin_of`]), every stage holds it, so the output
/// is `level × (c / (g·level) - E) = c/g - level·E`; computed, within `level·ρ` (the control
/// path's rounding, [`rounding_of`], which a gain near zero carries whole) and `2^-20·c`,
/// above the roundings of the ceiling to binary32, the gain and its product in either
/// precision (each within 2^-24 of it). A gain stuck low after a change, by far less than a
/// decibel, lies outside it.
pub(crate) fn exact_steady(
    ceiling: f64,
    detector: crate::Detector,
    maximum: usize,
    level: f64,
) -> (f64, f64) {
    (
        ceiling / constant_reading(detector) - level * margin_of(maximum),
        level * rounding_of(maximum) + libm::exp2(-20.0_f64) * ceiling,
    )
}

#[test]
fn reads_a_constant_through_each_detector_as_stated() {
    assert_eq!(
        constant_reading(crate::Detector::Standard).to_bits(),
        1.001_586_914_062_5_f64.to_bits(),
        "the largest sum of BS.1770's phases, exactly"
    );
    for detector in [crate::Detector::Precise, crate::Detector::Mastering] {
        assert!(
            (constant_reading(detector) - 1.0_f64).abs() <= libm::exp2(-48.0_f64),
            "{detector:?}"
        );
    }
}
