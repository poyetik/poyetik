use core::f64::consts::TAU;

use engine_core::SampleRate;
use serde_json::Value;

use super::{
    CONDITIONING_MAX, Component, FLOOR, HARMONICS, Harmonics, JACOBI_ITERATIONS, MAX_FRAMES,
    MeasureError, Measurements, Quality, Sum, Unknown, basis, components, cycles, measure,
    sine_bounds, solve, spectrum, split, unknowns, whole_turns,
};

/// The known-answer vectors of the specification, shared with the oracle and computed
/// independently of both (closed forms).
const VECTORS: &str = include_str!("../../../../spec/measurement-vectors.json");

/// The highest sidelobe of the window relative to its peak, -92 dB: what
/// `the_window_keeps_its_sidelobes_92_db_down` measures.
const SIDELOBE: f64 = 2.511_886_431_509_58e-5_f64;

/// A checked sample rate.
fn rate(hz: u32) -> SampleRate {
    SampleRate::new(hz).expect("a supported rate")
}

/// `SplitMix64` (Steele, Lea and Flood, 2014), as `spec/measurements.md` specifies it.
struct SplitMix(u64);

impl SplitMix {
    /// The next value, uniform in `[0, 1)` with 53 bits.
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut mixed = self.0;
        mixed = (mixed ^ (mixed >> 30_u32)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        mixed = (mixed ^ (mixed >> 27_u32)).wrapping_mul(0x94d0_49bb_1331_11eb);
        mixed ^= mixed >> 31_u32;
        // `mixed >> 11` is below 2^53, so exact in f64: split it in two halves of 32 bits.
        let top = mixed >> 11_u32;
        let high = u32::try_from(top >> 32_u32).expect("21 bits");
        let low = u32::try_from(top & 0xffff_ffff).expect("32 bits");
        (f64::from(high) * 4_294_967_296.0_f64 + f64::from(low)) * libm::exp2(-53.0_f64)
    }
}

/// A tone of a vector.
struct Tone {
    /// In hertz.
    frequency: f64,
    /// Peak value.
    amplitude: f64,
    /// In radians.
    phase: f64,
}

/// The signal a vector describes: `Σ a·cos(2π·((f·n) mod rate)/rate + φ)`, plus
/// `step · (u[n] - 1/2)` when it has noise.
fn synthesize(vector: &Value) -> Vec<f64> {
    let hz = vector["rate"].as_f64().expect("a rate");
    let frames = u32::try_from(vector["frames"].as_u64().expect("frames")).expect("u32");
    let tones: Vec<Tone> = vector["tones"]
        .as_array()
        .expect("tones")
        .iter()
        .map(|tone| Tone {
            frequency: tone["frequency"].as_f64().expect("a frequency"),
            amplitude: tone["amplitude"].as_f64().expect("an amplitude"),
            phase: tone["phase"].as_f64().expect("a phase"),
        })
        .collect();
    let noise = vector.get("noise").map(|noise| {
        (
            SplitMix(noise["seed"].as_u64().expect("a seed")),
            noise["step"].as_f64().expect("a step"),
        )
    });
    let mut noise = noise;
    (0..frames)
        .map(|frame| {
            let tones: f64 = tones
                .iter()
                .map(|tone| {
                    let reduced = (tone.frequency * f64::from(frame)) % hz;
                    tone.amplitude * libm::cos(TAU * reduced / hz + tone.phase)
                })
                .sum();
            tones
                + noise.as_mut().map_or(0.0_f64, |(generator, step)| {
                    *step * (generator.next() - 0.5_f64)
                })
        })
        .collect()
}

/// `20 log10(ratio)`.
fn amplitude_db(ratio: f64) -> f64 {
    20.0_f64 * libm::log10(ratio)
}

/// The largest floor of the measurement's fits.
fn floor_of(measured: &Measurements) -> f64 {
    let harmonic = measured
        .harmonics
        .as_ref()
        .map_or(0.0_f64, |harmonics| harmonics.quality.floor);
    if harmonic > measured.sine.floor {
        harmonic
    } else {
        measured.sine.floor
    }
}

/// Checks a level `got` (dB, a ratio of amplitudes to the fundamental `amplitude`) against
/// a vector's `wanted`: "floor" (an exact answer of minus infinity: at most
/// `20 log10(8·floor / amplitude)`, or at least its negation when `rising`), "none" (no
/// such level), or a number, within the floor's effect on both amplitudes, plus `slack`
/// (dB) for a level read from the spectrum.
#[expect(
    clippy::too_many_arguments,
    reason = "a test helper: each argument is one fact of the check"
)]
fn check_level(
    name: &str,
    what: &str,
    got: Option<f64>,
    wanted: &Value,
    floor: f64,
    amplitude: f64,
    rising: bool,
    slack: f64,
) {
    let limit = amplitude_db(8.0_f64 * floor / amplitude);
    match wanted.as_str() {
        Some("none") => assert_eq!(got, None, "{name}: {what}"),
        Some("floor") => {
            let level = got.expect("a level");
            if rising {
                assert!(
                    level >= -limit,
                    "{name}: {what} {level} dB, not below the floor {limit} dB"
                );
            } else {
                assert!(
                    level <= limit,
                    "{name}: {what} {level} dB, not below the floor {limit} dB"
                );
            }
        }
        _ => {
            let expected = wanted.as_f64().expect("a level");
            let level = got.expect("a level");
            let sign = if rising { -1.0_f64 } else { 1.0_f64 };
            let part = amplitude * libm::pow(10.0_f64, sign * expected / 20.0_f64);
            let tolerance =
                amplitude_db((1.0_f64 + floor / part) / (1.0_f64 - floor / amplitude)) + slack;
            assert!(
                (level - expected).abs() <= tolerance,
                "{name}: {what} {level} dB instead of {expected} dB (± {tolerance})"
            );
        }
    }
}

