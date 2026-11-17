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

/// How far the Hann sums of a window of `length` frames, whose largest value is `largest`,
/// may lie from a reference computed apart in binary64, the plain sum's bound and each of
/// the turning sum's parts', computed from their derivation (`spec/measurements.md`, "The
/// true-peak limiter", step 4), with `u = 2^-53`, for sums exact at most `length - 1`
/// frames before, as the limiter keeps them:
/// - the plain sum: `L(L - 1)uV` accumulated, `2(L + 1)uV` per slide, `L - 1` slides;
/// - the turning parts: `L(L + 20)uV` accumulated, `(38L + 37)uV` per slide, carried by
///   `(1 + 29u)` per frame (below `1 + 2^-40` up to 1,920 frames);
/// - the reference's own error: its plain sum of `L` terms `L(L - 1)uV`, and its turning
///   parts `L(L + 20)uV` (the angle's three roundings, libm within an ulp, a product and a
///   sum of `L` terms per term, as for the accumulated sums).
///
/// All times `1 + 2^-40` for the sums' error bounds of `L` terms written to first order
/// (`γ_{L-1}` against `(L - 1)u`, below `2^-41` of it).
pub(crate) fn hann_sums_bounds(length: usize, largest: f64) -> (f64, f64) {
    let frames = f64::from(u32::try_from(length).expect("small"));
    let unit = libm::exp2(-53.0_f64) * largest * (1.0_f64 + libm::exp2(-40.0_f64));
    let accumulated = frames * (frames - 1.0_f64);
    let slides = 2.0_f64 * (frames - 1.0_f64) * (frames + 1.0_f64);
    let turning = frames * (frames + 20.0_f64);
    let turned = (frames - 1.0_f64) * (38.0_f64 * frames + 37.0_f64);
    (
        (accumulated + slides + accumulated) * unit,
        ((turning + turned) * (1.0_f64 + libm::exp2(-40.0_f64)) + turning) * unit,
    )
}

#[test]
fn bounds_the_hann_sums_by_their_derivation() {
    // Computed apart at one and four frames: `(0 + 0 + 0)`, `(21 + 0)(1 + 2^-40) + 21`; and
    // `(12 + 30 + 12)`, `(96 + 3 × 189)(1 + 2^-40) + 96`, in units of `2^-53` times the
    // largest value and `1 + 2^-40`.
    let unit = libm::exp2(-53.0_f64) * 2.0_f64 * (1.0_f64 + libm::exp2(-40.0_f64));
    let grown = 1.0_f64 + libm::exp2(-40.0_f64);
    for (length, plain, turning) in [
        (1, 0.0_f64, 21.0_f64 * grown + 21.0_f64),
        (4, 54.0_f64, 663.0_f64 * grown + 96.0_f64),
    ] {
        assert_eq!(
            hann_sums_bounds(length, 2.0_f64),
            (plain * unit, turning * unit),
            "{length} frames"
        );
    }
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

/// A protected limiter at 48 kHz for `channels` channels, with the standard detector, a
/// maximum lookahead and a lookahead of 0.5 ms (24 frames, a latency of 30), a release of
/// 100 ms, an adaptivity of 0.5 and a ceiling of `decibels` dBTP: the one the tests of the
/// plan, the renderer and the output stage limit with.
pub(crate) fn limiter_at<S: crate::Sample>(
    channels: usize,
    decibels: f64,
) -> crate::TruePeakLimiter<S> {
    crate::TruePeakLimiter::new(
        crate::SampleRate::new(48_000).expect("a rate"),
        u16::try_from(channels).expect("few channels"),
        crate::Binary64::new(0.5_f64),
        crate::Detector::Standard,
        crate::LimiterSettings::new(
            crate::Binary64::new(decibels),
            crate::Binary64::new(0.5_f64),
            crate::Binary64::new(100.0_f64),
            crate::Binary64::new(0.5_f64),
        )
        .expect("settings"),
        crate::LimiterMode::Protected,
    )
    .expect("a limiter")
}

/// [`limiter_at`] at the default ceiling, -1 dBTP.
pub(crate) fn limiter<S: crate::Sample>(channels: usize) -> crate::TruePeakLimiter<S> {
    limiter_at(channels, -1.0_f64)
}

/// Deterministic draws of 64-bit patterns, without a dependency: `SplitMix64` (Steele, Lea
/// and Flood, 2014), as `spec/measurements.md` defines it. The one copy of this crate's
/// tests (the math layer's, the reduction tests and the limiter's draw from it); `audio-core` cannot use the battery's
/// (`audio_battery::splitmix64`), which depends on it.
pub(crate) mod splitmix {
    /// The state of the generator.
    pub(crate) struct Draws(pub(crate) u64);

    impl Draws {
        /// A generator from a fixed seed.
        pub(crate) const fn new() -> Self {
            Self(0x5059_0006)
        }

        /// The next 64 bits.
        pub(crate) const fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut mixed = self.0;
            mixed = (mixed ^ (mixed >> 30_u32)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            mixed = (mixed ^ (mixed >> 27_u32)).wrapping_mul(0x94d0_49bb_1331_11eb);
            mixed ^ (mixed >> 31_u32)
        }
    }
}

#[test]
fn draws_splitmix64_from_its_published_values() {
    // SplitMix64's first outputs from the state 0, computed apart from this code.
    let mut draws = splitmix::Draws(0);
    assert_eq!(
        [draws.next(), draws.next()],
        [0xe220_a839_7b1d_cdaf, 0x6e78_9e6a_a1b9_65f4]
    );
}
