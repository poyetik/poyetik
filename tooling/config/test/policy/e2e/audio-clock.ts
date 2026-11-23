import { setTimeout as wait } from "node:timers/promises";

/** A reviewed timed wait (TIMED_WAITS): readings of an audio clock spaced by the page's timers; `fetch` stays refused. */
export async function clockSamples(context: BaseAudioContext, apart: number): Promise<readonly number[]> {
  const times: number[] = [];
  for (let tried = 0; tried < 4; tried += 1) {
    times.push(context.currentTime);
    await new Promise((resolve) => {
      setTimeout(resolve, apart);
    });
  }
  await fetch("/times");
  return times;
}

/** The timer of Node's modules, which a timed wait may import. */
export const WAIT: unknown = wait;