/// The SFDR's slack for a vector's spur: the scalloping loss of its spectrum, and the
/// sidelobes of every other tone (two images each) relative to the spur, in dB.
fn sfdr_slack(vector: &Value, frames: u32, hz: u32, expected: f64, fundamental: f64) -> f64 {
    let spur = fundamental * libm::pow(10.0_f64, -expected / 20.0_f64);
    let others: f64 = vector["tones"]
        .as_array()
        .expect("tones")
        .iter()
        .skip(1)
        .map(|tone| tone["amplitude"].as_f64().expect("an amplitude"))
        .sum();
    let leak = 2.0_f64 * SIDELOBE * others / spur;
    let loss = spectrum(
        &vec![0.0_f64; usize::try_from(frames).expect("small")],
        rate(hz),
    )
    .expect("measurable")
    .scalloping();
    -amplitude_db(1.0_f64 - loss - leak)
}

/// How far the measured fundamental of `vector` over `frames` frames may be from its
/// expectation, for a measurement of floor `floor`.
///
/// Without noise, the fundamental is within the floor. With white noise of step `s`
/// (variance σ² = s²/12), the fitted amplitude is the norm of the two fitted coefficients,
/// so it deviates by at most the norm of their error, whose mean square is 4σ²/N: by
/// Chebyshev, ten times σ·√(2/N) holds it with probability at least 98%, plus the floor.
fn fundamental_tolerance(vector: &Value, frames: u32, floor: f64) -> f64 {
    vector.get("noise").map_or(floor, |noise| {
        let step = noise["step"].as_f64().expect("a step");
        10.0_f64 * step / 12.0_f64.sqrt() * (2.0_f64 / f64::from(frames)).sqrt() + floor
    })
}

/// The fundamental measured in `vector` with every tone scaled by `scale`, its noise
/// unchanged: the fundamental, a folded harmonic included, scales by `scale`.
fn scaled_fundamental(vector: &Value, scale: f64) -> f64 {
    let mut moved = vector.clone();
    for tone in moved["tones"].as_array_mut().expect("tones") {
        let scaled = tone["amplitude"].as_f64().expect("an amplitude") * scale;
        tone["amplitude"] = Value::from(scaled);
    }
    let hz = u32::try_from(vector["rate"].as_u64().expect("a rate")).expect("u32");
    let fundamental = vector["fundamental"].as_f64().expect("a fundamental");
    measure(&synthesize(&moved), rate(hz), fundamental)
        .expect("measurable")
        .fundamental
}

#[test]
fn meets_the_shared_known_answers() {
    let vectors: Value = serde_json::from_str(VECTORS).expect("valid JSON");
    assert_eq!(vectors["scheme"], "measurements/1", "the scheme");
    let list = vectors["vectors"].as_array().expect("vectors");
    assert_eq!(list.len(), 9, "every vector");
    for vector in list {
        let name = vector["name"].as_str().expect("a name");
        let hz = u32::try_from(vector["rate"].as_u64().expect("a rate")).expect("u32");
        let frames = u32::try_from(vector["frames"].as_u64().expect("frames")).expect("u32");
        let fundamental = vector["fundamental"].as_f64().expect("a fundamental");
        let measured = measure(&synthesize(vector), rate(hz), fundamental).expect(name);
        let expected = &vector["expected"];
        let floor = floor_of(&measured);
        let amplitude = expected["fundamental"].as_f64().expect("an amplitude");
        // With noise, a level's tolerance is the confidence interval of the specification
        // (checked below), not the floor.
        let noisy = vector.get("noise").is_some();
        let wanted_floor = if noisy { 0.0_f64 } else { floor };
        let tolerance = fundamental_tolerance(vector, frames, floor);
        let within = |level: f64| (level - amplitude).abs() <= tolerance;
        assert!(
            within(measured.fundamental),
            "{name}: fundamental {} instead of {amplitude}, beyond {tolerance}",
            measured.fundamental
        );
        // The positive control: the fundamental moved by twice the tolerance, measured, is
        // refused.
        let off = scaled_fundamental(vector, (amplitude + 2.0_f64 * tolerance) / amplitude);
        assert!(
            !within(off),
            "{name}: the tolerance accepts a fundamental of {off} measured twice as far"
        );
        if noisy {
            noise_within_its_interval(vector, &measured);
            continue;
        }
        check_level(
            name,
            "THD+N",
            Some(measured.thd_plus_n),
            &expected["thd_plus_n"],
            wanted_floor,
            amplitude,
            false,
            0.0_f64,
        );
        let slack = expected["sfdr"].as_f64().map_or(0.0_f64, |level| {
            sfdr_slack(vector, frames, hz, level, amplitude)
        });
        check_level(
            name,
            "SFDR",
            Some(measured.sfdr),
            &expected["sfdr"],
            wanted_floor,
            amplitude,
            true,
            slack,
        );
        if let Some(error) = expected["harmonics"].as_str() {
            let got = measured.harmonics.expect_err(name);
            assert_eq!(
                format!("{got:?}").split([' ', '{']).next(),
                Some(error),
                "{name}"
            );
        } else {
            let harmonics = measured.harmonics.expect(name);
            let wanted = &expected["harmonics"];
            check_level(
                name,
                "THD",
                harmonics.thd,
                &wanted["thd"],
                floor,
                amplitude,
                false,
                0.0_f64,
            );
            check_level(
                name,
                "aliasing",
                harmonics.aliasing,
                &wanted["aliasing"],
                floor,
                amplitude,
                false,
                0.0_f64,
            );
            check_level(
                name,
                "SNR",
                Some(harmonics.snr),
                &wanted["snr"],
                floor,
                amplitude,
                true,
                0.0_f64,
            );
        }
    }
}

