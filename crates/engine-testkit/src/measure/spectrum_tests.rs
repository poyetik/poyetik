use core::f64::consts::{PI, TAU};

use engine_core::SampleRate;

use super::{Complex, PADDING, WINDOW, fft, spectrum, window};
use crate::measure::{MAX_FRAMES, MeasureError};

/// 48 kHz.
fn rate() -> SampleRate {
    SampleRate::new(48_000).expect("a supported rate")
}

/// `a · cos(2π·cycles·n + phase)` for `n` below `frames`, `cycles` a fraction of a cycle
/// per sample whose product with every `n` is exact (few significant bits).
fn tone(frames: u32, cycles: f64, amplitude: f64, phase: f64) -> Vec<f64> {
    (0..frames)
        .map(|frame| {
            let turns = cycles * f64::from(frame);
            amplitude * libm::cos(TAU * (turns - turns.floor()) + phase)
        })
        .collect()
}

/// The window written again from its definition, term by term.
fn reference_window(frames: u32) -> Vec<f64> {
    (0..frames)
        .map(|frame| {
            let angle = TAU * f64::from(frame) / f64::from(frames);
            0.358_75_f64 - 0.488_29_f64 * libm::cos(angle)
                + 0.141_28_f64 * libm::cos(2.0_f64 * angle)
                - 0.011_68_f64 * libm::cos(3.0_f64 * angle)
        })
        .collect()
}

/// `|Σ w[n] e^(-2πiνn)| / Σ w[n]`: the window's transform at `nu` cycles per sample,
/// relative to its peak, by direct summation.
fn transform(weights: &[f64], nu: f64) -> f64 {
    let (re, im) = (0_u32..)
        .zip(weights)
        .fold((0.0_f64, 0.0_f64), |(re, im), (frame, &weight)| {
            let angle = TAU * nu * f64::from(frame);
            (
                re + weight * libm::cos(angle),
                im - weight * libm::sin(angle),
            )
        });
    (re * re + im * im).sqrt() / weights.iter().sum::<f64>()
}

#[test]
fn the_window_has_its_closed_form_values() {
    // N = 4: w[0] = a0 - a1 + a2 - a3, w[1] = w[3] = a0 - a2, w[2] = a0 + a1 + a2 + a3.
    // Tolerance: each weight is four products of a coefficient (below 1) by a cosine (within
    // an ulp) and three sums, values below 1: at most 8 roundings of 2^-53, below 1e-15.
    let expected = [6.0e-5_f64, 0.217_47_f64, 1.0_f64, 0.217_47_f64];
    for (value, wanted) in window(4).iter().zip(expected) {
        assert!(
            (value - wanted).abs() < 1e-15_f64,
            "{value} instead of {wanted}"
        );
    }
    assert_eq!(
        WINDOW.map(f64::to_bits),
        [0.358_75_f64, 0.488_29_f64, 0.141_28_f64, 0.011_68_f64].map(f64::to_bits),
        "Harris's coefficients"
    );
    for frames in [1, 2, 3, 7, 64, 1_000] {
        let weights = window(frames);
        assert_eq!(
            weights.len(),
            usize::try_from(frames).expect("small"),
            "one per sample"
        );
        // Tolerance: both forms are four products and three sums of values below 1, each
        // within 8 roundings of 2^-53 of the exact weight: 16 · 2^-53 below 1e-15 apart.
        for (value, wanted) in weights.iter().zip(reference_window(frames)) {
            assert!(
                (value - wanted).abs() < 1e-15_f64,
                "{value} instead of {wanted}"
            );
        }
        assert!(
            weights.iter().all(|&weight| weight > 0.0_f64),
            "positive at {frames}"
        );
    }
}

#[test]
fn the_window_keeps_its_sidelobes_92_db_down() {
    // Sampled every 1/500 of a bin from the edge of the main lobe (4 bins) to Nyquist, at
    // N = 256: the highest sidelobe is -92.01 dB.
    let frames = 256_u32;
    let weights = window(frames);
    let steps = 62_000_u32;
    let highest = (0..=steps)
        .map(|step| {
            let bins = 4.0_f64 + 124.0_f64 * f64::from(step) / f64::from(steps);
            transform(&weights, bins / f64::from(frames))
        })
        .fold(
            0.0_f64,
            |high, value| if value > high { value } else { high },
        );
    let decibels = 20.0_f64 * libm::log10(highest);
    assert!(decibels < -92.0_f64, "highest sidelobe {decibels} dB");
    assert!(
        decibels > -92.1_f64,
        "{decibels} dB: the window is not Harris's"
    );
}

