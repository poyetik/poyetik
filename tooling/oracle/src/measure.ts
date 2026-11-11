/**
 * Objective measurements of a signal, as `spec/measurements.md` defines them: its spectrum,
 * and for a known fundamental its THD+N, THD, aliasing level, SNR and SFDR, with the bounds
 * a sine within a known error per sample meets. Written again from the specification,
 * sharing no code with the engine's test bench (`engine_testkit::measure`); both read the
 * known-answer vectors of `spec/measurement-vectors.json`.
 */

/** The highest harmonic order analyzed: harmonics 2 to 10 count in the THD and the aliasing level. */
export const HARMONICS = 10;

/** The longest signal measured, in frames: every frame index stays below 2^26, which the exact phase needs. */
export const MAX_FRAMES = 2 ** 17;

/** The largest conditioning a fit accepts; above it, its components cannot be told apart. */
export const CONDITIONING_MAX = 0.5;

/** Iterations of the Jacobi solver, each contracting by the conditioning, at most 1/2. */
export const JACOBI_ITERATIONS = 64;

/** The relative floor of the measurements, 2^-36, derived in the specification ("Error of the measurements"). */
export const FLOOR = 2 ** -36;

/** The coefficients a0 to a3 of the four-term Blackman-Harris window (Harris, 1978). */
export const WINDOW = [0.35875, 0.48829, 0.14128, 0.01168] as const;

/** The spectrum has at least this many times more points than the signal has samples. */
export const PADDING = 4;

/** Why a signal cannot be measured: the names of `MeasureError` in Rust. */
export type MeasureErrorKind = "NoSamples" | "TooLong" | "NotFinite" | "Fundamental" | "TooShort" | "Unresolvable" | "NoFundamental";

/** A measurement refused: never a number for what cannot be measured. */
export class MeasureError extends Error {
  /** Why. */
  readonly kind: MeasureErrorKind;

  /**
   * A refusal of the kind `kind`.
   * @param kind Why.
   * @param detail What exactly, in words.
   */
  constructor(kind: MeasureErrorKind, detail: string) {
    super(detail);
    this.name = "MeasureError";
    this.kind = kind;
  }
}

/**
 * The value at `index` of `array`.
 * @throws {RangeError} When `index` is outside the array.
 */
export function at(array: ArrayLike<number>, index: number): number {
  const value = array[index];
  if (value === undefined) {
    throw new RangeError(`index ${String(index)} outside ${String(array.length)} values`);
  }
  return value;
}

/** A compensated sum: each addition's rounding error is kept exactly (TwoSum) and added at the end. */
class Sum {
  /** The rounded sum. */
  #high = 0;
  /** The sum of the rounding errors. */
  #low = 0;

  /**
   * Adds `term`.
   * @param term The term.
   */
  add(term: number): void {
    const total = this.#high + term;
    const back = total - this.#high;
    this.#low += this.#high - (total - back) + (term - back);
    this.#high = total;
  }

  /** The sum. */
  value(): number {
    return this.#high + this.#low;
  }
}

/** The compensated sum of `terms`. */
export function compensatedSum(terms: Iterable<number>): number {
  const sum = new Sum();
  for (const term of terms) {
    sum.add(term);
  }
  return sum.value();
}

/** `value` as two halves of at most 26 significant bits each, summing to it exactly (Veltkamp, with 2^27 + 1). */
export function split(value: number): [number, number] {
  const scaled = 134_217_729 * value;
  const high = scaled - (scaled - value);
  return [high, value - high];
}

/**
 * The phase of `frequency` hertz at `frame`, at `rate` hertz, in cycles, within 2^-51 of
 * `frequency · frame / rate` modulo 1 (as a value near [0, 1)): the product is computed
 * exactly as a rounded product and its error (Dekker, the frame below 2^26 left whole), the
 * rounded part reduced modulo the rate exactly, then the error added.
 */
export function cycles(frequency: number, frame: number, rate: number): number {
  const product = frequency * frame;
  const [high, low] = split(frequency);
  const error = high * frame - product + low * frame;
  return ((product % rate) + error) / rate;
}

/** Whether `multiple · fundamental` is a whole number of times `rate` (exact for a binary32 fundamental). */
export function wholeTurns(multiple: number, fundamental: number, rate: number): boolean {
  return (multiple * fundamental) % rate === 0;
}

/** One frequency of a fit: an order of the fundamental, folded into [0, rate / 2]. */
export interface Component {
  /** The lowest harmonic order landing on it: 0 for DC, 1 for the fundamental. */
  readonly order: number;
  /** Whether it lies at DC or Nyquist, where it has a single basis function. */
  readonly edge: boolean;
}

