// The timing logic of the bench page (harness.ts), which runs in a browser: what it renders,
// how many runs it times, and how, measured and mutated here while harness.ts only wires
// it to the page. The one place the bench page reads a clock (spec/benchmarks.md: wall
// times are asserted only as upper bounds, in the end-to-end tests).

/** A clock in milliseconds. */
export type Clock = () => number;

/** The page's clock: the only clock a test of the repository reads (CLAUDE.md, "Tests are deterministic"). */
export const PAGE_CLOCK: Clock = () => performance.now();

/** Frames per render quantum of Web Audio: what the engine renders at each call. */
const QUANTUM_FRAMES = 128;

/**
 * The number of runs `?runs=` asks in `search` (the page's `location.search`): 1 when absent.
 * @throws {RangeError} when it is not a positive whole number written in decimal.
 */
export function runsOf(search: string): number {
  const written = new URLSearchParams(search).get("runs");
  if (written === null) {
    return 1;
  }
  const runs = Number(written);
  if (!/^[1-9][0-9]*$/.test(written) || !Number.isSafeInteger(runs)) {
    throw new RangeError(`runs=${written} is not a positive whole number`);
  }
  return runs;
}

/** What {@link renderBlocks} needs of an engine: its channel count, its engine block, and rendering into buffers. */
export interface Renders {
  /** Its output channels. */
  readonly channels: number;
  /** The frames of each of its blocks, as its module gives them. */
  readonly blockSize: number;
  /** Renders the next frames into `outputs`, one per channel. */
  render(outputs: Float32Array[]): void;
}

/**
 * Renders `blocks` blocks of `engine`, of its own engine block, in render quanta of
 * {@link QUANTUM_FRAMES} frames, as Web Audio asks them, into one output per channel and at
 * least one. The engine renders a block when its first frame is output, so the frames of
 * `blocks` blocks render exactly `blocks` blocks.
 * @throws {RangeError} when the frames of `blocks` blocks are not a whole number of quanta.
 */
export function renderBlocks(engine: Renders, blocks: number): void {
  const quanta = (blocks * engine.blockSize) / QUANTUM_FRAMES;
  if (!Number.isInteger(quanta) || quanta < 0) {
    throw new RangeError(`${String(blocks)} blocks of ${String(engine.blockSize)} frames are not a whole number of ${String(QUANTUM_FRAMES)}-frame quanta`);
  }
  const outputs = Array.from({ length: Math.max(1, engine.channels) }, () => new Float32Array(QUANTUM_FRAMES));
  for (let quantum = 0; quantum < quanta; quantum += 1) {
    engine.render(outputs);
  }
}

/** The time `action` takes by `clock`, in milliseconds. */
export function timed(clock: Clock, action: () => void): number {
  const start = clock();
  action();
  return clock() - start;
}

/**
 * Runs `once` one time as a warm-up (compiling the code), not counted, then `runs` times:
 * the times it returned.
 */
export function timeRuns(runs: number, once: () => number): number[] {
  once();
  return Array.from({ length: runs }, once);
}

/**
 * Times `runs` awaited runs of `action`, one after the other, each from its start to the
 * end of the promise it returns, by `clock`; `after` then receives its result, untimed.
 * A run that rejects rejects the whole.
 */
export async function timeEach<T>(runs: number, clock: Clock, action: () => Promise<T>, after: (result: T) => void): Promise<number[]> {
  const times: number[] = [];
  for (let run = 0; run < runs; run += 1) {
    const start = clock();
    const result = await action();
    times.push(clock() - start);
    after(result);
  }
  return times;
}