/// The discrete Fourier transform by its definition, each angle reduced exactly first.
fn direct(signal: &[Complex]) -> Vec<Complex> {
    let points = u32::try_from(signal.len()).expect("small");
    (0..points)
        .map(|point| {
            (0_u32..).zip(signal).fold(
                Complex {
                    re: 0.0_f64,
                    im: 0.0_f64,
                },
                |sum, (frame, value)| {
                    let turns = u64::from(point)
                        .checked_mul(u64::from(frame))
                        .and_then(|product| product.checked_rem(u64::from(points)))
                        .expect("small");
                    let angle = -TAU * f64::from(u32::try_from(turns).expect("below the size"))
                        / f64::from(points);
                    sum.plus(value.times(Complex {
                        re: libm::cos(angle),
                        im: libm::sin(angle),
                    }))
                },
            )
        })
        .collect()
}

#[test]
fn transforms_like_the_definition() {
    for points in [1_u32, 2, 4, 8, 64, 512] {
        let signal: Vec<Complex> = (0..points)
            .map(|frame| Complex {
                re: libm::sin(f64::from(frame) * 1.3_f64) + 0.25_f64,
                im: libm::cos(f64::from(frame) * 0.7_f64) - 0.5_f64,
            })
            .collect();
        let fast = fft(signal.clone(), f64::from(points));
        let slow = direct(&signal);
        assert_eq!(fast.len(), slow.len(), "{points} points");
        // Both err by a few ulps of the largest value (at most `points`) per term: well
        // within 1e-12 for these sizes.
        for (got, wanted) in fast.iter().zip(&slow) {
            assert!(
                (got.re - wanted.re).abs() < 1e-12_f64 && (got.im - wanted.im).abs() < 1e-12_f64,
                "{points} points: {got:?} instead of {wanted:?}"
            );
        }
    }
}

#[test]
fn computes_complex_arithmetic() {
    let left = Complex {
        re: 1.5_f64,
        im: -2.0_f64,
    };
    let right = Complex {
        re: 0.25_f64,
        im: 3.0_f64,
    };
    assert_eq!(
        left.plus(right),
        Complex {
            re: 1.75_f64,
            im: 1.0_f64
        },
        "sum"
    );
    assert_eq!(
        left.minus(right),
        Complex {
            re: 1.25_f64,
            im: -5.0_f64
        },
        "difference"
    );
    // (1.5 - 2i)(0.25 + 3i) = 0.375 + 4.5i - 0.5i + 6 = 6.375 + 4i.
    assert_eq!(
        left.times(right),
        Complex {
            re: 6.375_f64,
            im: 4.0_f64
        },
        "product"
    );
}

#[test]
fn pads_to_a_power_of_two_at_least_four_times_longer() {
    assert_eq!(PADDING, 4, "the documented padding");
    for (frames, points) in [(1, 4), (2, 8), (3, 16), (4, 16), (5, 32), (1_000, 4_096)] {
        let measured = spectrum(&vec![0.5_f64; frames], rate()).expect("measurable");
        assert_eq!(measured.points(), points, "{frames} frames");
        assert_eq!(
            measured.amplitudes().len(),
            usize::try_from(points / 2 + 1).expect("small"),
            "0 to M / 2"
        );
    }
    let measured = spectrum(&[1.0_f64; 3], rate()).expect("measurable");
    assert_eq!(measured.frequency(0).to_bits(), 0.0_f64.to_bits(), "DC");
    assert_eq!(
        measured.frequency(8).to_bits(),
        24_000.0_f64.to_bits(),
        "Nyquist"
    );
    assert_eq!(
        measured.frequency(3).to_bits(),
        9_000.0_f64.to_bits(),
        "3 · 48000 / 16"
    );
}

#[test]
fn measures_dc_and_nyquist_exactly() {
    // Exact in exact arithmetic (the window's weights cancel); in f64, a few ulps per stage
    // of the transform (at most 13 here) and of the window's sum: below 1e-13.
    for frames in [1_u32, 2, 5, 100, 1_001] {
        let constant = vec![0.75_f64; usize::try_from(frames).expect("small")];
        let measured = spectrum(&constant, rate()).expect("measurable");
        assert!(
            (measured.amplitudes()[0] - 0.75_f64).abs() < 1e-13_f64,
            "DC at {frames}"
        );
        let alternating = tone(frames, 0.5_f64, -0.5_f64, 0.0_f64);
        let nyquist = spectrum(&alternating, rate()).expect("measurable");
        let last = nyquist.amplitudes().last().copied().expect("a point");
        assert!(
            (last - 0.5_f64).abs() < 1e-13_f64,
            "Nyquist at {frames}: {last}"
        );
    }
}