/// Checks the THD+N and SNR of a noisy vector against the confidence interval of the
/// specification ("Known answers with noise"): uniform noise of step `s` has the variance
/// `σ² = s²/12`; the mean square of `N` of its samples has the standard deviation
/// `s²/√(180·N)`, so it lies within 10 of them of `σ²` with a probability of at least 99 %
/// (Chebyshev); a fit of `K` unknowns removes `K·σ²/N` of it on average, at most 100 times
/// that with a probability of at least 99 % (Markov); both at once with a probability of at
/// least 98 % (a union bound).
fn noise_within_its_interval(vector: &Value, measured: &Measurements) {
    let step = vector["noise"]["step"].as_f64().expect("a step");
    let frames = vector["frames"].as_f64().expect("frames");
    let variance = step * step / 12.0_f64;
    let spread = 10.0_f64 * step * step / (180.0_f64 * frames).sqrt();
    let signal = measured.fundamental * measured.fundamental / 2.0_f64;
    let interval = |unknowns: f64| {
        let low = variance - spread - 100.0_f64 * unknowns * variance / frames;
        let high = variance + spread;
        (
            10.0_f64 * libm::log10(signal / high),
            10.0_f64 * libm::log10(signal / low),
        )
    };
    let (lowest, highest) = interval(3.0_f64);
    assert!(
        (lowest..=highest).contains(&-measured.thd_plus_n),
        "THD+N {} outside [{}, {}]",
        measured.thd_plus_n,
        -highest,
        -lowest
    );
    let harmonics = measured.harmonics.as_ref().expect("resolvable");
    let unknowns = f64::from(
        u32::try_from(unknowns(&components(997.0_f64, 48_000.0_f64, 0..=HARMONICS)).len())
            .expect("small"),
    );
    let (low, high) = interval(unknowns);
    assert!(
        (low..=high).contains(&harmonics.snr),
        "SNR {} outside [{low}, {high}]",
        harmonics.snr
    );
    let expected = vector["expected"]["harmonics"]["snr"]
        .as_f64()
        .expect("an SNR");
    assert!(
        (low..=high).contains(&expected),
        "the expectation lies inside"
    );
}

#[test]
fn refuses_the_shared_refusals() {
    let vectors: Value = serde_json::from_str(VECTORS).expect("valid JSON");
    let list = vectors["refusals"].as_array().expect("refusals");
    assert_eq!(list.len(), 7, "every refusal");
    for vector in list {
        let name = vector["name"].as_str().expect("a name");
        let hz = u32::try_from(vector["rate"].as_u64().expect("a rate")).expect("u32");
        let fundamental = vector["fundamental"].as_f64().expect("a fundamental");
        let error = measure(&synthesize(vector), rate(hz), fundamental).expect_err(name);
        let kind = format!("{error:?}");
        assert_eq!(
            kind.split([' ', '{']).next(),
            vector["error"].as_str(),
            "{name}: {kind}"
        );
    }
}

#[test]
fn sums_without_losing_a_term() {
    // Each of these sums loses a term in plain f64 addition, and none with compensation.
    for (terms, wanted) in [
        (
            vec![
                9_007_199_254_740_992.0_f64,
                1.0_f64,
                -9_007_199_254_740_992.0_f64,
            ],
            1.0_f64,
        ),
        (
            vec![
                1.0_f64,
                9_007_199_254_740_992.0_f64,
                1.0_f64,
                -9_007_199_254_740_992.0_f64,
            ],
            2.0_f64,
        ),
        (
            vec![1e16_f64, 1.0_f64, 1.0_f64, 1.0_f64, -1e16_f64],
            3.0_f64,
        ),
        (vec![-0.5_f64, 1e-17_f64, 0.5_f64], 1e-17_f64),
    ] {
        let mut sum = Sum::default();
        for &term in &terms {
            sum.add(term);
        }
        assert_eq!(sum.value().to_bits(), f64::to_bits(wanted), "{terms:?}");
    }
}

#[test]
fn splits_into_two_halves_of_26_bits() {
    for value in [
        1000.1_f64,
        core::f64::consts::PI,
        0.1_f64,
        1e-300_f64,
        123_456_789.123_f64,
        -7.25_f64,
    ] {
        let (high, low) = split(value);
        assert_eq!(
            (high + low).to_bits(),
            f64::to_bits(value),
            "{value}: exact"
        );
        // At most 26 significant bits: the 53-bit significand ends with 27 zeros.
        assert!(high.to_bits().trailing_zeros() >= 27, "{value}: {high}");
        assert!(
            low.abs() <= high.abs() * libm::exp2(-26.0_f64),
            "{value}: {low}"
        );
    }
}

/// `frequency · frame / rate` modulo 1, exactly, with `frequency = m · 2^-k` reduced in
/// integers: `(m · frame) mod (rate · 2^k)`, over `rate · 2^k`, as `⌊2^64 · remainder /
/// modulus⌋ · 2^-64` (within 2^-64 below the exact value) rounded once to `f64`.
fn exact_cycles(frequency: f64, frame: u32, rate: u32) -> f64 {
    let bits = frequency.to_bits();
    let biased = i32::try_from(bits >> 52_u32).expect("11 bits");
    let down = u32::try_from(1_075_i32.checked_sub(biased).expect("small")).expect("below 2^52 Hz");
    let mantissa = u128::from((bits & 0x000f_ffff_ffff_ffff) | 0x0010_0000_0000_0000);
    let modulus = u128::from(rate).checked_shl(down).expect("below 2^128");
    let remainder = mantissa
        .checked_mul(u128::from(frame))
        .and_then(|product| product.checked_rem(modulus))
        .expect("below 2^128");
    // 64 bits of the quotient by long division: the running remainder stays below the
    // modulus, below 2^127, so doubling it fits.
    let (fraction, _) = (0_u32..64).fold((0_u64, remainder), |(quotient, left), _| {
        let doubled = left.checked_mul(2).expect("below 2^128");
        let bit = doubled >= modulus;
        let next = if bit {
            doubled.checked_sub(modulus).expect("above")
        } else {
            doubled
        };
        (quotient.wrapping_shl(1) | u64::from(bit), next)
    });
    let high = u32::try_from(fraction >> 32_u32).expect("32 bits");
    let low = u32::try_from(fraction & 0xffff_ffff).expect("32 bits");
    (f64::from(high) * 4_294_967_296.0_f64 + f64::from(low)) * libm::exp2(-64.0_f64)
}