/**
 * The distinct folded frequencies of the orders 0 to `highest` of `fundamental`, each with
 * the lowest order landing on it: orders `k` and `j` land together when `(k - j)` or
 * `(k + j)` times the fundamental is a whole number of times the rate; order `k` lies at an
 * edge when `2k` times it is.
 */
export function components(fundamental: number, rate: number, highest: number): Component[] {
  const found: Component[] = [];
  for (let order = 0; order <= highest; order++) {
    if (found.every((component) => !wholeTurns(order - component.order, fundamental, rate) && !wholeTurns(order + component.order, fundamental, rate))) {
      found.push({ order, edge: wholeTurns(2 * order, fundamental, rate) });
    }
  }
  return found;
}

/** One unknown of a fit: a basis function of a component, with its mean square over a period. */
export interface Unknown extends Component {
  /** 1 at an edge (`1` or `(-1)^n`), 1/2 for a cosine or a sine. */
  readonly weight: number;
}

/** The unknowns of `found`: one at an edge, a cosine and a sine elsewhere. */
export function unknowns(found: readonly Component[]): Unknown[] {
  return found.flatMap((component) => (component.edge ? [{ ...component, weight: 1 }] : [0, 1].map(() => ({ ...component, weight: 0.5 }))));
}

/**
 * The basis values at phase `phase` (cycles of the fundamental), one per unknown of `found`
 * (`size` of them): `cos 2π·kφ`, and `sin 2π·kφ` off the edges, `kφ` reduced first.
 */
export function basis(found: readonly Component[], phase: number, size: number): Float64Array {
  const row = new Float64Array(size);
  let index = 0;
  for (const component of found) {
    const turns = component.order * phase;
    const angle = 2 * Math.PI * (turns - Math.floor(turns));
    row[index] = Math.cos(angle);
    index += 1;
    if (!component.edge) {
      row[index] = Math.sin(angle);
      index += 1;
    }
  }
  return row;
}

/** The quality of a fit. */
export interface Quality {
  /** In [0, CONDITIONING_MAX]: 0 for components orthogonal over the signal. */
  readonly conditioning: number;
  /** `FLOOR · (peak + Σ|c|)`: every amplitude and root mean square it gives is within it of its exact value. */
  readonly floor: number;
}

/**
 * The scaled normal matrix less the identity (`gram` holds `Σ φᵢφⱼ`, `scales` the inverse
 * nominal norms), and its conditioning: the largest sum of the absolute values of a row.
 */
function deviationOf(gram: readonly (readonly number[])[], scales: readonly number[]): { deviation: number[][]; conditioning: number } {
  const deviation = gram.map((row, i) => row.map((value, j) => value * at(scales, i) * at(scales, j) - (i === j ? 1 : 0)));
  return { deviation, conditioning: Math.max(...deviation.map((row) => row.reduce((total, value) => total + Math.abs(value), 0))) };
}

/** The `Unresolvable` refusal of a fit whose conditioning exceeds {@link CONDITIONING_MAX}, or null. */
function unresolvable(conditioning: number): MeasureError | null {
  return conditioning > CONDITIONING_MAX
    ? new MeasureError("Unresolvable", `the components are too close to be told apart (conditioning ${String(conditioning)}, at most ${String(CONDITIONING_MAX)})`)
    : null;
}

/**
 * Solves the scaled normal equations: `gram` holds `Σ φᵢφⱼ` row by row, `projections`
 * `Σ φᵢx`, `scales` the inverse nominal norms. Returns the coefficients and the conditioning.
 * @throws {MeasureError} `Unresolvable` when the conditioning exceeds {@link CONDITIONING_MAX}.
 */
export function solve(gram: readonly (readonly number[])[], projections: readonly number[], scales: readonly number[]): { coefficients: number[]; conditioning: number } {
  const { deviation, conditioning } = deviationOf(gram, scales);
  const refusal = unresolvable(conditioning);
  if (refusal !== null) {
    throw refusal;
  }
  const right = projections.map((value, i) => value * at(scales, i));
  const solution = Array.from({ length: JACOBI_ITERATIONS }).reduce<number[]>(
    (previous) => deviation.map((row, i) => at(right, i) - row.reduce((total, value, j) => total + value * at(previous, j), 0)),
    right,
  );
  return { coefficients: solution.map((value, i) => value * at(scales, i)), conditioning };
}

