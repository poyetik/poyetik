# Measurements

The objective measurements every box and every engine component that touches the signal is held to (`CLAUDE.md`, "Audio DSP: the objective"): what each level means, how it is computed, how far the computation itself can err, and how a threshold is derived from first principles. Implemented twice, independently: `audio_testkit::measure` (Rust, `crates/audio-testkit/src/measure/`) and the oracle's `src/measure.ts` (TypeScript, `tooling/oracle/`), which both read the known-answer vectors of [`measurement-vectors.json`](measurement-vectors.json).

Version: `measurements/1`, the `scheme` of the vectors. A change of a definition below changes the version.

## Standards followed

| Level | Standard | Here |
|---|---|---|
| Sine fit | IEEE Std 1241-2010, 4.1.4.1 (three-parameter least-squares fit, known frequency); IEEE Std 1057-2017 | The same fit (DC, cosine and sine at the fundamental), solved as below |
| SINAD, THD+N | IEEE Std 1241-2010, 4.5.3 (SINAD); IEC 60268-3 and AES17 (THD+N) | THD+N is minus the SINAD of IEEE 1241: the residual of the sine fit relative to the fundamental. **Deviations:** relative to the fundamental, not to the total signal (IEC 60268-3, AES17), which differ by `10 log10(1 + 10^(THD+N/10))` dB, below 10^-6 dB at -60 dB; no measurement bandwidth (AES17 uses 20 kHz): the whole band from DC to Nyquist counts; the fit replaces the notch filter |
| THD | IEEE Std 1241-2010, 4.5.5 | Harmonics 2 to `HARMONICS` (10), aliased ones included, each fitted by least squares at its exact folded frequency rather than read from bins. **Deviation:** a harmonic folding onto DC or the fundamental cannot be told apart from it and counts as it |
| SNR | IEEE Std 1241-2010, 4.5.4 | The residual of the harmonic fit: neither DC, the fundamental nor a harmonic 2 to 10 |
| SFDR | IEEE Std 1241-2010, 4.5.6 | The strongest point of the spectrum of the sine fit's residual, anywhere from DC to Nyquist |
| Aliasing level | No standard | Defined below |
| Window | Harris, "On the use of windows for harmonic analysis with the discrete Fourier transform", Proc. IEEE 66(1), 1978 | The four-term Blackman-Harris window |

## Notation

`x[n]`, `n = 0 … N-1`: the signal, `f64` (binary32 samples convert exactly). `fs`: the sample rate in hertz. `f1`: the fundamental in hertz, known. Levels are in dB relative to the fundamental (dBc): `10 log10` of a power ratio, `20 log10` of an amplitude ratio.

## Inputs and refusals

A measurement refuses, with a typed error and never a number:

| Error | When |
|---|---|
| `NoSamples` | `N = 0` |
| `TooLong` | `N > MAX_FRAMES` (2^17, 131,072 frames): keeps every frame index below 2^26, which the phase reduction needs, and the oracle's measurement of a long case to a few seconds (it measures the first `MAX_FRAMES` frames of a longer one, its error bound taken at that length) |
| `NotFinite` | a sample is NaN or infinite (the first one is named) |
| `Fundamental` | `f1` is not strictly between 0 and `fs / 2` (NaN included): a sine at DC or Nyquist has no phase to fit |
| `TooShort` | `N` is below the number of unknowns of a fit |
| `Unresolvable` | a fit's conditioning exceeds `CONDITIONING_MAX` (1/2): its components are too close over `N` samples to be told apart |
| `NoFundamental` | the fitted fundamental's amplitude is not above the fit's floor |

The sine fit's refusals refuse the whole measurement; the harmonic fit's refuse only its levels (THD, aliasing, SNR), reported as such.

## Phase

