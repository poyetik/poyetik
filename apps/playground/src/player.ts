/**
 * Plays a patch with the engine in an `AudioContext`, and keeps the latest samples it
 * outputs for the oscilloscope.
 *
 * The player is a state machine: every state it can reach is visible through
 * {@link Player.state}, including the ones the browser or the audio thread cause
 * (suspended, failed), so that the page never claims to play while it does not.
 */

import { addProcessors, createEngineNode, loadPatch, tap, type Tap } from "@poyetik/engine-wasm/web";
import { SampleHistory } from "./history.ts";

/** The latest output of the engine: samples of its first channel, and their rate. */
export interface Signal {
  readonly samples: Float32Array;
  /** Sample rate, in hertz. */
  readonly rate: number;
}

/** Where the player is. */
export type PlayerState =
  /** Not playing; can start. */
  | { readonly kind: "stopped" }
  /** Loading the engine and the patch. */
  | { readonly kind: "starting" }
  /** Playing. */
  | { readonly kind: "playing" }
  /** The browser suspended the audio (another tab, a phone call, no user gesture): can resume. */
  | { readonly kind: "suspended" }
  /** Fading out and releasing the audio device. */
  | { readonly kind: "stopping" }
  /** Could not start, or stopped by a failure: `reason` says why. Can start again. */
  | { readonly kind: "failed"; readonly reason: string };

/** Something that plays sound and shows what it plays. */
export interface Player {
  /**
   * Starts playing, from "stopped" or "failed"; while starting, returns the start in
   * progress. Call it from a user gesture: browsers only start audio then. Never rejects:
   * a failure is a state.
   */
  start(): Promise<void>;
  /** Stops playing; during a start, waits for it, then stops. From "failed", acknowledges it. */
  stop(): Promise<void>;
  /** Resumes a suspended player. Call it from a user gesture. */
  resume(): Promise<void>;
  /** The current state. The same object until the state changes. */
  state(): PlayerState;
  /** Calls `listener` after every change of state; returns how to unsubscribe. */
  subscribe(listener: () => void): () => void;
  /** What was played most recently, or undefined when not playing. */
  latest(): Signal | undefined;
}

/** What a browser may lack to play a patch. */
export type Missing = "secure-context" | "web-audio" | "audio-worklet" | "webassembly";

/** The parts of the browser the player needs: `globalThis` in a page. */
export interface Environment {
  readonly isSecureContext: boolean;
  readonly AudioContext?: unknown;
  readonly AudioWorkletNode?: unknown;
  readonly WebAssembly?: unknown;
}

/** What `environment` lacks to play, or undefined when it has everything. */
export function missing(environment: Environment): Missing | undefined {
  if (!environment.isSecureContext) {
    return "secure-context";
  }
  if (typeof environment.AudioContext !== "function") {
    return "web-audio";
  }
  if (typeof environment.AudioWorkletNode !== "function") {
    return "audio-worklet";
  }
  if (typeof environment.WebAssembly !== "object") {
    return "webassembly";
  }
  return undefined;
}

/** Explains to the user what is missing, and what to do. */
export function explain(lacking: Missing): string {
  switch (lacking) {
    case "secure-context":
      return "Audio needs a secure page: open this page over HTTPS (or from localhost).";
    case "web-audio":
      return "This browser cannot play audio from a page (no Web Audio). Try a recent Chrome, Firefox, Safari or Edge.";
    case "audio-worklet":
      return "This browser cannot run the engine in real time (no AudioWorklet). Try a recent Chrome, Firefox, Safari or Edge.";
    case "webassembly":
      return "This browser cannot run the engine (no WebAssembly). Try a recent Chrome, Firefox, Safari or Edge.";
  }
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
  /** Duration of the fade in and out, in seconds. */
  readonly fade: number;
  /** Number of samples kept for the oscilloscope. */
  readonly history: number;
  /** The browser, checked before starting. */
  readonly environment: Environment;
}

/** One audio context, from start to close. */
class Session {
  readonly context: AudioContext;
  readonly history: SampleHistory;
  /** The output stage, silent until the fade in: fades and gain, before the speakers. */
  readonly gain: GainNode;
  /** The tap feeding the history, once connected. */
  #tap: Tap | undefined;