#[test]
fn measures_a_sine_on_a_point_exactly() {
    // 1000 frames, 4096 points: 1024/4096 cycle per sample is on point 1024; its image at
    // minus that frequency lies 2048 points away, far below the sidelobes.
    let measured = spectrum(&tone(1_000, 0.25_f64, 0.5_f64, 1.0_f64), rate()).expect("measurable");
    // Tolerance: the window's weights (8 roundings each), its sum over 1000 frames (1000
    // roundings of values below 1000) and the 12 stages of the transform (a few ulps of the
    // largest value, at most 1000, each): about 1e-13 at worst, so 1e-12 is that bound with a
    // factor 10 to spare, far below the image's sidelobes.
    assert!(
        (measured.amplitudes()[1_024] - 0.5_f64).abs() < 1e-12_f64,
        "{}",
        measured.amplitudes()[1_024]
    );
    assert!(
        (measured.peak() - 0.5_f64).abs() < 1e-12_f64,
        "the peak is that point"
    );
}

/// The scalloping bound stays below 0.6 % at every length from 2: the window's second
/// moment is at most about 0.0192 N² and M ≥ 4N, so `L ≤ π² · 0.0192 / 32 ≈ 0.592 %`,
/// checked here at every N from 2 to 4096 and at every power of two to 2^20. One sample is
/// the exception: its moment is `(0 - 1/2)² = 1/4` whatever its
/// weight, and M = 4, so `L = π²/128`, about 7.7 %.
#[test]
fn bounds_the_scalloping_below_six_thousandths_from_two_samples() {
    let single = spectrum(&[0.25_f64], rate()).expect("one sample is measurable");
    assert_eq!(single.points(), 4, "M for one sample");
    // π²/128 from a few roundings of 2^-53 on values below 0.1: within 1e-16.
    assert!(
        (single.scalloping() - PI * PI / 128.0_f64).abs() <= 1e-16_f64,
        "{}",
        single.scalloping()
    );
    assert!(single.scalloping() > 0.077_f64, "one sample: about 7.7 %");
    let mut worst = 0.0_f64;
    for frames in (2_u32..=4_096).chain((13_u32..=20).map(|power| 1_u32 << power)) {
        let weights = reference_window(frames);
        let total: f64 = weights.iter().sum();
        let centre = f64::from(frames) / 2.0_f64;
        let moment: f64 = (0_u32..)
            .zip(&weights)
            .map(|(frame, weight)| {
                weight * (f64::from(frame) - centre) * (f64::from(frame) - centre)
            })
            .sum::<f64>()
            / total;
        // M, the smallest power of two at least 4N (`spec/measurements.md`, "Spectrum").
        let points = f64::from((4 * frames).next_power_of_two());
        let bound = PI * PI * moment / (2.0_f64 * points * points);
        assert!(bound < 0.006_f64, "{frames} samples: {bound}");
        if bound > worst {
            worst = bound;
        }
    }
    // The worst, at the large powers of two, is close to the limit: the bound is not loose.
    assert!(worst > 0.0059_f64, "{worst}");
}

#[test]
fn loses_at_most_the_scalloping_bound_between_points() {
    for frames in [64_u32, 1_000, 4_096] {
        let weights = reference_window(frames);
        let total: f64 = weights.iter().sum();
        let centre = f64::from(frames) / 2.0_f64;
        let moment: f64 = (0_u32..)
            .zip(&weights)
            .map(|(frame, weight)| {
                weight * (f64::from(frame) - centre) * (f64::from(frame) - centre)
            })
            .sum::<f64>()
            / total;
        let measured =
            spectrum(&tone(frames, 0.1_f64, 1.0_f64, 0.0_f64), rate()).expect("measurable");
        let points = f64::from(measured.points());
        let bound = PI * PI * moment / (2.0_f64 * points * points);
        // Tolerance: both values are below 0.01 and a few dozen roundings of 2^-53 deep
        // (the moment's sum included), so within 1e-15 of each other.
        assert!(
            (measured.scalloping() - bound).abs() <= 1e-15_f64,
            "{frames}: {} instead of {bound}",
            measured.scalloping()
        );
        // Below 0.6 % (0.052 dB) at every length from 2 (checked over every length below).
        assert!(bound < 0.006_f64, "{bound}");
        assert!(
            20.0_f64 * libm::log10(1.0_f64 - bound) > -0.052_f64,
            "{bound}: within 0.052 dB"
        );
        // The worst case, a sine halfway between two points, at 1/(2M) cycle from each.
        let cycles = (f64::from(measured.points() / 8) + 0.5_f64) / points;
        let halfway =
            spectrum(&tone(frames, cycles, 1.0_f64, 0.3_f64), rate()).expect("measurable");
        let image = transform(&weights, 2.0_f64 * cycles - 0.5_f64 / points);
        let near = halfway.amplitudes()[usize::try_from(measured.points() / 8).expect("small")];
        assert!(near >= 1.0_f64 - bound - image, "{frames}: {near}");
        assert!(near <= 1.0_f64 + image, "{frames}: {near}");
        // The bound is not loose by more than a factor 1.5 (the cosine's Taylor bound).
        assert!(
            near < 1.0_f64 - bound / 1.5_f64 + image,
            "{frames}: {near} loses less than expected"
        );
    }
}