The phase of `f1` at frame `n`, in cycles, is `φ[n] = (f1 · n mod fs) / fs`, computed so that nothing is lost however far `n` is: the product `f1 · n` is computed exactly as a rounded product `p` and its error `e` (Dekker's product, `f1` split in two halves of 26 bits by Veltkamp's method with `2^27 + 1`, `n` below 2^26 left whole), `p mod fs` is exact in floating point, then `φ = ((p mod fs) + e) / fs` rounds twice: within `2^-51` cycle. Harmonic `k` has the phase `k·φ` less its whole cycles: within `k · 2^-51 + 2^-53` cycle.

## Fits

A fit is a least-squares fit of `x` by sines at known frequencies (`fit` in the Rust code, `fit` in the oracle).

- **Components.** For orders `k` from 0 to `K` (0 to 1 for the **sine fit**, 0 to `HARMONICS` for the **harmonic fit**), the frequency `k · f1` folded into `[0, fs/2]` (`r = (k·f1) mod fs`, then `fs - r` if `r > fs/2`). Each distinct folded frequency is kept once, owned by the lowest order landing on it (0 is DC): orders `k` and `j` land together exactly when `(k - j)·f1` or `(k + j)·f1` is a whole number of times `fs`, which is decided exactly (`mod` is exact, and the products are for a binary32 `f1`). A component at DC or Nyquist, where `2k·f1` is a whole number of times `fs`, is an **edge**.
- **Basis.** For each component of order `k`: `cos(2π·kφ[n])` and, off the edges, `sin(2π·kφ[n])`. Each basis function has a **weight**, its mean square over a period: 1 at an edge (`1` or `(-1)^n`), 1/2 otherwise.
- **Normal equations, scaled.** `G = Σ φᵢ[n] φⱼ[n]` and `b = Σ φᵢ[n] x[n]`, every sum compensated (Knuth's TwoSum, Ogita, Rump and Oishi's Sum2: within half an ulp of the result plus `N²·2^-106` of the sum of magnitudes). With `sᵢ = 1 / √(weightᵢ · N)`, `Ĝ = S G S` and `b̂ = S b`; `Ĝ` is the identity for components orthogonal over the signal (a whole number of periods each).
- **Conditioning.** `ρ = maxᵢ Σⱼ |Ĝᵢⱼ - δᵢⱼ|`. By Gershgorin's theorem every eigenvalue of `Ĝ` lies in `[1 - ρ, 1 + ρ]`, and `‖Ĝ⁻¹‖∞ ≤ 1 / (1 - ρ)`. A fit with `ρ > 1/2` is refused.
- **Solution.** The Jacobi iteration `y ← b̂ - (Ĝ - I) y`, from `y = b̂`, `JACOBI_ITERATIONS` (64) times: it contracts by `ρ ≤ 1/2` per step, so `2^-64` of the start is left. The coefficients are `c = S y`.
- **Powers.** A component's power is `Σ weight · c²` over its unknowns: `(a² + b²) / 2` for a cosine and a sine, `c²` at an edge. Its amplitude is `√(2 · power)`.
- **Residual.** `r[n] = x[n] - Σ cⱼ φⱼ[n]`, and its mean square.

## Levels

With `A1` the fundamental's amplitude and `P1 = A1² / 2` its power:

- **Fundamental**: `A1` of the sine fit.
- **THD+N** = `10 log10(mean(d²) / P1)`, `d` the residual of the sine fit. Harmonics not orthogonal to the fundamental over the signal leak into the sine fit's fundamental, by at most `√(2(1 + ρ)) / (1 - ρ)` times their root mean square (the lemma below, applied to them), and not at all over whole periods.
- **THD** = `10 log10(Σ P_h / P1)` over the harmonic fit's components of order 2 or more, `P1` from the same fit; none when there is no such component.
- **Aliasing level** = the same sum over the components whose order `k` has `k · f1 > fs / 2`: harmonics above Nyquist, folded, that land on no lower order, DC or the fundamental; none when there is no such component.
- **SNR** = `10 log10(P1 / mean(r²))`, `r` the residual of the harmonic fit.
- **SFDR** = `20 log10(A1 / max_k a[k])` over the spectrum `a` of `d`.

