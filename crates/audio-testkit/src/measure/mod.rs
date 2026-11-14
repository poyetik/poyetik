//! Objective measurements of a signal: its spectrum, and for a known fundamental its THD,
//! THD+N, SNR, SFDR and aliasing level, in `f64`, as `spec/measurements.md` defines them.
//!
//! The levels come from least-squares fits of sines at known frequencies (the
//! three-parameter sine fit of IEEE 1241, extended to the harmonics), not from the bins of
//! a spectrum: a fit has no leakage and no scalloping, so a component between two bins
//! measures as one on a bin, and the fundamental is removed to the rounding of `f64`
//! before anything is compared with it. Only the strongest spur, which can lie anywhere,
//! is read from the spectrum of what the sine fit leaves ([`spectrum()`]).
//!
//! The same measurements are written again, independently, in the oracle
//! (`tooling/oracle/src/measure.ts`); both read the known-answer vectors of
//! `spec/measurement-vectors.json`.

use core::f64::consts::{SQRT_2, TAU};
use core::fmt;
use core::ops::RangeInclusive;

use audio_core::SampleRate;

mod spectrum;

pub use spectrum::{PADDING, Spectrum, WINDOW, spectrum};

/// The highest harmonic order analyzed: harmonics 2 to `HARMONICS` count in the THD and the
/// aliasing level.
pub const HARMONICS: u32 = 10;

/// The longest signal measured, in frames. It keeps every frame index below 2^26, which
/// the exact phase reduction needs (see `cycles`).
pub const MAX_FRAMES: u32 = 131_072;

// Dekker's product in `cycles` leaves the frame whole: exact only below 2^26.
const _: () = assert!(MAX_FRAMES < 1 << 26_u32, "frames below 2^26");

/// The largest conditioning a fit accepts (see [`Quality::conditioning`]).
///
/// Above it, components are too close to be told apart over the signal's length, and the fit is
/// refused ([`MeasureError::Unresolvable`]).
pub const CONDITIONING_MAX: f64 = 0.5_f64;

/// Iterations of the Jacobi solver: each divides the distance to the solution by
/// `1 / conditioning`, at least 2, so `2^-JACOBI_ITERATIONS` of the right-hand side is left.
pub const JACOBI_ITERATIONS: u32 = 64;

/// The relative floor of the measurements, 2^-36.
///
/// Every amplitude, root mean square and
/// spur amplitude a measurement computes is within `FLOOR · (peak + Σ|c|)` of its value in
/// exact arithmetic, `peak` being the largest absolute sample and `Σ|c|` the sum of the
/// absolute fitted coefficients (`spec/measurements.md`, "Error of the measurements").
pub const FLOOR: f64 = 1.455_191_522_836_685_2e-11_f64;

/// Why a signal cannot be measured.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MeasureError {
    /// The signal has no sample.
    NoSamples,
    /// The signal is longer than [`MAX_FRAMES`].
    TooLong {
        /// Its length, in frames.
        frames: usize,
    },
    /// A sample is NaN or infinite.
    NotFinite {
        /// The index of the first such sample.
        index: usize,
    },
    /// The fundamental is not a frequency strictly between 0 and Nyquist.
    Fundamental {
        /// The frequency given, in hertz.
        hz: f64,
    },
    /// The signal has fewer samples than a fit has unknowns.
    TooShort {
        /// Its length, in frames.
        frames: u32,
        /// The number of unknowns of the fit.
        needed: u32,
    },
    /// The components of a fit are too close to be told apart over the signal's length.
    Unresolvable {
        /// The conditioning of the fit, above [`CONDITIONING_MAX`].
        conditioning: f64,
    },
    /// The fitted fundamental is not above the floor of the measurement: there is no
    /// fundamental to compare anything with.
    NoFundamental {
        /// The fitted amplitude of the fundamental.
        amplitude: f64,
    },
}

impl fmt::Display for MeasureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSamples => write!(f, "the signal has no sample"),
            Self::TooLong { frames } => write!(
                f,
                "the signal has {frames} frames, more than the {MAX_FRAMES} measured"
            ),
            Self::NotFinite { index } => write!(f, "sample {index} is not finite"),
            Self::Fundamental { hz } => write!(
                f,
                "the fundamental, {hz} Hz, is not strictly between 0 and Nyquist"
            ),
            Self::TooShort { frames, needed } => write!(
                f,
                "the signal has {frames} frames, fewer than the {needed} unknowns of the fit"
            ),
            Self::Unresolvable { conditioning } => write!(
                f,
                "the components are too close to be told apart (conditioning {conditioning}, at most {CONDITIONING_MAX})"
            ),
            Self::NoFundamental { amplitude } => write!(
                f,
                "the fundamental, of amplitude {amplitude}, is not above the floor of the measurement"
            ),
        }
    }
}