/**
 * Why the sine fit of `fundamental` over `frames` frames at `rate` would be refused, as
 * {@link measure} refuses it (`TooShort` or `Unresolvable`), or null when it would not:
 * neither depends on the samples, only on the frequencies fitted and the frames.
 */
export function sineFitRefusal(fundamental: number, rate: number, frames: number): MeasureError | null {
  const conditioning = sineFitConditioning(fundamental, rate, frames);
  return conditioning instanceof MeasureError ? conditioning : null;
}

/**
 * The conditioning the sine fit of `fundamental` over `frames` frames at `rate` has when
 * {@link measure} measures it, computed as it computes it, or why it would refuse the fit
 * (`TooShort` or `Unresolvable`): neither depends on the samples.
 */
export function sineFitConditioning(fundamental: number, rate: number, frames: number): number | MeasureError {
  const found = components(fundamental, rate, 1);
  const list = unknowns(found);
  const size = list.length;
  if (frames < size) {
    return new MeasureError("TooShort", `the signal has ${String(frames)} frames, fewer than the ${String(size)} unknowns of the fit`);
  }
  const gram = list.map(() => list.map(() => new Sum()));
  for (let frame = 0; frame < frames; frame++) {
    const row = basis(found, cycles(fundamental, frame, rate), size);
    gram.forEach((sums, i) => {
      sums.forEach((sum, j) => {
        sum.add(at(row, i) * at(row, j));
      });
    });
  }
  const scales = list.map((unknown) => 1 / Math.sqrt(unknown.weight * frames));
  const { conditioning } = deviationOf(
    gram.map((sums) => sums.map((sum) => sum.value())),
    scales,
  );
  return unresolvable(conditioning) ?? conditioning;
}

/** What a fit found. */
interface Fit {
  /** The unknowns, each with its coefficient. */
  readonly fitted: readonly (readonly [Unknown, number])[];
  /** The signal less the fitted components. */
  readonly residual: Float64Array;
  /** The mean square of the residual. */
  readonly residualPower: number;
  /** Its conditioning and floor. */
  readonly quality: Quality;
}

/**
 * Fits the orders 0 to `highest` of `fundamental` to `samples` by least squares
 * (`spec/measurements.md`, "Fits").
 * @throws {MeasureError} `TooShort` or `Unresolvable`.
 */
function fit(samples: Float64Array, rate: number, fundamental: number, highest: number): Fit {
  const found = components(fundamental, rate, highest);
  const list = unknowns(found);
  const size = list.length;
  const frames = samples.length;
  if (frames < size) {
    throw new MeasureError("TooShort", `the signal has ${String(frames)} frames, fewer than the ${String(size)} unknowns of the fit`);
  }
  const gram = list.map(() => list.map(() => new Sum()));
  const projections = list.map(() => new Sum());
  samples.forEach((sample, frame) => {
    const row = basis(found, cycles(fundamental, frame, rate), size);
    gram.forEach((sums, i) => {
      const left = at(row, i);
      sums.forEach((sum, j) => {
        sum.add(left * at(row, j));
      });
    });
    projections.forEach((sum, i) => {
      sum.add(at(row, i) * sample);
    });
  });
  const scales = list.map((unknown) => 1 / Math.sqrt(unknown.weight * frames));
  const { coefficients, conditioning } = solve(
    gram.map((sums) => sums.map((sum) => sum.value())),
    projections.map((sum) => sum.value()),
    scales,
  );
  const squares = new Sum();
  const residual = samples.map((sample, frame) => {
    const model = basis(found, cycles(fundamental, frame, rate), size).reduce((modelled, value, j) => modelled + value * at(coefficients, j), 0);
    const left = sample - model;
    squares.add(left * left);
    return left;
  });
  const peak = samples.reduce((largest, sample) => Math.max(largest, Math.abs(sample)), 0);
  const total = coefficients.reduce((sum, coefficient) => sum + Math.abs(coefficient), 0);
  return {
    fitted: list.map((unknown, i) => [unknown, at(coefficients, i)] as const),
    residual,
    residualPower: squares.value() / frames,
    quality: { conditioning, floor: FLOOR * (peak + total) },
  };
}

/** The power of the fundamental, a component of every fit (order 1): `weight · c²` over its unknowns. */
function fundamentalPower(fitted: Fit): number {
  return fitted.fitted.reduce((total, [unknown, coefficient]) => total + (unknown.order === 1 ? unknown.weight * coefficient * coefficient : 0), 0);
}