## Spectrum

`spectrum(x)`: the amplitude spectrum of a signal.

- **Window**: `w[n] = a0 - a1 cos(2πn/N) + a2 cos(4πn/N) - a3 cos(6πn/N)`, periodic, with `a0 … a3 = 0.35875, 0.48829, 0.14128, 0.01168`. Positive everywhere; highest sidelobe -92 dB (checked on a dense grid); equivalent noise bandwidth `ENBW = N Σw² / (Σw)²`, 2.0044 bins.
- **Points**: the windowed signal is zero-padded to `M`, the smallest power of two at least `PADDING · N` (4N), and transformed by a radix-2 Stockham FFT with twiddle factors computed once from exact angles. Point `k` (0 to `M/2`) lies at `k · fs / M` hertz.
- **Amplitude**: `a[k] = 2 |X[k]| / Σw` inside, `|X[k]| / Σw` at DC and Nyquist: a sine of amplitude `a` on a point reads `a`, as does `a` at DC and `a · (-1)^n` at Nyquist.
- **Scalloping**: a sine between points reads at least `a · (1 - L)` at its nearest point, less the leakage of its image at minus its frequency, with `L = π² m₂ / (2 M²)`, `m₂ = Σ w[n] (n - N/2)² / Σw`: since `w ≥ 0` and `cos u ≥ 1 - u²/2`, `|W(δ)| ≥ Σ w[n] cos(2πδ(n - N/2)) ≥ Σw · (1 - 2π²δ² m₂ / Σw)` for an offset `δ ≤ 1/(2M)` cycle. Below 0.6 % (0.052 dB) for every `N` from 2: the window's second moment is at most about `0.0192 N²` and `M ≥ 4N`, so `L ≤ π² · 0.0192 / 32 ≈ 0.592 %` (checked at every `N` from 2 to 4096 and every power of two to 2^20); one sample is the exception, its moment `1/4` and `M = 4` giving `L = π²/128`, about 7.7 %.
- **Edges**: a component at Nyquist (or DC) is a single complex exponential; the points beside it read it with the factor 2 of a sine, up to twice its amplitude: a reading up to 6.02 dB high (conservative for the SFDR), never low.
- **Cauchy-Schwarz**: every point satisfies `a[k] ≤ 2 ‖x‖ √(Σw²) / Σw = 2 · rms(x) · √ENBW`.

## Error of the measurements

Every amplitude, root mean square and spur amplitude a measurement gives is within its **floor**, `FLOOR · (peak + Σ|c|)`, of its value in exact arithmetic, with `FLOOR = 2^-36`, `peak` the largest absolute sample and `Σ|c|` the sum of the absolute coefficients of the fit. The terms, each relative to `peak + Σ|c|` (`bounds_its_own_error_by_the_floor` computes their sum from the constants):

1. a basis value errs by at most `β = 2π·(HARMONICS · 2^-51 + 2^-53) + 2^-52` (the phase, then the sine or cosine within an ulp);
2. evaluating the model and the residual: `(K + 1) · 2^-53`, `K ≤ 2 (HARMONICS + 1)` unknowns;
3. the scaled normal equations err entry by entry by at most `2 · (2β + 2^-52)`, a row by `K` times that, `b̂` by `β·√(2N)·peak` (each of its products has one basis value) plus the rounding of those products and their compensated sum, `2^-53` and `2^-52` of it; with `‖Ĝ⁻¹‖∞ ≤ 1/(1 - ρ)`, the coefficients by `(K · 2 · (2β + 2^-52) + 2β + 2^-53 + 2^-52) / (1 - ρ)`, plus `ρ^64` of three times the right-hand side for the iterations left over;
4. the FFT errs by at most `log2 M · 5 · 2^-53` of the norm of its input (Higham, *Accuracy and Stability of Numerical Algorithms*, 2nd ed., 2002, theorem 24.2, with twiddle factors within an ulp), which on a point amplitude is at most `2·√8 / 0.358` times that relative to the residual's root mean square.

