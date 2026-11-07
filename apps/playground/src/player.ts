/**
 * Plays a patch with the engine in an `AudioContext`, and keeps the latest samples it
 * outputs for the oscilloscope.
 */

import { addProcessors, createEngineNode, loadPatch, tap, type Tap } from "@poyetik/engine-wasm/web";
import { SampleHistory } from "./history.ts";

/** The latest output of the engine: samples of its first channel, and their rate. */
export interface Signal {
  readonly samples: Float32Array;
  /** Sample rate, in hertz. */
  readonly rate: number;
}

/** Something that plays sound and shows what it plays. */
export interface Player {
  /** Starts playing. Call it from a user gesture: browsers only start audio then. */
  start(): Promise<void>;
  /** Stops playing and releases the audio device. */
  stop(): Promise<void>;
  /** What was played most recently, or undefined when not playing. */
  latest(): Signal | undefined;
}

/** How the engine player is set up. */
export interface PlayerOptions {
  /** URL of the engine WebAssembly module. */
  readonly wasm: string;
  /** URL of the module of the worklet processors. */
  readonly worklet: string;
  /** The patch, as UTF-8 text in the patch format. */
  readonly patch: Uint8Array;
  /** Gain applied after the engine, before the speakers (0 is silent). The oscilloscope shows the engine output, before it. */
  readonly gain: number;
  /** Number of samples kept for the oscilloscope. */
  readonly history: number;
  /** Receives errors happening while playing, outside any call. */
  readonly onError: (error: Error) => void;
}

/** A running session: one audio context and its tap. */
interface Session {
  readonly context: AudioContext;
  readonly tap: Tap;
  readonly history: SampleHistory;
}

/** Plays a patch with the engine. */
export class EnginePlayer implements Player {
  /** The setup. */
  readonly #options: PlayerOptions;
  /** The engine module, fetched once. */
  #wasm: ArrayBuffer | undefined;
  /** The running session, if playing. */
  #session: Session | undefined;

  /** Sets the player up; nothing is loaded before the first start. */
  constructor(options: PlayerOptions) {
    this.#options = options;
  }

  /** Fetches the engine module, once. */
  async #module(): Promise<ArrayBuffer> {
    if (this.#wasm === undefined) {
      const response = await fetch(this.#options.wasm);
      if (!response.ok) {
        throw new Error(`cannot load the engine: HTTP ${String(response.status)}`);
      }
      this.#wasm = await response.arrayBuffer();
    }
    return this.#wasm;
  }

  /** Starts playing; does nothing when already playing. */
  async start(): Promise<void> {
    if (this.#session !== undefined) {
      return;
    }
    // Created before any await, while the user gesture is still active (required by Safari).
    const context = new AudioContext();
    try {
      const loaded = await loadPatch(await this.#module(), this.#options.patch, context.sampleRate);
      await addProcessors(context, this.#options.worklet);
      const node = createEngineNode(context, loaded);
      const history = new SampleHistory(this.#options.history);
      const running = tap(
        context,
        node,
        1,
        (block) => {
          const [first] = block;
          if (first !== undefined) {
            history.push(first);
          }
        },
        this.#options.onError,
      );
      node.connect(new GainNode(context, { gain: this.#options.gain })).connect(context.destination);
      await context.resume();
      this.#session = { context, tap: running, history };
    } catch (error) {
      await context.close();
      throw error;
    }
  }

  /** Stops playing; does nothing when not playing. */
  async stop(): Promise<void> {
    const session = this.#session;
    if (session === undefined) {
      return;
    }
    this.#session = undefined;
    session.tap.stop();
    await session.context.close();
  }

  /** What was played most recently, or undefined when not playing. */
  latest(): Signal | undefined {
    const session = this.#session;
    return session === undefined ? undefined : { samples: session.history.snapshot(), rate: session.context.sampleRate };
  }
}