impl core::error::Error for MeasureError {}

/// A length as a `usize`: lossless on every target the test bench runs on, where `usize`
/// has at least 32 bits.
fn index(value: u32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

/// The length of `samples`, once checked: at least one sample, at most [`MAX_FRAMES`],
/// every one finite.
fn checked(samples: &[f64]) -> Result<u32, MeasureError> {
    if samples.is_empty() {
        return Err(MeasureError::NoSamples);
    }
    let frames = u32::try_from(samples.len())
        .ok()
        .filter(|&frames| frames <= MAX_FRAMES)
        .ok_or(MeasureError::TooLong {
            frames: samples.len(),
        })?;
    samples
        .iter()
        .position(|sample| !sample.is_finite())
        .map_or(Ok(frames), |index| Err(MeasureError::NotFinite { index }))
}

/// A sum of `f64` terms, compensated: each addition's rounding error is kept exactly
/// (Knuth's `TwoSum`) and added at the end, so the sum of `N` terms errs by at most half an
/// ulp of the result plus `N² · 2^-106` times the sum of the absolute terms (Ogita, Rump
/// and Oishi, 2005, "Sum2"), however long the signal.
#[derive(Debug, Clone, Copy, Default)]
struct Sum {
    /// The rounded sum.
    high: f64,
    /// The sum of the rounding errors.
    low: f64,
}

impl Sum {
    /// Adds `term`.
    fn add(&mut self, term: f64) {
        let total = self.high + term;
        let back = total - self.high;
        self.low += (self.high - (total - back)) + (term - back);
        self.high = total;
    }

    /// The sum.
    fn value(self) -> f64 {
        self.high + self.low
    }
}

/// `value` split into two halves of at most 26 significant bits each, whose sum is
/// `value` exactly (Veltkamp's splitting, with `2^27 + 1`).
fn split(value: f64) -> (f64, f64) {
    let scaled = 134_217_729.0_f64 * value;
    let high = scaled - (scaled - value);
    (high, value - high)
}

/// The phase of a sine of `frequency` hertz at `frame`, at `rate` hertz, in cycles,
/// within about `2^-51` of `frequency · frame / rate` modulo 1 (as a value near `[0, 1)`).
///
/// The product `frequency · frame` is computed exactly, as a rounded product and its
/// error (Dekker's product, with `frame` below 2^26 left whole), the rounded part is
/// reduced modulo `rate` exactly (`%` on `f64` is exact), then the error is added: one
/// rounding, of a value below `rate` plus a part in 2^53. So the phase loses nothing
/// however far the frame, unlike `frequency · frame / rate` (`spec/measurements.md`).
fn cycles(frequency: f64, frame: u32, rate: f64) -> f64 {
    let whole = f64::from(frame);
    let product = frequency * whole;
    let (high, low) = split(frequency);
    let error = (high * whole - product) + low * whole;
    ((product % rate) + error) / rate
}

/// Whether `multiple · fundamental` is a whole number of times `rate`: the frequencies of
/// two orders `k` and `j` fold onto the same frequency in `[0, rate / 2]` exactly when
/// `(k - j)` or `(k + j)` times the fundamental is, and order `k` folds onto DC or Nyquist
/// when `2k` times it is. Exact for a binary32 fundamental (the products have at most 29
/// significant bits, and `%` is exact).
fn whole_turns(multiple: f64, fundamental: f64, rate: f64) -> bool {
    (multiple * fundamental) % rate == 0.0_f64
}

/// One frequency of a fit: an order of the fundamental, folded into `[0, rate / 2]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Component {
    /// The lowest harmonic order that lands on it: 0 for DC, 1 for the fundamental.
    order: u32,
    /// Whether it lies at DC or at Nyquist, where a sine has a single basis function
    /// (`1`, or `(-1)^n`) instead of a cosine and a sine.
    edge: bool,
}