/** The total power of the fitted unknowns `selected` keeps (`weight · c²` each), or null when it keeps none. */
function power(fitted: Fit, selected: (unknown: Unknown) => boolean): number | null {
  const kept = fitted.fitted.filter(([unknown]) => selected(unknown));
  return kept.length === 0 ? null : kept.reduce((total, [unknown, coefficient]) => total + unknown.weight * coefficient * coefficient, 0);
}

/** `10 log10(ratio)`. */
function decibels(ratio: number): number {
  return 10 * Math.log10(ratio);
}

/**
 * The length of `samples`, once checked: at least one sample, at most {@link MAX_FRAMES},
 * every one finite.
 * @throws {MeasureError} `NoSamples`, `TooLong` or `NotFinite`.
 */
function checked(samples: Float64Array): number {
  if (samples.length === 0) {
    throw new MeasureError("NoSamples", "the signal has no sample");
  }
  if (samples.length > MAX_FRAMES) {
    throw new MeasureError("TooLong", `the signal has ${String(samples.length)} frames, more than the ${String(MAX_FRAMES)} measured`);
  }
  const index = samples.findIndex((sample) => !Number.isFinite(sample));
  if (index >= 0) {
    throw new MeasureError("NotFinite", `sample ${String(index)} is not finite`);
  }
  return samples.length;
}

/** An amplitude spectrum (`spec/measurements.md`, "Spectrum"). */
export interface Spectrum {
  /** The amplitude at each point from 0 (DC) to `points / 2` (Nyquist), in the unit of the signal. */
  readonly amplitudes: Float64Array;
  /** The number of points of the transform: the smallest power of two at least {@link PADDING} times the signal's length. */
  readonly points: number;
  /** The window's equivalent noise bandwidth, in bins of the signal: `N Σw² / (Σw)²`. */
  readonly enbw: number;
  /** The largest relative loss of a sine's amplitude between two points: `π² m₂ / (2 M²)`. */
  readonly scalloping: number;
}

/** The four-term Blackman-Harris window over `frames` samples, periodic. */
export function window(frames: number): Float64Array {
  const [a0, a1, a2, a3] = WINDOW;
  return Float64Array.from({ length: frames }, (_empty, index) => {
    const angle = (2 * Math.PI * index) / frames;
    return a0 - a1 * Math.cos(angle) + a2 * Math.cos(2 * angle) - a3 * Math.cos(3 * angle);
  });
}

/**
 * The discrete Fourier transform `X[k] = Σ x[n] e^(-2πikn/M)` of `re + i·im`, whose length
 * is a power of two: a Stockham radix-2 FFT (span `n` from M down to 2, stride `M / n`: rows
 * `p` and `p + n/2` give rows `2p`, their sum, and `2p + 1`, their difference times
 * `e^(-2πip/n)`), twiddle factors from exact angles.
 */
export function fft(re: Float64Array<ArrayBuffer>, im: Float64Array<ArrayBuffer>): [Float64Array<ArrayBuffer>, Float64Array<ArrayBuffer>] {
  const points = re.length;
  const half = points / 2;
  const angles = Float64Array.from({ length: half }, (_empty, index) => (-2 * Math.PI * index) / points);
  const cosines = angles.map((angle) => Math.cos(angle));
  const sines = angles.map((angle) => Math.sin(angle));
  let [inRe, inIm] = [re, im];
  let [outRe, outIm] = [new Float64Array(points), new Float64Array(points)];
  for (let stride = 1; stride < points; stride *= 2) {
    for (let row = 0; row * stride < half; row++) {
      const [wRe, wIm] = [at(cosines, row * stride), at(sines, row * stride)];
      for (let column = 0; column < stride; column++) {
        const top = row * stride + column;
        const [aRe, aIm, bRe, bIm] = [at(inRe, top), at(inIm, top), at(inRe, top + half), at(inIm, top + half)];
        const even = 2 * row * stride + column;
        outRe[even] = aRe + bRe;
        outIm[even] = aIm + bIm;
        const [dRe, dIm] = [aRe - bRe, aIm - bIm];
        outRe[even + stride] = dRe * wRe - dIm * wIm;
        outIm[even + stride] = dRe * wIm + dIm * wRe;
      }
    }
    [inRe, inIm, outRe, outIm] = [outRe, outIm, inRe, inIm];
  }
  return [inRe, inIm];
}

/**
 * The amplitude spectrum of `samples` (`spec/measurements.md`, "Spectrum").
 * @throws {MeasureError} `NoSamples`, `TooLong` or `NotFinite`.
 */
