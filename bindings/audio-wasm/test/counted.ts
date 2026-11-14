// Counting the typed arrays code creates, views included: what a view made on the audio
// thread costs is invisible to a garbage collection count when the view is kept.

/** A `Float32Array` that counts how many are created, views included. */
export class Counted extends Float32Array {
  /** Arrays created since the last reset. */
  static created = 0;

  /** Creates the array, and counts it. */
  constructor(...args: ConstructorParameters<typeof Float32Array>) {
    super(...args);
    Counted.created += 1;
  }
}

/** Arrays created by `action`, with `Counted` standing in for `Float32Array` (stubbed by the caller). */
export function created(action: () => void): number {
  Counted.created = 0;
  action();
  return Counted.created;
}
