/** A package's checked reader of an audio clock, the one file allowed to read `currentTime`. */
export function audioTime(context: BaseAudioContext): number | undefined {
  const now = context.currentTime;
  return Number.isFinite(now) ? now : undefined;
}