  /**
   * Opens a session on a new context.
   * @param history Number of samples kept for the oscilloscope.
   */
  constructor(history: number) {
    this.context = new AudioContext();
    this.history = new SampleHistory(history);
    this.gain = new GainNode(this.context, { gain: 0 });
  }

  /** Records the tap, once connected. */
  tapped(running: Tap): void {
    this.#tap = running;
  }

  /**
   * Stops the tap and releases the context. Called once per session: only by whoever
   * detaches it from the player first (a stop or a failure). A context the browser
   * already closed is not closed again (browsers reject that).
   */
  async close(): Promise<void> {
    this.#tap?.stop();
    if (this.context.state !== "closed") {
      await this.context.close();
    }
  }
}

/** The states, as shared objects: `state()` returns the same object until a change. */
const STOPPED: PlayerState = { kind: "stopped" };
const STARTING: PlayerState = { kind: "starting" };
const PLAYING: PlayerState = { kind: "playing" };
const SUSPENDED: PlayerState = { kind: "suspended" };
const STOPPING: PlayerState = { kind: "stopping" };

/** A live state: the player holds a session, from start until the context is released. */
interface Live {
  readonly state: PlayerState;
  readonly session: Session;
}

/** Plays a patch with the engine. */
export class EnginePlayer implements Player {
  /** The setup. */
  readonly #options: PlayerOptions;
  /** Who to tell about changes of state. */
  readonly #listeners = new Set<() => void>();
  /** The engine module, fetched once. */
  #wasm: ArrayBuffer | undefined;
  /** The state when no session is live: stopped or failed. */
  #idle: PlayerState = STOPPED;
  /** The live state and its session, if any: holding a session and being live are one. */
  #live: Live | undefined;
  /** The start in progress, if any. */
  #starting: Promise<void> | undefined;
  /** The stop in progress, if any. */
  #stopping: Promise<void> | undefined;

  /** Sets the player up; nothing is loaded before the first start. */
  constructor(options: PlayerOptions) {
    this.#options = options;
  }

  /** The current state. */
  state(): PlayerState {
    return this.#live?.state ?? this.#idle;
  }

  /** Calls `listener` after every change of state. */
  subscribe(listener: () => void): () => void {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  }