/// The distinct frequencies of `orders` of `fundamental`, folded: each once, with the
/// lowest order landing on it (a harmonic that folds onto a lower one, the fundamental or
/// DC is told apart from it by nothing, and counts as it).
fn components(fundamental: f64, rate: f64, orders: RangeInclusive<u32>) -> Vec<Component> {
    let mut found: Vec<Component> = Vec::new();
    for order in orders {
        let this = f64::from(order);
        let distinct = found.iter().all(|component| {
            let other = f64::from(component.order);
            !whole_turns(this - other, fundamental, rate)
                && !whole_turns(this + other, fundamental, rate)
        });
        if distinct {
            let edge = whole_turns(2.0_f64 * this, fundamental, rate);
            found.push(Component { order, edge });
        }
    }
    found
}

/// One unknown of a fit: a basis function of a component, and the mean square of that
/// function over a period relative to 1 (1 at an edge, where it is `1` or `(-1)^n`; `1/2`
/// for a cosine or a sine). A coefficient `c` of it contributes `weight · c²` to the power
/// of its component.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Unknown {
    /// The component it belongs to.
    component: Component,
    /// The mean square of its basis function: 1 or `1/2`.
    weight: f64,
}

/// The unknowns of `components`, in the order of [`basis`]: one at an edge, a cosine and a
/// sine elsewhere.
fn unknowns(components: &[Component]) -> Vec<Unknown> {
    components
        .iter()
        .flat_map(|&component| {
            if component.edge {
                vec![Unknown {
                    component,
                    weight: 1.0_f64,
                }]
            } else {
                vec![
                    Unknown {
                        component,
                        weight: 0.5_f64,
                    };
                    2
                ]
            }
        })
        .collect()
}

/// Writes the value of every basis function at the phase `phase` (in cycles of the
/// fundamental) into `row`: for each component of order `k`, `cos 2π·kφ` and, off the
/// edges, `sin 2π·kφ`, `kφ` reduced to its fraction first. DC is exactly 1.
fn basis(components: &[Component], phase: f64, row: &mut Vec<f64>) {
    row.clear();
    for component in components {
        let turns = f64::from(component.order) * phase;
        let angle = TAU * (turns - turns.floor());
        row.push(libm::cos(angle));
        if !component.edge {
            row.push(libm::sin(angle));
        }
    }
}

/// What a fit found.
#[derive(Debug, Clone, PartialEq)]
struct Fit {
    /// The unknowns fitted, each with its coefficient.
    fitted: Vec<(Unknown, f64)>,
    /// The residual: the signal less the fitted components.
    residual: Vec<f64>,
    /// The mean square of the residual.
    residual_power: f64,
    /// The conditioning and the floor of the fit.
    quality: Quality,
}

impl Fit {
    /// The total power (mean square) of the components `selected` keeps: `(a² + b²) / 2`
    /// for a cosine and a sine of coefficients `a` and `b`, `c²` at an edge; `None` when it
    /// keeps none.
    fn power(&self, selected: impl Fn(&Component) -> bool) -> Option<f64> {
        let kept: Vec<f64> = self
            .fitted
            .iter()
            .filter(|(unknown, _)| selected(&unknown.component))
            .map(|(unknown, coefficient)| unknown.weight * coefficient * coefficient)
            .collect();
        (!kept.is_empty()).then(|| kept.iter().sum())
    }

    /// The power of the fundamental, a component of every fit (order 1, strictly between
    /// DC and Nyquist): as [`Fit::power`], summed.
    fn fundamental_power(&self) -> f64 {
        self.fitted
            .iter()
            .filter(|(unknown, _)| unknown.component.order == 1)
            .map(|(unknown, coefficient)| unknown.weight * coefficient * coefficient)
            .sum()
    }
}