#[test]
fn reduces_the_phase_exactly_however_far() {
    // 1000.1 Hz uses every bit of its significand: the plain quotient `f·n / rate` loses
    // more than 2^-45 cycle at the last frame (the control below), the reduced phase 2^-51 at most.
    let last = MAX_FRAMES - 1;
    for (frequency, frame, hz) in [
        (1000.1_f64, last, 44_100),
        (1000.1_f64, 1, 44_100),
        (0.1_f64, last, 8_000),
        (191_999.9_f64, last, 384_000),
        (440.25_f64, 96_000, 96_000),
    ] {
        let got = cycles(frequency, frame, f64::from(hz));
        let wanted = exact_cycles(frequency, frame, hz);
        // Within 2^-51 of the exact phase, or of it plus or minus a whole cycle (a phase
        // just below 0 or 1 cycle); never further.
        let difference = got - wanted;
        let turns = difference.round();
        assert!(
            turns.abs() <= 1.0_f64,
            "{frequency} Hz, frame {frame}: {got}, more than a cycle off"
        );
        assert!(
            (difference - turns).abs() <= libm::exp2(-51.0_f64),
            "{frequency} Hz, frame {frame}: {got} instead of {wanted}"
        );
    }
    // The control: without the reduction, the error is far above 2^-51.
    let plain = 1000.1_f64 * f64::from(last) / 44_100.0_f64;
    let wanted = exact_cycles(1000.1_f64, last, 44_100);
    assert!(
        ((plain - plain.floor()) - wanted).abs() > libm::exp2(-45.0_f64),
        "the control fails"
    );
}

#[test]
fn tells_whole_turns_exactly() {
    for (multiple, fundamental, whole) in [
        (0.0_f64, 1_000.0_f64, true),
        (8.0_f64, 1_000.0_f64, true),
        (-8.0_f64, 1_000.0_f64, true),
        (7.0_f64, 1_000.0_f64, false),
        (2.0_f64, 4_000.0_f64, true),
        (3.0_f64, 2_666.666_748_046_875_f64, false),
        (16.0_f64, 1_000.5_f64, false),
        (16.0_f64, 1_500.0_f64, true),
    ] {
        assert_eq!(
            whole_turns(multiple, fundamental, 8_000.0_f64),
            whole,
            "{multiple} · {fundamental}"
        );
    }
}

#[test]
fn keeps_each_folded_frequency_once_with_its_lowest_order() {
    let found = components(1_000.0_f64, 8_000.0_f64, 0..=HARMONICS);
    let wanted = [(0, true), (1, false), (2, false), (3, false), (4, true)];
    assert_eq!(found.len(), wanted.len(), "{found:?}");
    for (component, (order, edge)) in found.iter().zip(wanted) {
        assert_eq!(*component, Component { order, edge }, "{found:?}");
    }
    // 7 kHz at 48 kHz: every order to 10 lands on its own frequency, none at an edge.
    let distinct = components(7_000.0_f64, 48_000.0_f64, 0..=HARMONICS);
    assert_eq!(distinct.len(), 11, "{distinct:?}");
    assert!(
        distinct.iter().skip(1).all(|component| !component.edge),
        "{distinct:?}"
    );
    // A quarter of the rate: DC, the fundamental, Nyquist.
    let quarter = components(2_000.0_f64, 8_000.0_f64, 0..=HARMONICS);
    let edges: Vec<(u32, bool)> = quarter
        .iter()
        .map(|component| (component.order, component.edge))
        .collect();
    assert_eq!(edges, [(0, true), (1, false), (2, true)], "{quarter:?}");
    // A third of the rate: every harmonic lands on the fundamental or DC.
    let third = components(16_000.0_f64, 48_000.0_f64, 0..=HARMONICS);
    assert_eq!(
        third
            .iter()
            .map(|component| component.order)
            .collect::<Vec<_>>(),
        [0, 1],
        "{third:?}"
    );
    // 440 Hz at 48 kHz: eleven distinct frequencies, all but DC inside the band.
    let all = components(440.0_f64, 48_000.0_f64, 0..=HARMONICS);
    assert_eq!(all.len(), 11, "{all:?}");
    assert!(
        all.iter().skip(1).all(|component| !component.edge),
        "{all:?}"
    );
    assert_eq!(
        components(440.0_f64, 48_000.0_f64, 0..=1).len(),
        2,
        "the sine fit: DC and the fundamental"
    );
    assert_eq!(HARMONICS, 10, "the documented order");
}

#[test]
fn gives_each_unknown_the_mean_square_of_its_function() {
    let found = unknowns(&components(1_000.0_f64, 8_000.0_f64, 0..=HARMONICS));
    let weights: Vec<(u32, f64)> = found
        .iter()
        .map(|unknown: &Unknown| (unknown.component.order, unknown.weight))
        .collect();
    assert_eq!(
        weights,
        [
            (0, 1.0_f64),
            (1, 0.5_f64),
            (1, 0.5_f64),
            (2, 0.5_f64),
            (2, 0.5_f64),
            (3, 0.5_f64),
            (3, 0.5_f64),
            (4, 1.0_f64)
        ],
        "one unknown at an edge, two elsewhere"
    );
}

#[test]
fn evaluates_the_basis_at_a_phase() {
    let found = components(1_000.0_f64, 8_000.0_f64, 0..=HARMONICS);
    let mut row = Vec::new();
    basis(&found, 0.125_f64, &mut row);
    // Orders 0 to 4 at an eighth of a cycle: 1; cos and sin of π/4, π/2, 3π/4; cos π.
    let half = core::f64::consts::FRAC_1_SQRT_2;
    let wanted = [1.0_f64, half, half, 0.0_f64, 1.0_f64, -half, half, -1.0_f64];
    // Tolerance: each value is a sine or cosine of `2π·k·φ` (a product and the library's
    // function, within an ulp each, of values below 1): a few ulps, 2^-53 each, below 1e-15;
    // from 7.125, the fraction of `k·φ` (up to 4 · 7.125) is exact, its product with 2π
    // rounds once more on a value below 2π: below 1e-14.
    assert_eq!(row.len(), wanted.len(), "{row:?}");
    for (value, expected) in row.iter().zip(wanted) {
        assert!((value - expected).abs() < 1e-15_f64, "{row:?}");
    }
    // Far from [0, 1): the fraction of `k·φ` is taken first.
    let mut far = Vec::new();
    basis(&found, 7.125_f64, &mut far);
    for (value, expected) in far.iter().zip(&row) {
        assert!((value - expected).abs() < 1e-14_f64, "{far:?}");
    }
}