#[test]
fn has_the_window_s_equivalent_noise_bandwidth() {
    for frames in [1_u32, 4, 1_000] {
        let weights = reference_window(frames);
        let sum: f64 = weights.iter().sum();
        let squares: f64 = weights.iter().map(|weight| weight * weight).sum();
        let wanted = f64::from(frames) * squares / (sum * sum);
        let measured =
            spectrum(&tone(frames, 0.1_f64, 1.0_f64, 0.0_f64), rate()).expect("measurable");
        // Tolerance: two sums of at most 1000 terms below 1 (1000 roundings of 2^-53 each,
        // 2.2e-13 at most) and a quotient near 2: within 1e-12.
        assert!(
            (measured.enbw() - wanted).abs() < 1e-12_f64,
            "{frames}: {}",
            measured.enbw()
        );
    }
    // Harris's value for the four-term window: 2.0044 bins. Tolerance: the published value
    // has five decimals, 2.00435 for N → ∞; at 1000 frames the window's ENBW is within 1e-5
    // of it (the sums of its weights differ from their integrals by O(1/N²)).
    let long = spectrum(&[0.0_f64; 1_000], rate()).expect("measurable");
    assert!(
        (long.enbw() - 2.004_35_f64).abs() < 1e-5_f64,
        "{}",
        long.enbw()
    );
}

#[test]
fn bounds_every_point_by_cauchy_schwarz() {
    // The SFDR bound rests on 2·|X[k]| / Σw ≤ 2·‖x‖·√(Σw²) / Σw = 2·rms·√ENBW; a sine of
    // amplitude 1 has an rms of 1/√2, so no point exceeds √(2·ENBW).
    let measured = spectrum(&tone(4_096, 0.125_f64, 1.0_f64, 0.0_f64), rate()).expect("measurable");
    let limit = (2.0_f64 * measured.enbw()).sqrt();
    assert!(
        measured.peak() <= limit,
        "{} above {limit}",
        measured.peak()
    );
}

#[test]
fn refuses_what_it_cannot_measure() {
    assert_eq!(spectrum(&[], rate()), Err(MeasureError::NoSamples), "empty");
    let longest = usize::try_from(MAX_FRAMES).expect("small");
    assert_eq!(
        spectrum(&vec![0.0_f64; longest + 1], rate()),
        Err(MeasureError::TooLong {
            frames: longest + 1
        }),
        "one frame too long"
    );
    assert_eq!(
        spectrum(&[0.0_f64, 1.0_f64, f64::NAN], rate()),
        Err(MeasureError::NotFinite { index: 2 }),
        "NaN"
    );
    assert_eq!(
        spectrum(&[f64::NEG_INFINITY, f64::NAN], rate()),
        Err(MeasureError::NotFinite { index: 0 }),
        "the first one"
    );
}

#[test]
fn reads_up_to_twice_a_component_beside_an_edge() {
    // A component at Nyquist (or DC) has a single exponential, which the points beside it
    // read with the factor 2 of a sine: up to twice its amplitude, a conservative reading
    // (6.02 dB high at most), never a low one.
    let frames = 1_000_u32;
    let alternating = tone(frames, 0.5_f64, 0.5_f64, 0.0_f64);
    let measured = spectrum(&alternating, rate()).expect("measurable");
    let points = measured.points();
    let beside = measured.amplitudes()[usize::try_from(points / 2 - 1).expect("small")];
    let wanted =
        2.0_f64 * 0.5_f64 * transform(&reference_window(frames), 1.0_f64 / f64::from(points));
    // Tolerance: as for a sine on a point, a sum over 1000 frames and the 12 stages of the
    // transform, about 1e-13 at worst: 1e-12 is that bound with a factor 10 to spare.
    assert!(
        (beside - wanted).abs() < 1e-12_f64,
        "{beside} instead of {wanted}"
    );
    assert!(
        beside > 0.97_f64 && beside <= 1.0_f64,
        "{beside}: about twice 0.5"
    );
}