/// Solves the normal equations of a fit: `gram` (row by row) and `projections` are the
/// sums `Σ φᵢφⱼ` and `Σ φᵢx`, `scales` the inverse nominal norms of the basis functions.
/// Returns the coefficients and the conditioning (see [`fit`]).
fn solve(
    gram: &[Sum],
    projections: &[Sum],
    scales: &[f64],
) -> Result<(Vec<f64>, f64), MeasureError> {
    // The scaled Gram matrix less the identity, row by row.
    let deviation: Vec<Vec<f64>> = gram
        .chunks_exact(scales.len())
        .zip(scales)
        .enumerate()
        .map(|(i, (sums, &left))| {
            sums.iter()
                .zip(scales)
                .enumerate()
                .map(|(j, (sum, &right))| {
                    let entry = sum.value() * left * right;
                    if i == j { entry - 1.0_f64 } else { entry }
                })
                .collect()
        })
        .collect();
    let conditioning = deviation
        .iter()
        .map(|values| values.iter().map(|value| value.abs()).sum::<f64>())
        .max_by(f64::total_cmp)
        .unwrap_or_default();
    if conditioning > CONDITIONING_MAX {
        return Err(MeasureError::Unresolvable { conditioning });
    }
    let right: Vec<f64> = projections
        .iter()
        .zip(scales)
        .map(|(sum, &scale)| sum.value() * scale)
        .collect();
    let mut solution = right.clone();
    for _ in 0..JACOBI_ITERATIONS {
        solution = deviation
            .iter()
            .zip(&right)
            .map(|(values, &target)| {
                target
                    - values
                        .iter()
                        .zip(&solution)
                        .map(|(value, unknown)| value * unknown)
                        .sum::<f64>()
            })
            .collect();
    }
    let coefficients = solution
        .iter()
        .zip(scales)
        .map(|(unknown, scale)| unknown * scale)
        .collect();
    Ok((coefficients, conditioning))
}

/// Fits the components of `orders` of `fundamental` to `samples` by least squares
/// (`spec/measurements.md`, "Fits").
///
/// The normal equations are scaled by the nominal norms of the basis functions (`N` at an
/// edge, `N / 2` elsewhere), so that the scaled Gram matrix `Ĝ` is the identity for
/// orthogonal components; the conditioning `ρ = maxᵢ Σⱼ |Ĝᵢⱼ - δᵢⱼ|` bounds every eigenvalue
/// of `Ĝ` within `[1 - ρ, 1 + ρ]` (Gershgorin), and the Jacobi iteration
/// `y ← b - (Ĝ - I) y` then converges by a factor `ρ` per step.
fn fit(
    samples: &[f64],
    frames: u32,
    rate: f64,
    fundamental: f64,
    orders: RangeInclusive<u32>,
) -> Result<Fit, MeasureError> {
    let components = components(fundamental, rate, orders);
    let unknowns = unknowns(&components);
    let size = unknowns.len();
    let needed = u32::try_from(size).unwrap_or(u32::MAX);
    if frames < needed {
        return Err(MeasureError::TooShort { frames, needed });
    }
    let mut gram = vec![Sum::default(); size.saturating_mul(size)];
    let mut projections = vec![Sum::default(); size];
    let mut row = Vec::with_capacity(size);
    for (frame, &sample) in (0_u32..).zip(samples) {
        basis(&components, cycles(fundamental, frame, rate), &mut row);
        for (sums, &left) in gram.chunks_exact_mut(size).zip(&row) {
            for (sum, &right) in sums.iter_mut().zip(&row) {
                sum.add(left * right);
            }
        }
        for (sum, &value) in projections.iter_mut().zip(&row) {
            sum.add(value * sample);
        }
    }
    let length = f64::from(frames);
    let scales: Vec<f64> = unknowns
        .iter()
        .map(|unknown| 1.0_f64 / (unknown.weight * length).sqrt())
        .collect();
    let (coefficients, conditioning) = solve(&gram, &projections, &scales)?;
    let mut power = Sum::default();
    let residual: Vec<f64> = (0_u32..)
        .zip(samples)
        .map(|(frame, &sample)| {
            basis(&components, cycles(fundamental, frame, rate), &mut row);
            let model: f64 = row
                .iter()
                .zip(&coefficients)
                .map(|(value, coefficient)| value * coefficient)
                .sum();
            let left = sample - model;
            power.add(left * left);
            left
        })
        .collect();
    let peak = samples
        .iter()
        .map(|sample| sample.abs())
        .max_by(f64::total_cmp)
        .unwrap_or_default();
    let total: f64 = coefficients
        .iter()
        .map(|coefficient| coefficient.abs())
        .sum();
    Ok(Fit {
        fitted: unknowns.into_iter().zip(coefficients).collect(),
        residual,
        residual_power: power.value() / length,
        quality: Quality {
            conditioning,
            floor: FLOOR * (peak + total),
        },
    })
}

/// `10 · log10(ratio)`: a power ratio in decibels.
fn decibels(ratio: f64) -> f64 {
    10.0_f64 * libm::log10(ratio)
}

