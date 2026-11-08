//! The amplitude spectrum of a signal: a Blackman-Harris window, zero padding and a
//! radix-2 FFT, all in `f64` (`spec/measurements.md`, "Spectrum").

use core::f64::consts::{PI, TAU};
use core::iter;

use engine_core::SampleRate;

use super::{MeasureError, checked};

/// The coefficients `a0` to `a3` of the four-term Blackman-Harris window (Harris, 1978).
///
/// `w[n] = a0 - a1 cos(2πn/N) + a2 cos(4πn/N) - a3 cos(6πn/N)`, periodic over the `N`
/// samples. Its highest sidelobe is 92 dB below its peak (checked by
/// `the_window_keeps_its_sidelobes_92_db_down`), and every value is positive.
pub const WINDOW: [f64; 4] = [0.358_75_f64, 0.488_29_f64, 0.141_28_f64, 0.011_68_f64];

/// How many more points the spectrum has than the signal has samples, at least.
///
/// The signal is zero-padded to the next power of two, so that a component lies at most an
/// eighth of a bin of the signal (`1 / (2 · PADDING · N)` cycle per sample) from a point.
pub const PADDING: u32 = 4;

/// The window of [`WINDOW`] over `frames` samples.
fn window(frames: u32) -> Vec<f64> {
    let length = f64::from(frames);
    (0_u32..frames)
        .map(|index| {
            let term = |order: f64, coefficient: f64| {
                coefficient * libm::cos(TAU * order * f64::from(index) / length)
            };
            let [a0, a1, a2, a3] = WINDOW;
            a0 - term(1.0_f64, a1) + term(2.0_f64, a2) - term(3.0_f64, a3)
        })
        .collect()
}

/// A complex number, in `f64`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Complex {
    /// Real part.
    re: f64,
    /// Imaginary part.
    im: f64,
}

impl Complex {
    /// `self + other`.
    fn plus(self, other: Self) -> Self {
        Self {
            re: self.re + other.re,
            im: self.im + other.im,
        }
    }

    /// `self - other`.
    fn minus(self, other: Self) -> Self {
        Self {
            re: self.re - other.re,
            im: self.im - other.im,
        }
    }

    /// `self · other`.
    fn times(self, other: Self) -> Self {
        Self {
            re: self.re * other.re - self.im * other.im,
            im: self.re * other.im + self.im * other.re,
        }
    }
}

/// Zero.
const ZERO: Complex = Complex {
    re: 0.0_f64,
    im: 0.0_f64,
};

/// The discrete Fourier transform of `signal`, whose length is a power of two, `length`
/// as an `f64`:
/// `X[k] = Σ x[n] e^(-2πikn/M)`.
///
/// A Stockham radix-2 FFT (decimation in frequency, sorted as it goes, so no bit
/// reversal): stage by stage, span `n` from `M` down to 2 and stride `s = M / n`, the
/// rows `p` and `p + n/2` of the input (each `s` values) give the rows `2p`, their sum,
/// and `2p + 1`, their difference times `e^(-2πip/n)`. The twiddle factors are evaluated
/// once, each from an exact angle (`p / M` is exact, `M` a power of two). Its rounding
/// error is bounded in `spec/measurements.md` ("Error of the spectrum").
fn fft(signal: Vec<Complex>, length: f64) -> Vec<Complex> {
    let points = signal.len();
    let half = points / 2;
    let twiddles: Vec<Complex> = (0_u32..)
        .take(half)
        .map(|index| {
            let angle = -TAU * f64::from(index) / length;
            Complex {
                re: libm::cos(angle),
                im: libm::sin(angle),
            }
        })
        .collect();
    let mut input = signal;
    let mut output = vec![ZERO; points];
    let strides = iter::successors(Some((1_usize, 2_usize)), |&(_, double)| {
        double.checked_mul(2).map(|next| (double, next))
    });
    for (stride, double) in strides.take_while(|&(stride, _)| stride < points) {
        let (top, bottom) = input.split_at(half);
        let rows = top.chunks_exact(stride).zip(bottom.chunks_exact(stride));
        let pairs = output.chunks_exact_mut(double);
        for ((first, second), (pair, twiddle)) in
            rows.zip(pairs.zip(twiddles.iter().step_by(stride)))
        {
            let (even, odd) = pair.split_at_mut(stride);
            for (((&upper, &lower), sum), difference) in first.iter().zip(second).zip(even).zip(odd)
            {
                *sum = upper.plus(lower);
                *difference = upper.minus(lower).times(*twiddle);
            }
        }
        core::mem::swap(&mut input, &mut output);
    }
    input
}

