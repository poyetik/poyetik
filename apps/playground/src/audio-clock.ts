/**
 * The one reader of an audio context's clock (`currentTime`; the page's ESLint
 * configuration refuses it anywhere else in its sources): a browser may report a clock that
 * is no time (non-finite), which every `AudioParam` refuses with a `TypeError`, so that
 * whoever schedules on the clock has to handle its absence, the type saying so.
 */

/** The time of the audio clock of `context`, in seconds, or undefined while the browser reports one that is no time. */
export function audioTime(context: BaseAudioContext): number | undefined {
  const now = context.currentTime;
  return Number.isFinite(now) ? now : undefined;
}

/**
 * The frame of an audio clock at `time` seconds and `rate` hertz: the nearest integer to
 * `time` × `rate`, half-way rounded up.
 *
 * A browser's clock counts the frames it has rendered, N, and reports their time: when it
 * reports N / rate rounded to the nearest binary64 (a relative error of at most 2⁻⁵³), the
 * product rounds again (2⁻⁵³ more), so that it lies within N × (2⁻⁵² + 2⁻¹⁰⁶) of N: less
 * than half a frame while N < 2⁵¹, about 186 years at 384 kHz, the most the engine plays
 * at. The nearest integer is then N exactly; past it, a frame or so off, never far. A
 * browser that coarsens its clock gives the frame of the time it reports: Firefox rounds it
 * to 1 ms once a render quantum of 128 frames lasts 1 ms or less (128 kHz and above), up to rate ×
 * 1 ms frames off (`spec/limitations.md`, "Images follow the audio clock, not the sound").
 */
export function frameAt(time: number, rate: number): bigint {
  return BigInt(Math.round(time * rate));
}
