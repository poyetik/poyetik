/** Reads an audio clock unchecked, plainly, optionally and by destructuring: three refusals. */
export function times(context: BaseAudioContext, maybe: BaseAudioContext | undefined): readonly unknown[] {
  const { currentTime } = context;
  return [context.currentTime, maybe?.currentTime, currentTime];
}

/** A fake's clock: it sets and reads its own field, which stays allowed. */
export class FakeClock {
  /** The time of the fake, in seconds. */
  currentTime = 1;

  /** Moves the clock on by `seconds`. */
  advance(seconds: number): number {
    this.currentTime += seconds;
    return this.currentTime;
  }
}

/** A test sets the clock of a fake: a write, allowed. */
export function stopped(): FakeClock {
  const fake = new FakeClock();
  fake.currentTime = Number.NaN;
  return fake;
}