/// The measurements of a signal with a known fundamental (`spec/measurements.md`). Every
/// level is in decibels relative to the fundamental (dBc).
#[derive(Debug, Clone, PartialEq)]
pub struct Measurements {
    /// The amplitude of the fundamental, from the sine fit, in the unit of the signal.
    pub fundamental: f64,
    /// THD+N: the power of everything but the fundamental and DC (the residual of the sine
    /// fit), relative to the fundamental's, in dB. Its negation is the SINAD of IEEE 1241.
    pub thd_plus_n: f64,
    /// SFDR: the fundamental's amplitude relative to the strongest point of the spectrum of
    /// the sine fit's residual, in dB.
    pub sfdr: f64,
    /// The quality of the sine fit (DC and the fundamental).
    pub sine: Quality,
    /// The equivalent noise bandwidth of the window over the signal, in bins (see
    /// [`Spectrum::enbw`]).
    pub enbw: f64,
    /// What the harmonic fit found, or why it was refused: harmonics folded too close to one
    /// another, to DC, the fundamental or Nyquist, to be told apart over the signal's
    /// length ([`MeasureError::Unresolvable`], [`MeasureError::TooShort`]).
    pub harmonics: Result<Harmonics, MeasureError>,
}

/// How far a fit's results can be from their exact values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quality {
    /// The conditioning of the fit, in `[0, CONDITIONING_MAX]`: 0 when its components are
    /// orthogonal over the signal; it bounds how far the fit's errors can exceed the
    /// error of the signal (see [`sine_bounds`]).
    pub conditioning: f64,
    /// The absolute floor of the fit, `FLOOR · (peak + Σ|c|)`, in the unit of the signal:
    /// every amplitude and root mean square it gives is within it of its exact value.
    pub floor: f64,
}

/// The levels of the harmonic fit (DC, the fundamental and its harmonics 2 to
/// [`HARMONICS`], folded), in dB relative to the fundamental it fits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Harmonics {
    /// THD: the power of the harmonics 2 to [`HARMONICS`] (those not landing on DC or the
    /// fundamental), relative to the fundamental's, in dB; `None` when every harmonic lands
    /// on DC or on the fundamental.
    pub thd: Option<f64>,
    /// The aliasing level: the power of the harmonics above Nyquist, folded below it (those
    /// not landing on DC, the fundamental or a harmonic below Nyquist), relative to the
    /// fundamental's, in dB; `None` when no harmonic up to [`HARMONICS`] lies above Nyquist
    /// apart from those.
    pub aliasing: Option<f64>,
    /// SNR: the fundamental's power relative to the residual of the harmonic fit (neither
    /// DC, the fundamental nor a harmonic), in dB.
    pub snr: f64,
    /// The quality of the harmonic fit.
    pub quality: Quality,
}

/// The levels of the harmonic fit of `samples`.
fn harmonics(
    samples: &[f64],
    frames: u32,
    rate: f64,
    fundamental: f64,
) -> Result<Harmonics, MeasureError> {
    let fitted = fit(samples, frames, rate, fundamental, 0..=HARMONICS)?;
    let reference = fitted.fundamental_power();
    Ok(Harmonics {
        thd: fitted
            .power(|component| component.order > 1)
            .map(|harmonics| decibels(harmonics / reference)),
        aliasing: fitted
            // Neither DC nor the fundamental lies above Nyquist: only harmonics can.
            .power(|component| f64::from(component.order) * fundamental > rate / 2.0_f64)
            .map(|aliased| decibels(aliased / reference)),
        snr: decibels(reference / fitted.residual_power),
        quality: fitted.quality,
    })
}

/// Measures `samples`, at `rate`, whose fundamental is `fundamental` hertz
/// (`spec/measurements.md`).
///
/// # Errors
///
/// - [`MeasureError::NoSamples`], [`MeasureError::TooLong`], [`MeasureError::NotFinite`]
///   for a signal that cannot be measured at all;
/// - [`MeasureError::Fundamental`] for a fundamental not strictly between 0 and Nyquist;
/// - [`MeasureError::TooShort`] or [`MeasureError::Unresolvable`] when the sine fit cannot
///   tell DC and the fundamental apart over the signal's length (the harmonic fit reports
///   the same errors in [`Measurements::harmonics`]);
/// - [`MeasureError::NoFundamental`] when the fitted fundamental is not above the floor.
pub fn measure(
    samples: &[f64],
    rate: SampleRate,
    fundamental: f64,
) -> Result<Measurements, MeasureError> {
    let frames = checked(samples)?;
    let hz = f64::from(rate.hz());
    let inside = fundamental > 0.0_f64 && fundamental < hz / 2.0_f64;
    if !inside {
        return Err(MeasureError::Fundamental { hz: fundamental });
    }
    let sine = fit(samples, frames, hz, fundamental, 0..=1)?;
    let power = sine.fundamental_power();
    let amplitude = (2.0_f64 * power).sqrt();
    if amplitude <= sine.quality.floor {
        return Err(MeasureError::NoFundamental { amplitude });
    }
    let spurs = spectrum(&sine.residual, rate)?;
    Ok(Measurements {
        fundamental: amplitude,
        thd_plus_n: decibels(sine.residual_power / power),
        sfdr: 2.0_f64 * decibels(amplitude / spurs.peak()),
        sine: sine.quality,
        enbw: spurs.enbw(),
        harmonics: harmonics(samples, frames, hz, fundamental),
    })
}