#[test]
fn bounds_its_own_error_by_the_floor() {
    // The derivation of `spec/measurements.md`, "Error of the measurements", term by term:
    // the phase of order k errs by at most k·2^-51 + 2^-53 cycle, so a basis value by
    // 2π·(HARMONICS·2^-51 + 2^-53) + 2^-52 (the sine or cosine itself);
    let basis_error = TAU * (f64::from(HARMONICS) * libm::exp2(-51.0_f64) + libm::exp2(-53.0_f64))
        + libm::exp2(-52.0_f64);
    // a model of K ≤ 2·(HARMONICS + 1) values and the sample errs by (K + 1)·2^-53 of the
    // sum of their magnitudes;
    let unknowns = 2.0_f64 * (f64::from(HARMONICS) + 1.0_f64);
    let evaluation = (unknowns + 1.0_f64) * libm::exp2(-53.0_f64);
    // the scaled normal equations `Ĝ y = b` err, entry by entry, by at most 2·(2β + 2^-52)
    // (products of two basis values, each within β, then compensated sums within 2^-52;
    // the scaled entries are at most 2 in magnitude), so a row of `Ĝ` by K times that, and
    // `b` by β·√(2N)·peak (each product has one basis value) plus the rounding of those
    // products and their compensated sum, 2^-53 and 2^-52 of it; as `‖Ĝ⁻¹‖∞ ≤ 1/(1 - ρ)`,
    // the coefficients `c = s·y` err by at most
    // `(K·2·(2β + 2^-52)·Σ|c| + (2β + 2^-53 + 2^-52)·peak) / (1 - ρ)`, plus the Jacobi
    // iterations left over (`ρ^T` of three times the right-hand side);
    let rows = unknowns * 2.0_f64 * (2.0_f64 * basis_error + libm::exp2(-52.0_f64));
    let right = 2.0_f64 * basis_error + libm::exp2(-53.0_f64) + libm::exp2(-52.0_f64);
    let solve = (rows + right) / (1.0_f64 - CONDITIONING_MAX)
        + libm::pow(CONDITIONING_MAX, f64::from(JACOBI_ITERATIONS)) * 3.0_f64;
    // and the spectrum errs by log2(M)·5·2^-53 of its norm (Higham, 2002, theorem 24.2),
    // at most 23 stages, times √(M/N)·2/Σw·√N ≤ 2·√8/0.358 at most.
    let transform =
        23.0_f64 * 5.0_f64 * libm::exp2(-53.0_f64) * 2.0_f64 * 8.0_f64.sqrt() / 0.358_f64;
    let total = basis_error + evaluation + solve + transform;
    assert!(total <= FLOOR, "{total} above the floor {FLOOR}");
    assert_eq!(FLOOR.to_bits(), libm::exp2(-36.0_f64).to_bits(), "2^-36");
    assert!(
        libm::pow(CONDITIONING_MAX, f64::from(JACOBI_ITERATIONS)) <= libm::exp2(-60.0_f64),
        "converged"
    );
}

/// `cycles · n` exactly reduced, as a sine of amplitude `amplitude` and phase `phase`.
fn sine(frames: u32, cycles: f64, amplitude: f64, phase: f64) -> Vec<f64> {
    (0..frames)
        .map(|frame| {
            let turns = cycles * f64::from(frame);
            amplitude * libm::sin(TAU * (turns - turns.floor()) + phase)
        })
        .collect()
}

#[test]
fn measures_at_the_longest_length() {
    // A quarter of the rate: DC, the fundamental and Nyquist only, so a long fit is quick.
    let samples = sine(MAX_FRAMES, 0.25_f64, 0.5_f64, 0.3_f64);
    let measured = measure(&samples, rate(8_000), 2_000.0_f64).expect("measurable");
    assert!(
        (measured.fundamental - 0.5_f64).abs() <= measured.sine.floor,
        "{}",
        measured.fundamental
    );
    assert!(measured.thd_plus_n < -200.0_f64, "{}", measured.thd_plus_n);
    let mut longer = samples;
    longer.push(0.0_f64);
    let frames = usize::try_from(MAX_FRAMES).expect("small") + 1;
    assert_eq!(
        measure(&longer, rate(8_000), 2_000.0_f64),
        Err(MeasureError::TooLong { frames }),
        "one frame more"
    );
}

#[test]
fn refuses_what_it_cannot_measure() {
    let tone = sine(64, 0.1_f64, 1.0_f64, 0.0_f64);
    let at = |fundamental: f64| measure(&tone, rate(8_000), fundamental);
    for fundamental in [
        0.0_f64,
        -0.0_f64,
        -1.0_f64,
        4_000.0_f64,
        4_000.5_f64,
        f64::NAN,
        f64::INFINITY,
    ] {
        assert_eq!(
            format!("{:?}", at(fundamental)),
            format!(
                "{:?}",
                Err::<(), _>(MeasureError::Fundamental { hz: fundamental })
            ),
            "{fundamental}"
        );
    }
    assert!(
        at(3_999.0_f64).is_ok()
            || matches!(at(3_999.0_f64), Err(MeasureError::Unresolvable { .. })),
        "just below Nyquist"
    );
    assert!(at(800.0_f64).is_ok(), "inside");
    assert_eq!(
        measure(&[], rate(8_000), 800.0_f64),
        Err(MeasureError::NoSamples),
        "empty"
    );
    let mut broken = tone.clone();
    broken[5] = f64::INFINITY;
    assert_eq!(
        measure(&broken, rate(8_000), 800.0_f64),
        Err(MeasureError::NotFinite { index: 5 }),
        "infinite"
    );
    assert_eq!(
        measure(&tone[..2], rate(8_000), 800.0_f64),
        Err(MeasureError::TooShort {
            frames: 2,
            needed: 3
        }),
        "fewer frames than DC, a cosine and a sine"
    );
    assert!(
        matches!(
            measure(&tone[..3], rate(8_000), 800.0_f64),
            Err(MeasureError::Unresolvable { .. })
        ),
        "as many frames as unknowns, too few to tell them apart"
    );
    assert_eq!(
        measure(&[0.0_f64; 64], rate(8_000), 800.0_f64),
        Err(MeasureError::NoFundamental { amplitude: 0.0_f64 }),
        "silence"
    );
    // DC only: the fitted fundamental is rounding noise, below the floor.
    assert!(
        matches!(
            measure(&[0.5_f64; 64], rate(8_000), 800.0_f64),
            Err(MeasureError::NoFundamental { .. })
        ),
        "a constant"
    );
}

