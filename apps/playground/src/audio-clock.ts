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