  /** Moves to the live `state` of `session`, and tells the listeners. */
  #goLive(state: PlayerState, session: Session): void {
    this.#live = { state, session };
    this.#notify();
  }

  /** Releases the live session (its owner closes it) and moves to `state`, and tells the listeners. */
  #goIdle(state: PlayerState): void {
    this.#live = undefined;
    this.#idle = state;
    this.#notify();
  }

  /** Tells the listeners the state changed. */
  #notify(): void {
    for (const listener of [...this.#listeners]) {
      listener();
    }
  }

  /** Fetches the engine module, once. */
  async #module(): Promise<ArrayBuffer> {
    if (this.#wasm === undefined) {
      const response = await fetch(this.#options.wasm);
      if (!response.ok) {
        throw new Error(`cannot load the engine from ${this.#options.wasm} (HTTP ${String(response.status)})`);
      }
      this.#wasm = await response.arrayBuffer();
    }
    return this.#wasm;
  }

  /** Starts playing. */
  start(): Promise<void> {
    if (this.#starting !== undefined) {
      return this.#starting;
    }
    if (this.#live !== undefined) {
      return Promise.resolve();
    }
    const lacking = missing(this.#options.environment);
    if (lacking !== undefined) {
      this.#goIdle({ kind: "failed", reason: explain(lacking) });
      return Promise.resolve();
    }
    // Created before any await, while the user gesture is still active (required by Safari).
    const session = new Session(this.#options.history);
    this.#goLive(STARTING, session);
    this.#starting = this.#open(session).finally(() => {
      this.#starting = undefined;
    });
    return this.#starting;
  }

  /** Loads the engine and the patch into `session`, then plays, fading in. */
  async #open(session: Session): Promise<void> {
    const { context, gain } = session;
    try {
      const loaded = await loadPatch(await this.#module(), this.#options.patch, context.sampleRate);
      await addProcessors(context, this.#options.worklet);
      const engine = createEngineNode(context, loaded, (error) => {
        void this.#fail(session, error);
      });
      const onBlock = (block: Float32Array[]): void => {
        const [first] = block;
        if (first !== undefined) {
          session.history.push(first);
        }
      };
      session.tapped(
        tap(context, engine.node, 1, onBlock, (error) => {
          void this.#fail(session, error);
        }),
      );
      engine.node.connect(gain).connect(context.destination);
      context.onstatechange = () => {
        this.#follow(session);
      };
      await engine.ready;
      await context.resume();
      const now = context.currentTime;
      gain.gain.setValueAtTime(0, now);
      gain.gain.linearRampToValueAtTime(this.#options.gain, now + this.#options.fade);
    } catch (error) {
      await this.#fail(session, error);
      return;
    }
    // A failure may have come first: then this session is no longer live.
    if (this.#live?.session === session) {
      this.#goLive(context.state === "running" ? PLAYING : SUSPENDED, session);
    }
  }

  /** Ends `session` on a failure, unless it is no longer live. */
  async #fail(session: Session, error: unknown): Promise<void> {
    if (this.#live?.session !== session) {
      return;
    }
    this.#goIdle({ kind: "failed", reason: String(error) });
    await session.close();
  }

  /** Follows the state of the context of `session`, changed by the browser. */
  #follow(session: Session): void {
    const live = this.#live;
    if (live?.session !== session) {
      return;
    }
    const state: string = session.context.state;
    if (state === "closed") {
      void this.#fail(session, new Error("the browser closed the audio device"));
    } else if (live.state === PLAYING && state !== "running") {
      this.#goLive(SUSPENDED, session);
    } else if (live.state === SUSPENDED && state === "running") {
      this.#goLive(PLAYING, session);
    }
  }

  /** Resumes a suspended player. */
  async resume(): Promise<void> {
    const live = this.#live;
    if (live?.state !== SUSPENDED) {
      return;
    }
    await live.session.context.resume();
    this.#follow(live.session);
  }

  /** Stops playing. */
  stop(): Promise<void> {
    this.#stopping ??= this.#stop().finally(() => {
      this.#stopping = undefined;
    });
    return this.#stopping;
  }

  /** Waits for a start in progress, then fades out and closes; from "failed", acknowledges it. */
  async #stop(): Promise<void> {
    // Without a start in progress, nothing is awaited before the state is "stopping".
    if (this.#starting !== undefined) {
      await this.#starting;
    }
    const live = this.#live;
    if (live === undefined) {
      this.#goIdle(STOPPED);
      return;
    }
    const { session } = live;
    this.#goLive(STOPPING, session);
    await this.#fadeOut(session);
    // The browser may have closed the context during the fade: it then failed instead.
    if (this.#live?.session === session) {
      this.#goIdle(STOPPED);
      await session.close();
    }
  }

  /**
   * Fades `session` out on the audio clock: the context closes only once the ramp has
   * reached silence, so that stopping never cuts a wave mid-cycle (a click). A suspended
   * context makes no sound and its clock does not run: it closes at once.
   */
  async #fadeOut(session: Session): Promise<void> {
    const { context, gain } = session;
    if (context.state !== "running") {
      return;
    }
    const end = context.currentTime + this.#options.fade;
    gain.gain.cancelScheduledValues(context.currentTime);
    gain.gain.setValueAtTime(gain.gain.value, context.currentTime);
    gain.gain.linearRampToValueAtTime(0, end);
    // A silent source (offset 0: never a constant signal to the speakers) whose end
    // marks the end of the ramp on the audio clock.
    const clock = new ConstantSourceNode(context, { offset: 0 });
    const ended = new Promise<void>((resolve) => {
      clock.onended = () => {
        resolve();
      };
    });
    clock.connect(context.destination);
    clock.start();
    clock.stop(end);
    await ended;
  }

  /** What was played most recently, or undefined when not playing. */
  latest(): Signal | undefined {
    const live = this.#live;
    return live?.state === PLAYING ? { samples: live.session.history.snapshot(), rate: live.session.context.sampleRate } : undefined;
  }
}