#[test]
fn refuses_components_it_cannot_tell_apart() {
    // 1024 frames at 48 kHz: a bin is 46.9 Hz. At 0.06 bin from DC, the sine's cosine and
    // DC are nearly the same function.
    let near = sine(1_024, 3.0_f64 / 48_000.0_f64, 1.0_f64, 0.0_f64);
    match measure(&near, rate(48_000), 3.0_f64) {
        Err(MeasureError::Unresolvable { conditioning }) => {
            assert!(conditioning > CONDITIONING_MAX, "{conditioning}");
        }
        other => panic!("{other:?}"),
    }
    // Ten bins from DC, they are told apart, with a conditioning far below the limit.
    let apart = sine(1_024, 10.0_f64 / 1_024.0_f64, 1.0_f64, 0.0_f64);
    let measured = measure(&apart, rate(48_000), 468.75_f64).expect("resolvable");
    assert!(
        measured.sine.conditioning < 0.1_f64,
        "{}",
        measured.sine.conditioning
    );
    // Orthogonal components: a whole number of periods of every component, even 2 Hz from
    // DC and Nyquist (3999 Hz over one second at 8 kHz).
    let coherent = sine(8_000, 3_999.0_f64 / 8_000.0_f64, 1.0_f64, 0.0_f64);
    let coherent_harmonics = measure(&coherent, rate(8_000), 3_999.0_f64)
        .expect("resolvable")
        .harmonics
        .expect("resolvable");
    // Tolerance: orthogonal components give a Gram matrix diagonal in exact arithmetic; its
    // off-diagonal entries are sums of N products below 1, each within N · 2^-53 of zero
    // (8000 · 2^-53 < 1e-12), the conditioning the largest of them relative to the diagonal.
    assert!(
        coherent_harmonics.quality.conditioning < 1e-12_f64,
        "{}",
        coherent_harmonics.quality.conditioning
    );
    let whole = sine(1_000, 0.125_f64, 1.0_f64, 0.0_f64);
    let orthogonal = measure(&whole, rate(8_000), 1_000.0_f64).expect("resolvable");
    assert!(
        orthogonal.sine.conditioning < 1e-12_f64,
        "{}",
        orthogonal.sine.conditioning
    );
    let quality = orthogonal.harmonics.expect("resolvable").quality;
    assert!(quality.conditioning < 1e-12_f64, "{}", quality.conditioning);
    // The harmonic fit alone refused: 16001 Hz at 48 kHz over 4096 frames, whose second
    // harmonic folds a quarter of a bin from it.
    let close = sine(4_096, 16_001.0_f64 / 48_000.0_f64, 1.0_f64, 0.0_f64);
    let refused = measure(&close, rate(48_000), 16_001.0_f64).expect("the sine fit resolves");
    assert!(
        matches!(refused.harmonics, Err(MeasureError::Unresolvable { conditioning }) if conditioning > CONDITIONING_MAX),
        "{:?}",
        refused.harmonics
    );
    assert!(
        refused.sine.conditioning <= CONDITIONING_MAX,
        "{}",
        refused.sine.conditioning
    );
}

#[test]
fn says_what_it_refuses() {
    for (error, text) in [
        (
            MeasureError::NoSamples,
            "the signal has no sample".to_owned(),
        ),
        (
            MeasureError::TooLong { frames: 7 },
            format!("the signal has 7 frames, more than the {MAX_FRAMES} measured"),
        ),
        (
            MeasureError::NotFinite { index: 3 },
            "sample 3 is not finite".to_owned(),
        ),
        (
            MeasureError::Fundamental { hz: 9.5_f64 },
            "the fundamental, 9.5 Hz, is not strictly between 0 and Nyquist".to_owned(),
        ),
        (
            MeasureError::TooShort {
                frames: 2,
                needed: 3,
            },
            "the signal has 2 frames, fewer than the 3 unknowns of the fit".to_owned(),
        ),
        (
            MeasureError::Unresolvable {
                conditioning: 0.75_f64,
            },
            "the components are too close to be told apart (conditioning 0.75, at most 0.5)"
                .to_owned(),
        ),
        (
            MeasureError::NoFundamental {
                amplitude: 0.25_f64,
            },
            "the fundamental, of amplitude 0.25, is not above the floor of the measurement"
                .to_owned(),
        ),
    ] {
        assert_eq!(error.to_string(), text, "{error:?}");
    }
}

/// A measurement with the given qualities, for the bounds alone.
fn measured_with(sine: Quality, harmonic: Option<Quality>, enbw: f64) -> Measurements {
    Measurements {
        fundamental: 1.0_f64,
        thd_plus_n: 0.0_f64,
        sfdr: 0.0_f64,
        sine,
        enbw,
        harmonics: harmonic
            .map(|quality| Harmonics {
                thd: None,
                aliasing: None,
                snr: 0.0_f64,
                quality,
            })
            .ok_or(MeasureError::Unresolvable {
                conditioning: 1.0_f64,
            }),
    }
}