Each language computes these with its own sine, cosine and logarithm (`libm` in Rust, `Math` in JavaScript, both within an ulp): the two agree on every vector within the sum of their tolerances, which the floor dominates.

## Bounds for a sine

A signal whose samples each lie within `ε` of an ideal sine of amplitude `A` at `f1` measures at worst as follows (`sine_bounds`, `sineBounds`), with `b = ε + floor`, `ρ` the conditioning of the fit concerned and `a = A - b·√(2(1 + ρ)) / (1 - ρ)`, the lowest the fundamental can measure (no bound when `a ≤ 0`):

| Level | Bound | Why |
|---|---|---|
| THD+N | `≤ 20 log10(√2·b / a)` | The ideal sine lies in the span of the sine fit, so its residual is at most the distance from it: `‖d‖ ≤ √N·b` |
| SNR | `≥ -20 log10(√2·b / a)` | The same for the harmonic fit's residual |
| THD, aliasing | `≤ 20 log10(√2·b·√(1 + ρ) / ((1 - ρ)·a))` | The fitted harmonics are the fit of the error alone: `‖S⁻¹Δc‖ ≤ √(1 + ρ)/(1 - ρ) · √N·b`, and `Σ P_h · N` is its square over the harmonics |
| SFDR | `≥ 20 log10(a / (2·b·√ENBW))` | Cauchy-Schwarz on `d` |
| Fundamental | `≥ a` | The same lemma on the fundamental's two unknowns, of weight 1/2 |

These hold for every such signal, whatever the shape of its error: `every_signal_within_the_error_meets_the_bounds` attacks them at three rates and lengths with errors of magnitude `ε`: square waves at the second harmonic, at the seventh (aliased at some of the rates) and at a frequency unrelated to the fundamental, alternating signs (Nyquist) and random values, and checks that every level stays within its bound. Only one check shows the bounds are not vacuous: the THD+N of the square wave at the second harmonic comes within 4 dB of its bound.

## Thresholds of `osc~`

The thresholds of `osc~` are the bounds above for `A = 1` and `ε` the error bound per sample its tests derive and prove against an exact reference, in each **render format** (`engine-design.md`, decision 10; `bound` in `crates/audio-boxes/src/osc_tests.rs`, `sineErrorBound` in the oracle, both read the vectors of `sine_error_bounds`, each with its `format`). Two terms are common to every format: the phase accumulated in `f64` in both precisions, `2π·N·2^-52` (in each regime of cycles per sample below 4), and evaluating the sine, `2^-48` (the angle `2π·phase` in binary64 and an ulp of `libm`'s sine). The third term is how the sine is computed and given:

| Format | Third term | Why | THD+N bound | SFDR bound |
|---|---|---|---|---|
| `binary64` | `2^-54` | the sine in binary64, half an ulp below 1 | about -204.8 dB | about 198.8 dB |
| `binary64-as-binary32` | `2^-25` | the binary64 sine rounded to binary32, half an ulp below 1 | about -147.5 dB | about 141.5 dB |
| `binary32` | `2^-22 + 2 · 2^-25 + 2^-33` | the angle, below 2π < 8, rounded to binary32 (half an ulp of [4, 8), which moves the sine by as much); binary32's `libm` sine within `2^-33` of the sine of that angle before it rounds (FreeBSD's kernels: `2^-34.1` for the cosine, `2^-37.5` relative for the sine, after a reduction by multiples of π/2 in binary64); its result rounded, half an ulp below 1; and the rounding of a gain after the sine, which the shared formula of a binary32 sine counts (`osc~` has none, and meets the bound with that much to spare) | about -127.5 dB | about 121.5 dB |