export function spectrum(samples: Float64Array): Spectrum {
  const frames = checked(samples);
  let points = PADDING;
  while (points < PADDING * frames) {
    points *= 2;
  }
  const weights = window(frames);
  const sum = weights.reduce((total, weight) => total + weight, 0);
  const squares = weights.reduce((total, weight) => total + weight * weight, 0);
  const moment = weights.reduce((total, weight, index) => total + weight * (index - frames / 2) * (index - frames / 2), 0);
  const re = new Float64Array(points);
  re.set(samples.map((sample, index) => sample * at(weights, index)));
  const [outRe, outIm] = fft(re, new Float64Array(points));
  const amplitudes = Float64Array.from({ length: points / 2 + 1 }, (_empty, point) => {
    const gain = point === 0 || point === points / 2 ? 1 : 2;
    return (gain * Math.sqrt(at(outRe, point) ** 2 + at(outIm, point) ** 2)) / sum;
  });
  return {
    amplitudes,
    points,
    enbw: (frames * squares) / (sum * sum),
    scalloping: ((Math.PI * Math.PI) / 2) * (moment / sum) / (points * points),
  };
}

/** The quality of a fit and, where it was not refused, the levels of the harmonic fit. */
export interface Harmonics {
  /** THD, dB, or null when every harmonic lands on DC or the fundamental. */
  readonly thd: number | null;
  /** The aliasing level, dB, or null when no harmonic lies above Nyquist apart from those. */
  readonly aliasing: number | null;
  /** SNR, dB. */
  readonly snr: number;
  /** The harmonic fit's quality. */
  readonly quality: Quality;
}

/** The measurements of a signal with a known fundamental, levels in dB relative to it. */
export interface Measurements {
  /** The fundamental's amplitude, from the sine fit. */
  readonly fundamental: number;
  /** THD+N: the residual of the sine fit relative to the fundamental. */
  readonly thdPlusN: number;
  /** SFDR: the fundamental relative to the strongest point of the spectrum of that residual. */
  readonly sfdr: number;
  /** The sine fit's quality. */
  readonly sine: Quality;
  /** The window's equivalent noise bandwidth over the signal, in bins. */
  readonly enbw: number;
  /** The harmonic fit's levels, or why it was refused. */
  readonly harmonics: Harmonics | MeasureError;
}

/**
 * The harmonic fit's levels.
 * @throws {MeasureError} `TooShort` or `Unresolvable`.
 */
function harmonicLevels(samples: Float64Array, rate: number, fundamental: number): Harmonics {
  const fitted = fit(samples, rate, fundamental, HARMONICS);
  const reference = fundamentalPower(fitted);
  const relative = (part: number | null): number | null => (part === null ? null : decibels(part / reference));
  return {
    thd: relative(power(fitted, (unknown) => unknown.order > 1)),
    // Neither DC nor the fundamental lies above Nyquist: only harmonics can.
    aliasing: relative(power(fitted, (unknown) => unknown.order * fundamental > rate / 2)),
    snr: decibels(reference / fitted.residualPower),
    quality: fitted.quality,
  };
}

/**
 * Measures `samples`, at `rate` hertz, whose fundamental is `fundamental` hertz.
 * @throws {MeasureError} `NoSamples`, `TooLong`, `NotFinite`, `Fundamental`, and the sine
 * fit's `TooShort`, `Unresolvable` or `NoFundamental`; the harmonic fit's refusals are
 * returned in `harmonics`.
 */
export function measure(samples: Float64Array, rate: number, fundamental: number): Measurements {
  checked(samples);
  if (!(fundamental > 0 && fundamental < rate / 2)) {
    throw new MeasureError("Fundamental", `the fundamental, ${String(fundamental)} Hz, is not strictly between 0 and Nyquist`);
  }
  const sine = fit(samples, rate, fundamental, 1);
  const sinePower = fundamentalPower(sine);
  const amplitude = Math.sqrt(2 * sinePower);
  if (!(amplitude > sine.quality.floor)) {
    throw new MeasureError("NoFundamental", `the fundamental, of amplitude ${String(amplitude)}, is not above the floor of the measurement`);
  }
  const spurs = spectrum(sine.residual);
  let harmonics: Harmonics | MeasureError;
  try {
    harmonics = harmonicLevels(samples, rate, fundamental);
  } catch (error) {
    if (!(error instanceof MeasureError)) {
      throw error;
    }
    harmonics = error;
  }
  return {
    fundamental: amplitude,
    thdPlusN: decibels(sine.residualPower / sinePower),
    sfdr: 2 * decibels(amplitude / spurs.amplitudes.reduce((largest, value) => Math.max(largest, value), 0)),
    sine: sine.quality,
    enbw: spurs.enbw,
    harmonics,
  };
}