#[test]
fn derives_the_bounds_term_by_term() {
    // b = 2^-25 + 2^-40 = 2.9803141...e-8; ρ = 0.1, then 0.2; ENBW = 2.
    let sine = Quality {
        conditioning: 0.1_f64,
        floor: libm::exp2(-40.0_f64),
    };
    let harmonic = Quality {
        conditioning: 0.2_f64,
        floor: libm::exp2(-40.0_f64),
    };
    let bounds = sine_bounds(
        1.0_f64,
        libm::exp2(-25.0_f64),
        &measured_with(sine, Some(harmonic), 2.0_f64),
    )
    .expect("bounded");
    // Written again from the formulas of the specification, with b, a computed by hand:
    // a = 1 - b·√2.2/0.9 for the sine fit, 1 - b·√2.4/0.8 for the harmonic fit.
    let error_bound = libm::exp2(-25.0_f64) + libm::exp2(-40.0_f64);
    let a_sine = 1.0_f64 - error_bound * 2.2_f64.sqrt() / 0.9_f64;
    let a_harmonic = 1.0_f64 - error_bound * 2.4_f64.sqrt() / 0.8_f64;
    // Tolerance: levels near -150 dB, a few roundings deep on both sides (a square root, a
    // quotient, a logarithm): within a few ulps of 150, about 1e-13: 1e-12 dB.
    let close = |got: f64, wanted: f64, what: &str| {
        assert!(
            (got - wanted).abs() < 1e-12_f64,
            "{what}: {got} instead of {wanted}"
        );
    };
    close(
        bounds.thd_plus_n,
        amplitude_db(2.0_f64.sqrt() * error_bound / a_sine),
        "THD+N",
    );
    close(
        bounds.sfdr,
        amplitude_db(a_sine / (2.0_f64 * error_bound * 2.0_f64.sqrt())),
        "SFDR",
    );
    let harmonics = bounds.harmonics.expect("the harmonic fit");
    close(
        harmonics.thd,
        amplitude_db(2.0_f64.sqrt() * error_bound * 1.2_f64.sqrt() / (0.8_f64 * a_harmonic)),
        "THD",
    );
    close(
        harmonics.snr,
        -amplitude_db(2.0_f64.sqrt() * error_bound / a_harmonic),
        "SNR",
    );
    // About -147.5 dB, 147.5 dB and -144.5 dB at this scale: the levels of a full-scale
    // binary32 sine. Tolerance: the value rounded to one decimal for this comment, within
    // 0.05 dB of it; 0.2 dB only checks the comment's figure, the exact levels are checked
    // above.
    assert!(
        (bounds.thd_plus_n + 147.5_f64).abs() < 0.2_f64,
        "{}",
        bounds.thd_plus_n
    );
    // No harmonic bound without a harmonic fit, nor any bound when the error reaches the
    // amplitude.
    let alone = sine_bounds(
        1.0_f64,
        libm::exp2(-25.0_f64),
        &measured_with(sine, None, 2.0_f64),
    )
    .expect("bounded");
    assert_eq!(alone.harmonics, None, "refused harmonic fit");
    assert_eq!(
        sine_bounds(
            1.0_f64,
            0.7_f64,
            &measured_with(sine, Some(harmonic), 2.0_f64)
        ),
        None,
        "too large an error"
    );
    let partial = sine_bounds(
        1.0_f64,
        0.55_f64,
        &measured_with(
            Quality {
                conditioning: 0.0_f64,
                floor: 0.0_f64,
            },
            Some(Quality {
                conditioning: 0.5_f64,
                floor: 0.0_f64,
            }),
            2.0_f64,
        ),
    )
    .expect("the sine fit bounds");
    assert_eq!(
        partial.harmonics, None,
        "the harmonic fit's margin is gone first"
    );
    // A negative amplitude bounds like its magnitude.
    assert_eq!(
        sine_bounds(
            -1.0_f64,
            libm::exp2(-25.0_f64),
            &measured_with(sine, Some(harmonic), 2.0_f64)
        ),
        Some(bounds),
        "the sign of the amplitude"
    );
}

#[test]
fn every_signal_within_the_error_meets_the_bounds() {
    // The bounds attacked: an ideal sine plus errors of magnitude b, square waves at a
    // harmonic, at an aliased harmonic and at an unrelated frequency, alternating signs and
    // random values, at several rates and lengths. Every measurement must stay within the
    // bounds.
    let error = 1e-6_f64;
    let mut generator = SplitMix(7);
    for (hz, frames, cycles) in [
        (8_000_u32, 1_000_u32, 0.123_f64),
        (48_000, 4_096, 0.021_f64),
        (44_100, 3_000, 0.41_f64),
    ] {
        let fundamental = cycles * f64::from(hz);
        let ideal = sine(frames, cycles, 1.0_f64, 0.4_f64);
        let shapes: [Box<dyn Fn(u32) -> f64>; 5] = [
            Box::new(|frame| libm::cos(TAU * 2.0_f64 * cycles * f64::from(frame)).signum()),
            Box::new(|frame| libm::cos(TAU * 7.0_f64 * cycles * f64::from(frame)).signum()),
            Box::new(|frame| libm::cos(TAU * 0.3171_f64 * f64::from(frame)).signum()),
            Box::new(|frame| if frame % 2 == 0 { 1.0_f64 } else { -1.0_f64 }),
            Box::new(|_| 0.0_f64),
        ];
        for (index, shape) in shapes.iter().enumerate() {
            let samples: Vec<f64> = (0..frames)
                .zip(&ideal)
                .map(|(frame, value)| {
                    let sign = if index == 4 {
                        2.0_f64 * generator.next() - 1.0_f64
                    } else {
                        shape(frame)
                    };
                    value + error * sign
                })
                .collect();
            let measured = measure(&samples, rate(hz), fundamental).expect("measurable");
            let bounds = sine_bounds(1.0_f64, error, &measured).expect("bounded");
            assert!(
                measured.thd_plus_n <= bounds.thd_plus_n,
                "{hz} {index}: THD+N {} above {}",
                measured.thd_plus_n,
                bounds.thd_plus_n
            );
            assert!(
                measured.sfdr >= bounds.sfdr,
                "{hz} {index}: SFDR {} below {}",
                measured.sfdr,
                bounds.sfdr
            );
            let harmonics = measured.harmonics.as_ref().expect("resolvable");
            let limits = bounds.harmonics.expect("bounded");
            assert!(
                harmonics.snr >= limits.snr,
                "{hz} {index}: SNR {} below {}",
                harmonics.snr,
                limits.snr
            );
            for level in [harmonics.thd, harmonics.aliasing].into_iter().flatten() {
                assert!(
                    level <= limits.thd,
                    "{hz} {index}: {level} above {}",
                    limits.thd
                );
            }
            // The bounds are not vacuous: a full-scale square error of the harmonic shape
            // reaches within 4 dB of the THD+N bound.
            if index == 0 {
                assert!(
                    measured.thd_plus_n > bounds.thd_plus_n - 4.0_f64,
                    "{hz}: {} far below {}",
                    measured.thd_plus_n,
                    bounds.thd_plus_n
                );
            }
        }
    }
}

/// A fit's quality as a vector gives it.
fn quality(value: &Value) -> Quality {
    Quality {
        conditioning: value["conditioning"].as_f64().expect("a conditioning"),
        floor: value["floor"].as_f64().expect("a floor"),
    }
}