The bounds are for a full-scale sine over 8192 frames (`has_the_thresholds_of_each_render_format`); in binary64 the measurements' own floor, `2^-36` of the signal, dominates the bound. The osc~ tests (`measures_within_its_derived_thresholds_at_every_rate`) measure 8192 frames in every format at every rate of `matrix::sample_rates()`, at 0.0007, 0.0229, 0.2871 and 0.4993 cycle per sample (low, mid, near Nyquist; their binary32 frequencies are fractional), at a quarter and an eighth of the rate (periodic). One source: the error bound and its derivation (`osc_tests.rs`), the bounds' formulas (this section), and their shared vectors.

Measured values sit below the bounds, and the gap is explained: the bound takes every sample at its largest rounding error, half an ulp; a sine that is not periodic over few samples spreads its rounding uniformly over the ulp, whose mean square is `ulp² / 12`.

- `binary64-as-binary32`: most samples lie in the top octave of binary32 values, where the ulp is `2^-24`. The predicted THD+N is `10 log10(2 · Σ_k p_k · 2^(-2(k+23)) / 12)`, `p_k = (2/π)(asin 2^(1-k) - asin 2^-k)` the share of samples in `[2^-k, 2^(1-k))`: about -153.7 dB, 6.2 dB below the bound.
- `binary32`: the rounding of the angle `x`, uniform over `[0, 2π)`, adds `(1/2π) ∫ cos²(x) ulp(x)² / 12 dx` (octave by octave, the ulp `2^(k-23)` in `[2^k, 2^(k+1))`, with `∫ cos² = x/2 + sin(2x)/4`) to that of the value: about -140.9 dB, 13.4 dB below the bound. The angle dominates: binary32 holds the angle near 2π to `2^-21` only.
- `binary64`: the error of the sine, below its bound of about `1.14e-11` here, is below the floor of the measurements of a full-scale sine (`2^-35`, about `2.9e-11`) by a factor of about 2.5 only, so the levels cannot resolve it (`osc~`'s test `bounds_its_binary64_error_below_the_floor` computes both from the constants); the prediction is that every level is that of the floor alone (`sine_bounds` with no error), which each non-periodic and periodic case meets (measured, about -230 to -258 dB), and the binding check in binary64 is the exact comparison with the reference sine (`assert_exact`).

The osc~ tests check every non-periodic case of the two binary32 formats measures within a band of its prediction measured over their deterministic domain (8 rates, 4 frequencies, 8192 frames) and rounded outward to 0.05 dB: [-0.10, +0.10] dB given as binary32 (measured from -0.055 to +0.057), [-0.30, +0.15] dB in binary32 (measured from -0.251 to +0.124, biased low at 0.0007 cycle per sample, whose few periods leave the angles not uniform as the prediction assumes). A periodic case (1 kHz at 8 kHz, 8 samples per period) puts all its rounding into a few harmonics, which is why the THD and SFDR bounds hold the whole error budget.

The aliasing law: a sine below Nyquist has no aliased component above the floor of its output: its aliasing level never exceeds the THD bound, in every format, at every rate of the matrix and every frequency tested.

## The oracle

The oracle (`tooling/oracle`) measures every channel of every conformance render, in each of its three render formats, whose expectation is a sine strictly between DC and Nyquist once folded, rises above its error bound in that format (`sineErrorBound`) somewhere in the frames measured, and whose amplitude leaves the fundamental a margin above that bound for the conditioning `ρ` of its sine fit (`|a| > (bound + floor) · √(2(1+ρ))/(1−ρ)`, the condition of the bounds above, `floor` the largest floor of a render within that bound, below; `quality.ts`): a sine whose ideal values all stay within that bound of zero, such as one at 2^-126 Hz over a few thousand frames, cannot be told from silence by any level, and one without that margin, such as a binary32 sine just above an amplitude of 2^-126, whose bound nears its amplitude, has no level any render could be held to; the distance of both from the ideal sine is checked anyway. The harmonic fit has a conditioning of its own, worse than the sine fit's when the frames hold no whole number of cycles: when the amplitude leaves it no such margin, its levels (THD, aliasing, SNR) are reported as not measured, as for a harmonic fit refused, and the other levels are held to their bounds. Both margins are facts of the expectation (a fit's conditioning depends on its frequencies and frames, never on the samples), so the exact render of an expectation the oracle accepts is never failed, which the oracle's tests check at frequencies and frame counts that are not whole cycles: each threshold of both fits is found from the code and pinned to its bits, and the exact render is judged one binary32 step below it, at it and one step above, in the regime of its side, as are amplitudes between and beyond the thresholds. The floor of a render within `ε` of a sine of amplitude `A` is at most `2 · 2^-36 · ((1 + √2)·|A| + (1 + √(2m)·√(1+ρ)/(1−ρ))·ε)` (`renderFloor`): its peak is at most `|A| + ε`; its coefficients are those of the sine, `|c_cos| + |c_sin| ≤ √2·|A|`, plus the fit of the error, `‖S⁻¹Δc‖ ≤ √(1+ρ)/(1−ρ)·√N·ε` (above), so `Σ|Δc| ≤ √(2m)·√(1+ρ)/(1−ρ)·ε` over the `m ≤ 2·HARMONICS + 1` unknowns of a fit, each of weight at least 1/2 (`sᵢ ≤ √(2/N)`, then Cauchy-Schwarz); doubled for the computed coefficients' own error, which the floor bounds far below that. A test checks that the fits of exact renders stay below it. It measures the first `MAX_FRAMES` frames, held to the bounds above for the expectation's amplitude and `sineErrorBound` at that length and in that format, so the thresholds come from the expectation and its derivation alone. A level out of its bound, or a render it cannot measure, fails the case; its verdict prints the worst levels of the case's channels, per format. The browser renders are the same bits as the native ones in both precisions (the render hashes of `bindings/audio-wasm`'s end-to-end tests), so measuring the native export measures them too.

## Known-answer vectors

`measurement-vectors.json`, read by both test suites:

- `vectors`: signals `x[n] = Σ aₜ cos(2π·((fₜ·n) mod fs)/fs + φₜ) + s·(u[n] - 1/2)`, the noise term only when `noise` is given, `u[n]` the `n`-th output of SplitMix64 (Steele, Lea and Flood, 2014) from `seed`: `state += 0x9E3779B97F4A7C15`, `z = state`, `z = (z ^ (z >> 30)) · 0xBF58476D1CE4E5B9`, `z = (z ^ (z >> 27)) · 0x94D049BB133111EB`, `z ^= z >> 31`, `u = (z >> 11) · 2^-53` (all modulo 2^64). Their expected levels are closed forms. A level is a number (dB), `"floor"` (an exact answer of minus infinity: the level is at most `20 log10(8·floor / A)`, or at least its negation for SNR and SFDR, `8` covering `√2`, `2√ENBW` and `√(2K)`), or `"none"` (no such level). A harmonic fit that must be refused gives the error's name. A missing level is not checked.
- Tolerance of a numbered level: the floor's effect on both amplitudes, `20 log10((1 + floor/a_level)/(1 - floor/A))`; for the SFDR, also the scalloping loss of the vector's spectrum and two images of every other tone at the sidelobe level (-92 dB) relative to the spur.
- **With noise**: uniform noise of step `s` has the variance `σ² = s²/12`; the mean square of `N` samples has the standard deviation `s² / √(180 N)` (since `E u⁴ = s⁴/80`), so it lies within 10 of them of `σ²` with probability at least 99 % (Chebyshev); a fit of `K` unknowns removes `K σ² / N` of it on average, at most 100 times that with probability at least 99 % (Markov); both at once with probability at least 98 % (a union bound). The SNR and THD+N must lie in the interval this gives, and so must the expectation in the vector; the seed is fixed, so the test is deterministic, and the interval is its justification.
- `refusals`: signals each implementation must refuse, by the error's name.
- `sine_error_bounds`: `(format, amplitude, frames)` and the error bound of a sine box per sample in that render format: with `m = |amplitude|`, `e = ⌈log2 m⌉` and `acc = 2π·frames·2^-52 + 2^-48`, `2^(max(e, -968) - 54) + m·acc` in `binary64` (values below 2^-1022 flushed), `2^(max(e, -101) - 25) + m·acc` in `binary64-as-binary32` (values rounding below 2^-126 flushed at delivery, `engine-design.md` decision 11), `2^(max(e, -101) - 25) + m·(2^-22 + 2^-25 + 2^-33 + acc)` in `binary32` (the sine's own rounding to binary32, `m·2^-25`, comes before any gain, whose product is rounded again; values below 2^-126 flushed), for an amplitude exact in binary32 only (a gain that itself rounds to binary32 adds `2^(e - 25)` more, which the first box with such a gain derives and adds; every `binary32` vector, and every amplitude the oracle accepts, is exact in binary32); 0 for silence.
- `sine_bounds`: inputs of the bounds above and their expected values, computed independently.

## Images

The visual library's boxes are measured against the same bar (`spec/visual-design.md`). The square is in the exact tier: its CPU reference renderer computes every pixel in integers, so its measurements are bit equalities, with no tolerance to derive.

| Measurement | How | Threshold |
|---|---|---|
| Coverage | Every pixel of every image of a whole cycle, and far in time, against its coverage computed independently from `spec/visual-design.md` (sections 4 and 6) in 128-bit integers (`renders_every_pixel_as_its_exact_reference`, `crates/visual-boxes`) | Equal bits |
| Area | The values of an image sum to the square's area, wherever it is | Equal, in `f64` (exact below 2⁵³) |
| Motion | The left edge, read from the pixels, advances by `⌊max(W − s, 0) q b / (p a)⌋` subpixel units or one more each image, and is at the left edge in an image that falls on the start of a cycle (image 0, and every whole number of periods after it), over one cycle of 35/6 images (`crates/visual-boxes`); a square that does not move and a cycle shorter than one image are covered by the coverage row's exact references and by the property tests | Exact |

Perceptual measurements (aliasing of moving edges, flicker, banding once images are quantized) and the GPU renderer's distance to this reference come with the visual measurements of the test kit (#35 step 7) and the GPU (#34 step 5); `spec/limitations.md` lists what is not measured yet.

## How to add measurements for a new box or engine component

1. **Say what it must be.** Write the ideal signal it should produce for a test input (a sine, a filtered sine, a delayed copy, a ramp…), with its exact parameters.
2. **Derive the error bound per sample** against that ideal, term by term (output rounding, phase or coefficient precision, evaluation), with its preconditions per regime, next to the box's exact-reference test (as `bound` in `osc_tests.rs`); prove it with that test at every rate of the matrix and at the edges of every parameter.
3. **Derive each threshold** from that bound with the lemmas of "Bounds for a sine", or a new lemma added to this document with its proof and an attacking test. A level the box need not meet is not measured; a level with no derivation is not a threshold.
4. **Record it in one place**: the derivation stays next to the error bound in the box's tests, the formula in this document, and any number both languages need in `measurement-vectors.json`.
5. **Test it** in the box's crate across `matrix::sample_rates()` and the frequencies or parameters that matter (low, mid, near Nyquist, fractional, a periodic case): every measured level within its bound, and the measured values explained against the bound (a prediction within a stated margin, as for `osc~`).
6. **Add conformance cases** whose expectations name the ideal signal: the oracle measures every render with the same thresholds, natively and in the browser. If the box can output -0, one case whose render holds it, so that the browser cases prove its bits through Web Audio: no render of the corpus holds -0 until such a box comes.
7. **For an engine component** (a resampler, smoothing, the limiter, compensation delays, crossfades, event timing), the same steps with the measurements its decision in `engine-design.md` lists, landing with the step that builds it.