/// The amplitude spectrum of a signal (`spec/measurements.md`, "Spectrum").
#[derive(Debug, Clone, PartialEq)]
pub struct Spectrum {
    /// The amplitude at each point `k` from 0 to `M / 2`, in the unit of the signal.
    amplitudes: Vec<f64>,
    /// The sample rate of the signal, in hertz.
    rate: f64,
    /// `M`: the number of points of the transform.
    points: u32,
    /// The window's equivalent noise bandwidth, in bins of the signal.
    enbw: f64,
    /// The largest relative loss of a component's amplitude between two points.
    scalloping: f64,
}

impl Spectrum {
    /// The amplitude at each point `k`, from 0 (DC) to `M / 2` (Nyquist), in the unit of
    /// the signal: a sine of amplitude `a` at a point measures `a` there, as does a
    /// constant `a` at DC and `a · (-1)^n` at Nyquist.
    #[must_use]
    pub fn amplitudes(&self) -> &[f64] {
        &self.amplitudes
    }

    /// The frequency of point `k`, in hertz: `k · rate / M`.
    #[must_use]
    pub fn frequency(&self, point: u32) -> f64 {
        f64::from(point) * self.rate / f64::from(self.points)
    }

    /// `M`, the number of points of the transform: the smallest power of two at least
    /// [`PADDING`] times the signal's length.
    #[must_use]
    pub const fn points(&self) -> u32 {
        self.points
    }

    /// The equivalent noise bandwidth of the window over the signal's `N` samples, in
    /// bins of `rate / N`: `N Σw² / (Σw)²`.
    #[must_use]
    pub const fn enbw(&self) -> f64 {
        self.enbw
    }

    /// The largest relative loss of the measured amplitude of a sine between two points
    /// (the scalloping loss): `π² m₂ / (2 M²)`, where `m₂ = Σ w[n] (n - N/2)² / Σ w[n]`.
    /// A sine of amplitude `a` alone measures at least `a · (1 - loss)` at its nearest
    /// point, less the leakage of its own image at minus its frequency.
    #[must_use]
    pub const fn scalloping(&self) -> f64 {
        self.scalloping
    }

    /// The largest amplitude of the spectrum, in the unit of the signal (never empty: a
    /// spectrum has at least the points DC and Nyquist).
    #[must_use]
    pub fn peak(&self) -> f64 {
        self.amplitudes
            .iter()
            .copied()
            .max_by(f64::total_cmp)
            .unwrap_or_default()
    }
}

/// The amplitude spectrum of `samples` at `rate` (`spec/measurements.md`, "Spectrum").
///
/// # Errors
///
/// [`MeasureError::NoSamples`], [`MeasureError::TooLong`] or
/// [`MeasureError::NotFinite`]: an empty, too long or non-finite signal.
pub fn spectrum(samples: &[f64], rate: SampleRate) -> Result<Spectrum, MeasureError> {
    let frames = checked(samples)?;
    let points = frames.saturating_mul(PADDING).next_power_of_two();
    let weights = window(frames);
    let (sum, squares, moment) = (0_u32..).zip(&weights).fold(
        (0.0_f64, 0.0_f64, 0.0_f64),
        |(sum, squares, moment), (index, &weight)| {
            let offset = f64::from(index) - f64::from(frames) / 2.0_f64;
            (
                sum + weight,
                squares + weight * weight,
                moment + weight * offset * offset,
            )
        },
    );
    let signal: Vec<Complex> = samples
        .iter()
        .zip(&weights)
        .map(|(&sample, &weight)| Complex {
            re: sample * weight,
            im: 0.0_f64,
        })
        .chain(iter::repeat(ZERO))
        .take(super::index(points))
        .collect();
    let transform = fft(signal, f64::from(points));
    let edges = [0, points / 2];
    let amplitudes = (0_u32..=points / 2)
        .zip(&transform)
        .map(|(point, value)| {
            let gain = if edges.contains(&point) {
                1.0_f64
            } else {
                2.0_f64
            };
            gain * (value.re * value.re + value.im * value.im).sqrt() / sum
        })
        .collect();
    let length = f64::from(points);
    Ok(Spectrum {
        amplitudes,
        rate: f64::from(rate.hz()),
        points,
        enbw: f64::from(frames) * squares / (sum * sum),
        scalloping: (PI * PI / 2.0_f64) * (moment / sum) / (length * length),
    })
}

#[cfg(test)]
#[path = "spectrum_tests.rs"]
mod tests;