/** The worst levels of a signal within a known error per sample of an ideal sine (`spec/measurements.md`, "Bounds for a sine"). */
export interface Bounds {
  /** The highest THD+N, dB. */
  readonly thdPlusN: number;
  /** The lowest SFDR, dB. */
  readonly sfdr: number;
  /** The highest THD and aliasing level, and the lowest SNR, dB; null without a harmonic fit, or without a margin for it. */
  readonly harmonics: { readonly thd: number; readonly snr: number } | null;
}

/** `[b, a]`: the error per sample with the floor, and the lowest the fundamental can measure; null when that is not above zero. */
function margins(amplitude: number, error: number, quality: Quality): [number, number] | null {
  const rho = quality.conditioning;
  const total = error + quality.floor;
  const lowest = Math.abs(amplitude) - (total * Math.sqrt(2 * (1 + rho))) / (1 - rho);
  return lowest > 0 ? [total, lowest] : null;
}

/** The most unknowns a fit has: DC, then a cosine and a sine for each order up to {@link HARMONICS}. */
const MOST_UNKNOWNS = 2 * HARMONICS + 1;

/**
 * The largest floor a fit of conditioning `conditioning` has over a render within `error`
 * per sample of a sine of `amplitude` (`spec/measurements.md`, "The oracle"): its floor is
 * `FLOOR · (peak + Σ|c|)`, the peak at most `|amplitude| + error`, the coefficients those of
 * the sine (`|c_cos| + |c_sin| ≤ √2·|amplitude|`) plus the fit of the error, whose scaled
 * coefficients have a norm of at most `√(1 + ρ)/(1 - ρ) · √N · error`, so at most
 * `√(2m) · √(1 + ρ)/(1 - ρ) · error` in sum over its `m` unknowns, each of weight at least
 * 1/2; doubled for the computed coefficients' own error, which the floor itself bounds far
 * below that.
 */
export function renderFloor(amplitude: number, error: number, conditioning: number): number {
  const fitted = (Math.sqrt(2 * MOST_UNKNOWNS) * Math.sqrt(1 + conditioning)) / (1 - conditioning);
  return 2 * FLOOR * ((1 + Math.SQRT2) * Math.abs(amplitude) + (1 + fitted) * error);
}

/**
 * Whether a fit of conditioning `conditioning` leaves the fundamental of every render within
 * `error` per sample of a sine of `amplitude` a margin above that error, its own floor
 * included ({@link renderFloor}): the condition {@link sineBounds} needs to bound the levels
 * of that fit for any such render, the exact one first. It depends on the expectation
 * alone, never on the samples judged.
 */
export function hasMargin(amplitude: number, error: number, conditioning: number): boolean {
  return margins(amplitude, error, { conditioning, floor: renderFloor(amplitude, error, conditioning) }) !== null;
}

/** The bounds of the harmonic fit of quality `quality` (see {@link Bounds}); null without a margin. */
function harmonicBounds(amplitude: number, error: number, quality: Quality): Bounds["harmonics"] {
  const found = margins(amplitude, error, quality);
  if (found === null) {
    return null;
  }
  const [total, lowest] = found;
  const rho = quality.conditioning;
  return {
    thd: 20 * Math.log10((Math.SQRT2 * total * Math.sqrt(1 + rho)) / ((1 - rho) * lowest)),
    snr: -20 * Math.log10((Math.SQRT2 * total) / lowest),
  };
}

/** The {@link Bounds} of a signal within `error` per sample of an ideal sine of `amplitude`, for the fits and window of `measured`; null when there is no margin. */
export function sineBounds(amplitude: number, error: number, measured: Pick<Measurements, "sine" | "enbw" | "harmonics">): Bounds | null {
  const sine = margins(amplitude, error, measured.sine);
  if (sine === null) {
    return null;
  }
  const [total, lowest] = sine;
  return {
    thdPlusN: 20 * Math.log10((Math.SQRT2 * total) / lowest),
    sfdr: 20 * Math.log10(lowest / (2 * total * Math.sqrt(measured.enbw))),
    harmonics: measured.harmonics instanceof MeasureError ? null : harmonicBounds(amplitude, error, measured.harmonics.quality),
  };
}