#[test]
fn bounds_like_the_shared_vectors() {
    let vectors: Value = serde_json::from_str(VECTORS).expect("valid JSON");
    let list = vectors["sine_bounds"].as_array().expect("bounds");
    assert_eq!(list.len(), 7, "every vector");
    for vector in list {
        let name = vector["name"].as_str().expect("a name");
        let harmonic = (!vector["harmonic"].is_null()).then(|| quality(&vector["harmonic"]));
        let measured = measured_with(
            quality(&vector["sine"]),
            harmonic,
            vector["enbw"].as_f64().expect("an ENBW"),
        );
        let got = sine_bounds(
            vector["amplitude"].as_f64().expect("an amplitude"),
            vector["error"].as_f64().expect("an error"),
            &measured,
        );
        let expected = &vector["expected"];
        // The formulas are a few roundings deep: within 1e-9 dB of the independent values.
        let close = |level: f64, wanted: &Value, what: &str| {
            let wanted = wanted.as_f64().expect("a level");
            assert!(
                (level - wanted).abs() < 1e-9_f64,
                "{name}: {what} {level} instead of {wanted}"
            );
        };
        if expected.is_null() {
            assert_eq!(got, None, "{name}");
            continue;
        }
        let bounds = got.expect(name);
        close(bounds.thd_plus_n, &expected["thd_plus_n"], "THD+N");
        close(bounds.sfdr, &expected["sfdr"], "SFDR");
        if expected["harmonics"].is_null() {
            assert_eq!(bounds.harmonics, None, "{name}");
        } else {
            let harmonics = bounds.harmonics.expect(name);
            close(harmonics.thd, &expected["harmonics"]["thd"], "THD");
            close(harmonics.snr, &expected["harmonics"]["snr"], "SNR");
        }
    }
}

/// A compensated sum holding `value`.
fn sum_of(value: f64) -> Sum {
    let mut sum = Sum::default();
    sum.add(value);
    sum
}

#[test]
fn accepts_a_conditioning_of_one_half_and_no_more() {
    // One unknown of scale 1: `Ĝ = 1.5`, a conditioning of exactly 1/2, accepted and solved
    // (`1.5·y = 3`); just above it, refused.
    let (coefficients, conditioning) =
        solve(&[sum_of(1.5_f64)], &[sum_of(3.0_f64)], &[1.0_f64]).expect("accepted");
    assert_eq!(conditioning.to_bits(), 0.5_f64.to_bits(), "exactly 1/2");
    // Tolerance: `3 / 1.5` through the solver's few roundings of values near 2: within a few
    // ulps of 2 (2^-51 each), below 1e-15.
    assert!(
        (coefficients[0] - 2.0_f64).abs() < 1e-15_f64,
        "{coefficients:?}"
    );
    let above = 1.5_f64 + libm::exp2(-52.0_f64);
    assert!(
        matches!(
            solve(&[sum_of(above)], &[sum_of(3.0_f64)], &[1.0_f64]),
            Err(MeasureError::Unresolvable { .. })
        ),
        "just above"
    );
    // Two coupled unknowns, scaled: `G = [[4, 1], [1, 1]]`, scales 1/2 and 1: `Ĝ = [[1,
    // 1/2], [1/2, 1]]`, conditioning 1/2; the solution of `Ĝ y = (1, 2)` is `(0, 2)`, so
    // `c = (0, 2)`.
    let gram = [
        sum_of(4.0_f64),
        sum_of(1.0_f64),
        sum_of(1.0_f64),
        sum_of(1.0_f64),
    ];
    let (coupled, rows) = solve(
        &gram,
        &[sum_of(2.0_f64), sum_of(2.0_f64)],
        &[0.5_f64, 1.0_f64],
    )
    .expect("accepted");
    assert_eq!(rows.to_bits(), 0.5_f64.to_bits(), "the larger row");
    // Rows of different deviations: the largest decides, 0.6 here, refused.
    let uneven = [
        1.0_f64, 0.3_f64, 0.3_f64, 0.3_f64, 1.0_f64, 0.0_f64, 0.3_f64, 0.0_f64, 1.0_f64,
    ]
    .map(sum_of);
    let ones = [1.0_f64; 3].map(sum_of);
    assert!(
        matches!(solve(&uneven, &ones, &[1.0_f64; 3]), Err(MeasureError::Unresolvable { conditioning: largest }) if largest.to_bits() == 0.6_f64.to_bits()),
        "the largest row"
    );
    // Tolerance: a 2 × 2 system with entries below 2 solved in a few roundings: within a few
    // ulps of its exact solution (0, 2), below 1e-15.
    assert!(
        coupled[0].abs() < 1e-15_f64 && (coupled[1] - 2.0_f64).abs() < 1e-15_f64,
        "{coupled:?}"
    );
}

#[test]
fn scales_its_floor_by_the_signal_and_the_fit() {
    // 0.5·sin(2π·n/8) + 0.25: peak 0.75 (at n = 2), coefficients 0.25 (DC) and 0.5 (the
    // sine), so a floor of FLOOR · 1.5, in both fits (the harmonics fit to rounding).
    let samples: Vec<f64> = sine(1_000, 0.125_f64, 0.5_f64, 0.0_f64)
        .into_iter()
        .map(|value| value + 0.25_f64)
        .collect();
    let measured = measure(&samples, rate(8_000), 1_000.0_f64).expect("measurable");
    let wanted = FLOOR * 1.5_f64;
    // Tolerance, relative: the floor is FLOOR times the peak plus the coefficients' sizes,
    // each fitted to within the fit's own error, about 1e-13 of them: 1e-12 of the floor.
    assert!(
        (measured.sine.floor - wanted).abs() <= wanted * 1e-12_f64,
        "{}",
        measured.sine.floor
    );
    let harmonic = measured.harmonics.expect("resolvable").quality.floor;
    assert!(
        (harmonic - wanted).abs() <= wanted * 1e-12_f64,
        "{harmonic}"
    );
}

#[test]
fn bounds_nothing_without_a_margin_above_zero() {
    // `a = A - b·√2` exactly zero: no bound, as below zero.
    let amplitude = 0.5_f64 * 2.0_f64.sqrt();
    let orthogonal = Quality {
        conditioning: 0.0_f64,
        floor: 0.0_f64,
    };
    let measured = measured_with(orthogonal, Some(orthogonal), 2.0_f64);
    assert_eq!(
        sine_bounds(amplitude, 0.5_f64, &measured),
        None,
        "a margin of zero"
    );
    assert!(
        sine_bounds(amplitude, 0.499_f64, &measured).is_some(),
        "a margin above zero"
    );
}