/// The worst levels a signal can measure when each of its samples lies within `error` of
/// an ideal sine of amplitude `amplitude` at the fundamental.
///
/// Defined in `spec/measurements.md`, "Bounds for a sine": a box producing a sine meets its
/// thresholds when its measurements stay within these, `error` being its derived error
/// bound per sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    /// The highest THD+N, in dB.
    pub thd_plus_n: f64,
    /// The lowest SFDR, in dB.
    pub sfdr: f64,
    /// The bounds of the harmonic fit's levels, when it was not refused.
    pub harmonics: Option<HarmonicBounds>,
}

/// The worst levels of the harmonic fit (see [`Bounds`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HarmonicBounds {
    /// The highest THD, and aliasing level, in dB.
    pub thd: f64,
    /// The lowest SNR, in dB.
    pub snr: f64,
}

/// `(b, a)`: the error per sample `b = error + floor` of a fit of quality `quality`, and
/// `a = amplitude - b·√(2(1 + ρ)) / (1 - ρ)`, the lowest its fundamental can measure;
/// `None` when that is not above zero.
fn margins(amplitude: f64, error: f64, quality: Quality) -> Option<(f64, f64)> {
    let rho = quality.conditioning;
    let total = error + quality.floor;
    let lowest = amplitude.abs() - total * (2.0_f64 * (1.0_f64 + rho)).sqrt() / (1.0_f64 - rho);
    (lowest > 0.0_f64).then_some((total, lowest))
}

/// The [`Bounds`] of a signal within `error` per sample of an ideal sine of `amplitude`.
///
/// They hold for the fits and the window of `measured`; `None` when `error` is too large
/// to bound anything (the fundamental could then measure zero), and no harmonic bounds
/// when the harmonic fit was refused or `error` is too large for it.
///
/// With `b` and `a` as `margins` gives them and `ρ` the conditioning of the fit:
/// - THD+N ≤ `20 log10(√2·b / a)` and SNR ≥ its negation: the residual of a least-squares
///   fit whose span holds the ideal sine is at most the signal's distance from it, `√N·b`;
/// - THD and aliasing ≤ `20 log10(√2·b·√(1 + ρ) / ((1 - ρ)·a))`: the fitted harmonics are
///   the fit of the error alone, whose scaled norm is at most `√((1 + ρ)·N)·b / (1 - ρ)`;
/// - SFDR ≥ `20 log10(a / (2·b·√ENBW))`: by Cauchy-Schwarz, no point of a windowed
///   spectrum exceeds `2·‖d‖·√(Σw²) / Σw` for a residual `d` of norm at most `√N·b`.
#[must_use]
pub fn sine_bounds(amplitude: f64, error: f64, measured: &Measurements) -> Option<Bounds> {
    let (total, lowest) = margins(amplitude, error, measured.sine)?;
    let harmonics = measured.harmonics.as_ref().ok().and_then(|harmonics| {
        let rho = harmonics.quality.conditioning;
        let (error_bound, fundamental) = margins(amplitude, error, harmonics.quality)?;
        Some(HarmonicBounds {
            thd: 2.0_f64
                * decibels(
                    SQRT_2 * error_bound * (1.0_f64 + rho).sqrt() / ((1.0_f64 - rho) * fundamental),
                ),
            snr: -2.0_f64 * decibels(SQRT_2 * error_bound / fundamental),
        })
    });
    Some(Bounds {
        thd_plus_n: 2.0_f64 * decibels(SQRT_2 * total / lowest),
        sfdr: 2.0_f64 * decibels(lowest / (2.0_f64 * total * measured.enbw.sqrt())),
        harmonics,
    })
}

#[cfg(test)]
#[path = "measure_tests.rs"]
mod tests;
