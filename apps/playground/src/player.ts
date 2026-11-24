/**
 * Plays a patch with the engine in an `AudioContext`, and keeps the latest samples it
 * outputs for the oscilloscope.
 *
 * The player is a state machine: every state it can reach is visible through
 * {@link Player.state}, including the ones the browser or the audio thread cause
 * (suspended, failed), so that the page never claims to play while it does not.
 */

import { addProcessors, createEngineNode, loadPatch, tap, type Replaced, type SubnormalArithmetic, type Tap } from "@poyetik/audio-wasm/web";
import { audioTime, frameAt } from "./audio-clock.ts";
import type { Cause } from "./cause.ts";
import { failure } from "./failure.ts";
import { fetchBytes } from "./fetch.ts";
import { SampleHistory } from "./history.ts";
import { PLAYED_OUTPUT } from "./patch.ts";
import { cannotCompile, isWasmModule } from "./wasm-bytes.ts";

/** The latest output of the engine: samples of its first channel, and their rate. */
export interface Signal {
  /** The latest samples of the first channel, oldest first, as many as the history keeps. */
  readonly samples: Float32Array;
  /** Sample rate, in hertz. */
  readonly rate: number;
}

/** A moment of the audio clock: the frame it is at, and its rate. */
export interface AudioFrame {
  /** The frames the clock has played since its context was created ({@link frameAt}). */
  readonly frame: bigint;
  /** Sample rate of the clock, in hertz. */
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
  /**
   * The browser suspended the audio (another tab, a phone call, no user gesture): can
   * resume. `refused` when the browser kept it suspended after the last resume.
   */
  | { readonly kind: "suspended"; readonly refused: boolean }
  /** Fading out and releasing the audio device. */
  | { readonly kind: "stopping" }
  /**
   * Could not start, or stopped by a failure: `cause` says where (explained by
   * {@link explain}), `detail` what exactly (the error, for those who debug). Can start again.
   */
  | { readonly kind: "failed"; readonly cause: Failure; readonly detail: string }
  /** The browser lacks what the player needs (known from the start): cannot start. */
  | { readonly kind: "unsupported"; readonly lacking: Missing };

/** Something that plays sound and shows what it plays. */
export interface Player {
  /**
   * Starts playing, from "stopped" or "failed"; while starting, returns the start in
   * progress. Call it from a user gesture: browsers only start audio then. Never throws
   * nor rejects: a failure is a state.
   */
  start(): Promise<void>;
  /**
   * Stops playing, fading out; during a start, cancels it. From "failed", acknowledges it.
   * Never throws nor rejects: a failure is a state.
   */
  stop(): Promise<void>;
  /** Resumes a suspended player. Call it from a user gesture. Never throws nor rejects: a failure is a state. */
  resume(): Promise<void>;
  /** The current state. The same object until the state changes. */
  state(): PlayerState;
  /** Calls `listener` after every change of state; returns how to unsubscribe. */
  subscribe(listener: () => void): () => void;
  /** What was played most recently, or undefined when not playing. */
  latest(): Signal | undefined;
  /**
   * What the arithmetic of the browser's audio thread does with subnormal numbers, as the
   * engine observed it when it last loaded; undefined until it first has.
   */
  arithmetic(): SubnormalArithmetic | undefined;
  /**
   * What the engine's output stage replaced since the current or last start: the non-finite
   * samples it played as silence, how many and the first; undefined while none was.
   */
  replaced(): Required<Replaced> | undefined;
  /**
   * The frame of the audio clock now, what an image shown with the sound follows: from the
   * start of a session to its end, whatever its state (the clock stands still while the
   * context is suspended, and so does the frame); while the browser reports a clock that is
   * no time, the last frame read in the session, so that an image shown with it stays;
   * undefined only while no frame was read in a session yet (between sessions, there is none).
   */
  frame(): AudioFrame | undefined;
}

/**
 * Whether `arithmetic` flushes some subnormals to zero: then the engine's renders in this
 * browser are not proven bit-identical to its renders elsewhere (`spec/limitations.md`).
 */
export function flushes(arithmetic: SubnormalArithmetic): boolean {
  return [arithmetic.binary32, arithmetic.binary64].some((kept) => !kept.operands || !kept.results);
}

/** What a browser may lack to play a patch. */
export type Missing = "secure-context" | "web-audio" | "audio-worklet" | "webassembly";

/**
 * Where playing failed: loading the engine (network: the request, the server's answer, or
 * bytes that are no WebAssembly module), compiling it (a module this browser cannot
 * compile), running the engine, or the audio device (the browser's audio output).
 */
export type Failure = "load" | "module" | "engine" | "device";

/** The parts of the browser the player needs: `globalThis` in a page. */
export interface Environment {
  /** Whether the page is a secure context (HTTPS or localhost), without which browsers hide `AudioWorklet`. */
  readonly isSecureContext: boolean;
  /** The `AudioContext` constructor, missing from browsers without Web Audio. */
  readonly AudioContext?: unknown;
  /** The `AudioWorkletNode` constructor, missing from browsers without audio worklets (or outside a secure context). */
  readonly AudioWorkletNode?: unknown;
  /** The `WebAssembly` namespace, missing from browsers without WebAssembly (or where it is disabled). */
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

/** Whose each failure of the sound is: the engine's download, this browser that cannot compile it, the engine itself, this machine's audio device. */
export const FAILURE_CAUSE: Readonly<Record<Failure, Cause>> = { load: "network", module: "environment", engine: "defect", device: "environment" };

/** What to do about a failure of the sound, by its cause ({@link FAILURE_CAUSE}): words that fit each of its members ({@link explain} adds what fits one alone). */
export const SOUND_REMEDY: Readonly<Record<Cause, string>> = {
  network: "Press Start to try again; if it fails again, check your connection.",
  environment: "Press Start to try again; if it fails again, try another browser or device.",
  defect: "Press Start to try again; if it happens again, its details say what failed.",
};

/** Explains to the user what is missing or what failed, and what to do next: for a failure, the remedy of its cause. */
export function explain(problem: Missing | Failure): string {
  switch (problem) {
    case "secure-context":
      return "Audio needs a secure page: open this page over HTTPS (or from localhost).";
    case "web-audio":
      return "This browser cannot play audio from a page (no Web Audio). Try a recent Chrome, Firefox, Safari or Edge.";
    case "audio-worklet":
      return "This browser cannot run the engine in real time (no AudioWorklet). Try a recent Chrome, Firefox, Safari or Edge.";
    case "webassembly":
      return "This browser cannot run the engine (no WebAssembly). Try a recent Chrome, Firefox, Safari or Edge.";
    case "load":
      return `Could not load the audio engine. ${SOUND_REMEDY[FAILURE_CAUSE.load]}`;
    case "module":
      return `This browser could not compile the audio engine. ${SOUND_REMEDY[FAILURE_CAUSE.module]}`;
    case "engine":
      return `The audio engine stopped on an error. ${SOUND_REMEDY[FAILURE_CAUSE.engine]}`;
    case "device":
      return `The browser could not play audio on this device: check the sound output. ${SOUND_REMEDY[FAILURE_CAUSE.device]}`;
  }
}

/**
 * The sample rates the engine accepts, in hertz (both included). Browsers run their
 * contexts from 3 kHz to 768 kHz; outside this range the player asks for
 * {@link FALLBACK_RATE} instead. The tests check these limits against the engine itself
 * and the test matrix.
 */
export const ENGINE_RATES = { min: 8_000, max: 384_000 } as const;

/** The rate asked for when the device runs outside {@link ENGINE_RATES}: the browser resamples to the device. */
export const FALLBACK_RATE = 48_000;

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
  /** Duration kept for the oscilloscope, in seconds: as many samples as that at the context rate. */
  readonly historySeconds: number;
  /** The browser, checked once, when the player is created. */
  readonly environment: Environment;
}

/**
 * How often the player reads an audio clock the browser reports as no time while its
 * context runs, in milliseconds, waiting to fade in ({@link CLOCK_READS}).
 */
export const CLOCK_POLL_MS = 20;

/**
 * How many times the player reads an audio clock that is no time while its context runs
 * before it fails on the device, counted from the first read of a wait: a context that
 * stops running ends the wait at its next read, and a new run waits anew, but one that runs
 * again before that read goes on with the same wait and its count. {@link CLOCK_POLL_MS}
 * apart while the page's timers run on time, about one second in a visible page; a browser
 * slows a hidden page's timers, and the wait with them. The page shows no playing state
 * meanwhile, since the tone is silent until it fades in.
 */
export const CLOCK_READS = 50;

/**
 * `value` when it is a number (finite), else `fallback`: every value the player gives a Web
 * Audio scheduling call passes here, each call throwing a `TypeError` ("non-finite") on any
 * other, that the time it is told comes from the clock's one reader ({@link audioTime}).
 */
function finiteOr(value: number, fallback: number): number {
  return Number.isFinite(value) ? value : fallback;
}

/** Waits `milliseconds` on a timer. */
function pause(milliseconds: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

/** Why a wait of a session stops: the session ended first, so nobody needs its result. */
class SessionEnded extends Error {}

/**
 * One audio context, from start to close. The session owns its lifetime: every wait on
 * the browser (a download, the engine, the audio clock) ends with the session, so that
 * nothing the player does can wait for an event a closed or frozen context never sends.
 */
class Session {
  /** The context of this session, at the device rate when the engine accepts it, closed by {@link Session.close}. */
  readonly context: AudioContext;
  /** The latest engine output, {@link PlayerOptions.historySeconds} long in samples at the context rate. */
  readonly history: SampleHistory;
  /** The output stage, silent until the fade in: fades and gain, before the speakers. */
  readonly gain: GainNode;
  /**
   * Where {@link Session.settleStart} announces the end of the start, to the executor of
   * {@link Session.started}: `Promise.withResolvers`, which would hand the resolver over,
   * is missing from browsers the friendly checks still explain.
   */
  readonly #start = new EventTarget();
  /** The start of the session: settles once it plays, fails or ends ({@link Session.settleStart}). */
  readonly started = new Promise<undefined>((resolve) => {
    this.#start.addEventListener("settled", () => {
      resolve(undefined);
    });
  });
  /** The stop of the session, once asked: settles once the session ended. */
  stopping: Promise<void> | undefined;
  /** Whether a fade in is due ({@link Session.fadeDue}). */
  #fadeDue = false;
  /** The wait for a clock that is a time while the context runs, in flight: one at a time ({@link Session.timeOnce}). */
  #timing: Promise<void> | undefined;
  /** Cancels the downloads and the waits of the session when it ends. */
  readonly #abort = new AbortController();
  /**
   * Rejects once the session ends: every wait of the session races it. Never an unhandled
   * rejection: the start races it as soon as the session exists, and a race keeps its handler.
   */
  readonly #ended: Promise<never>;
  /** The tap feeding the history, once connected. */
  #tap: Tap | undefined;
  /** The last frame {@link Session.frame} read of the clock, if it read one. */
  #frame: AudioFrame | undefined;

  /**
   * Opens a session on a new context.
   * @param historySeconds Duration kept for the oscilloscope, in seconds.
   */
  constructor(historySeconds: number) {
    const device = new AudioContext();
    const { sampleRate } = device;
    if (sampleRate < ENGINE_RATES.min || sampleRate > ENGINE_RATES.max) {
      // Never used: released at once, whether or not the browser manages to.
      void device.close().catch((_unclosable: unknown) => undefined);
      this.context = new AudioContext({ sampleRate: FALLBACK_RATE });
    } else {
      this.context = device;
    }
    this.history = new SampleHistory(Math.round(historySeconds * this.context.sampleRate));
    this.gain = new GainNode(this.context, { gain: 0 });
    const { signal } = this.#abort;
    this.#ended = new Promise((_resolve, reject) => {
      signal.addEventListener("abort", () => {
        reject(new SessionEnded());
      });
    });
  }

  /**
   * Whether a fade in is due: asked while the clock was no time, the output silent since
   * ({@link Session.silence}). Cleared by the fade in, and by the fade out, after which none is due.
   */
  fadeDue(): boolean {
    return this.#fadeDue;
  }

  /** Records whether a fade in is due ({@link Session.fadeDue}). */
  owesFade(due: boolean): void {
    this.#fadeDue = due;
  }

  /** Starts the wait `wait` makes, unless one is in flight; forgets it once it settles. `wait` never rejects. */
  timeOnce(wait: () => Promise<void>): void {
    if (this.#timing === undefined) {
      this.#timing = wait().finally(() => {
        this.#timing = undefined;
      });
    }
  }

  /** Silences the output at once, untimed, whatever it had scheduled: what is left when the clock is no time. */
  silence(): void {
    this.gain.gain.cancelScheduledValues(0);
    this.gain.gain.value = 0;
  }

  /**
   * The frame of the clock now, or the last one read in this session while the browser
   * reports a clock that is no time; undefined only before the session's first reading.
   */
  frame(): AudioFrame | undefined {
    const now = audioTime(this.context);
    if (now !== undefined) {
      this.#frame = { frame: frameAt(now, this.context.sampleRate), rate: this.context.sampleRate };
    }
    return this.#frame;
  }

  /** Aborted when the session ends: given to the downloads of the session. */
  get signal(): AbortSignal {
    return this.#abort.signal;
  }

  /**
   * Waits for `event`, unless the session ends first.
   * @throws {SessionEnded} when the session ends before `event` settles.
   */
  until<T>(event: Promise<T>): Promise<T> {
    return Promise.race([event, this.#ended]);
  }

  /** Settles {@link Session.started}: the session plays, failed or ended. */
  settleStart(): void {
    this.#start.dispatchEvent(new Event("settled"));
  }

  /** Records the tap, once connected. */
  tapped(running: Tap): void {
    this.#tap = running;
  }

  /**
   * Ends the session: its waits stop, its downloads are cancelled, the tap stops and the
   * context is released. Called once per session: only by whoever detaches it from the
   * player first (a stop, a cancel or a failure). Never rejects: a context that cannot be
   * closed (the browser rejects closing a context it already closed) has nothing left to
   * do, and the session is already over, so that failure is reported nowhere.
   */
  async close(): Promise<void> {
    this.#abort.abort();
    this.#tap?.stop();
    // Dropped: a context that cannot be closed has nothing left to do (above).
    await this.context.close().catch((_unclosable: unknown) => undefined);
  }
}
/** The states, as shared objects: `state()` returns the same object until a change. */
const STOPPED: PlayerState = { kind: "stopped" };
/** Shared state while the engine and the patch load. */
const STARTING: PlayerState = { kind: "starting" };
/** Shared state while the context runs. */
const PLAYING: PlayerState = { kind: "playing" };
/** Shared state while the browser keeps the context suspended. */
const SUSPENDED: PlayerState = { kind: "suspended", refused: false };
/** Shared state while the browser keeps the context suspended, though asked to resume. */
const SUSPENDED_REFUSED: PlayerState = { kind: "suspended", refused: true };
/** Shared state while fading out and closing the context. */
const STOPPING: PlayerState = { kind: "stopping" };

/** A live state: the player holds a session, from start until the context is released. */
interface Live {
  /** What the player shows while this session lives: starting, playing, suspended or stopping. */
  readonly state: PlayerState;
  /** The session the state belongs to, ended when the player leaves it. */
  readonly session: Session;
}

/** Plays a patch with the engine. */
export class EnginePlayer implements Player {
  /** The setup. */
  readonly #options: PlayerOptions;
  /** Who to tell about changes of state. */
  readonly #listeners = new Set<() => void>();
  /** The engine module, fetched once while it compiles: forgotten when it does not. */
  #wasm: ArrayBuffer | undefined;
  /** The state when no session is live: stopped, failed or unsupported. */
  #idle: PlayerState;
  /** The live state and its session, if any: holding a session and being live are one. */
  #live: Live | undefined;
  /** What {@link EnginePlayer.arithmetic} returns: the arithmetic the engine last observed. */
  #arithmetic: SubnormalArithmetic | undefined;
  /** What {@link EnginePlayer.replaced} returns: what the output stage replaced since the last start. */
  #replaced: Required<Replaced> | undefined;

  /**
   * Sets the player up; nothing is loaded before the first start. A browser lacking what
   * the player needs is known at once: the player is then unsupported, for good.
   */
  constructor(options: PlayerOptions) {
    this.#options = options;
    const lacking = missing(options.environment);
    this.#idle = lacking === undefined ? STOPPED : { kind: "unsupported", lacking };
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

  /** Whether `session` is the live one: a session the player released is over for good. */
  #holds(session: Session): boolean {
    return this.#live?.session === session;
  }

  /** Tells the listeners the state changed. */
  #notify(): void {
    for (const listener of [...this.#listeners]) {
      listener();
    }
  }

  /**
   * Fetches the engine module, once while it compiles: a download the end of `session`
   * cancels. Errors name the URL; bytes that are no WebAssembly module are a download that
   * failed, never kept, so that the next start downloads again.
   */
  async #module(session: Session): Promise<ArrayBuffer> {
    if (this.#wasm === undefined) {
      const bytes = await session.until(fetchBytes("the engine", this.#options.wasm, { signal: session.signal }));
      if (!isWasmModule(bytes)) {
        throw new Error(`cannot load the engine from ${this.#options.wasm} (not a WebAssembly module)`);
      }
      this.#wasm = bytes;
    }
    return this.#wasm;
  }

  /** Starts playing. */
  start(): Promise<void> {
    const live = this.#live;
    if (live !== undefined) {
      return live.session.started;
    }
    if (this.#idle.kind === "unsupported") {
      return Promise.resolve();
    }
    // Forgotten before the start is told, so that its first notification carries no report
    // of the last session, and a report of this one fills the page's live region anew.
    this.#replaced = undefined;
    let session;
    try {
      // Created before any await, while the user gesture is still active (required by Safari).
      session = new Session(this.#options.historySeconds);
    } catch (error) {
      // No context, for instance no audio device: there is no session to end.
      this.#goIdle({ kind: "failed", cause: "device", detail: failure(error) });
      return Promise.resolve();
    }
    this.#goLive(STARTING, session);
    void this.#open(session);
    return session.started;
  }

  /**
   * Loads the engine and the patch into `session`, then plays, fading in; settles the
   * start of `session` in every case.
   */
  async #open(session: Session): Promise<void> {
    const { context, gain } = session;
    // Where the start is: what a failure there is caused by.
    let cause: Failure = "load";
    // Followed from before the first await: a context the browser closes at any point of
    // the start fails the session on the device, never later on whatever awaited it.
    context.onstatechange = () => {
      this.#follow(session);
    };
    try {
      const wasm = await this.#module(session);
      await session.until(addProcessors(context, this.#options.worklet));
      cause = "engine";
      // The engine's default precision, binary64.
      const loaded = await session.until(loadPatch(wasm, this.#options.patch, context.sampleRate, undefined, PLAYED_OUTPUT)).catch((error: unknown) => {
        // A module this browser cannot compile is no use to keep: its bytes are forgotten, so
        // that the next start fetches what the server serves then.
        if (cannotCompile(error)) {
          this.#wasm = undefined;
          cause = "module";
        }
        throw error;
      });
      const engine = createEngineNode(context, loaded, {
        onFailure: (error) => {
          void this.#fail(session, "engine", error);
        },
        // The playground sets nothing live: a refusal is a failure of its engine.
        onRefused: (error) => {
          void this.#fail(session, "engine", error);
        },
        onReplaced: (replaced) => {
          if (this.#live?.session === session) {
            this.#replaced = replaced;
            this.#notify();
          }
        },
      });
      const onBlock = (block: Float32Array[]): void => {
        const [first] = block;
        if (first !== undefined) {
          session.history.push(first);
        }
      };
      session.tapped(
        tap(context, engine.node, 1, onBlock, (error) => {
          void this.#fail(session, "engine", error);
        }),
      );
      engine.node.connect(gain).connect(context.destination);
      // Awaited before resuming: Chromium, Firefox and WebKit all construct the processor,
      // and so settle `ready`, while the context is still suspended: checked once by hand in
      // Playwright's builds of each (Safari unchecked); an end-to-end test is #24 step 54.
      this.#arithmetic = await session.until(engine.ready);
      cause = "device";
      await session.until(context.resume());
      this.#fadeIn(session);
      // Still starting while a running context's clock is no time: its tone is silent.
      await this.#timed(session);
      // A failure or a cancel may have come first: then this session is no longer live.
      if (this.#holds(session)) {
        this.#goLive(context.state === "running" ? PLAYING : SUSPENDED, session);
      }
    } catch (error) {
      await this.#fail(session, cause, error);
    } finally {
      session.settleStart();
    }
  }

  /**
   * Fades `session` in on the audio clock, from silence at its current time: when the
   * context runs, at once; when it is suspended, its clock is frozen and the ramp plays
   * as soon as it resumes, whoever resumes it. Never a step from silence to full level (a
   * click). On a clock that is no time, silent at once, untimed, the fade in due until the
   * clock is read anew ({@link EnginePlayer.#timed}, or the next run of the context).
   */
  #fadeIn(session: Session): void {
    const { context, gain } = session;
    const now = audioTime(context);
    if (now === undefined) {
      if (!session.fadeDue()) {
        session.owesFade(true);
        session.silence();
      }
      return;
    }
    session.owesFade(false);
    gain.gain.cancelScheduledValues(now);
    gain.gain.setValueAtTime(0, now);
    gain.gain.linearRampToValueAtTime(finiteOr(this.#options.gain, 0), finiteOr(now + this.#options.fade, now));
  }

  /**
   * Waits, while the context of `session` runs and a fade in is due, for its clock to be a
   * time, reading it every {@link CLOCK_POLL_MS}, and fades in then. Ends, the fade still
   * due, when the context stops running.
   * @throws {Error} when the clock is still no time after {@link CLOCK_READS} reads.
   * @throws {SessionEnded} when the session ends first.
   */
  async #timed(session: Session): Promise<void> {
    for (let reads = 0; session.fadeDue() && session.context.state === "running"; reads += 1) {
      if (reads === CLOCK_READS) {
        throw new Error("the browser's audio clock is not a time");
      }
      await session.until(pause(CLOCK_POLL_MS));
      // A fade out meanwhile leaves no fade in due.
      if (session.fadeDue()) {
        this.#fadeIn(session);
      }
    }
  }

  /**
   * Plays `session`, suspended, once its running context's clock is a time and it faded
   * in ({@link EnginePlayer.#timed}); fails on the device when it stays no time.
   */
  async #playTimed(session: Session): Promise<void> {
    try {
      await this.#timed(session);
    } catch (error) {
      // Nothing when the session ended first: it is no longer live.
      await this.#fail(session, "device", error);
      return;
    }
    // The session is the live one: its wait resolved while it was, and no other session
    // suspends meanwhile (a start suspends only after awaits); once it is over, the state is idle.
    if (this.state().kind === "suspended" && !session.fadeDue()) {
      this.#goLive(PLAYING, session);
    }
  }

  /** Ends `session` and moves to `state`, unless the session is no longer live. */
  async #end(session: Session, state: PlayerState): Promise<void> {
    if (!this.#holds(session)) {
      return;
    }
    this.#goIdle(state);
    await session.close();
  }

  /** Ends `session` on a failure caused by `cause`, unless it is no longer live. */
  #fail(session: Session, cause: Failure, error: unknown): Promise<void> {
    return this.#end(session, { kind: "failed", cause, detail: failure(error) });
  }

  /**
   * Follows the state of the context of `session`, changed by the browser. A suspension
   * while playing arms the fade in for the resume. A context that stops running while
   * fading out makes no sound: the stop ends at once. While starting, the start decides.
   */
  #follow(session: Session): void {
    const live = this.#live;
    if (live?.session !== session) {
      return;
    }
    // A string: Safari also reports "interrupted", which the DOM types do not list.
    const state: string = session.context.state;
    if (state === "closed") {
      void this.#fail(session, "device", new Error("the browser closed the audio device"));
    } else if (state === "running") {
      if (session.fadeDue()) {
        this.#fadeIn(session);
      }
      if (live.state.kind === "suspended") {
        if (session.fadeDue()) {
          // Paused until it fades in: its tone is silent meanwhile. The browser runs the
          // context: a resume it refused before is refused no more, and the page says so
          // (the same state, already not refused, renders nothing again).
          this.#goLive(SUSPENDED, session);
          session.timeOnce(() => this.#playTimed(session));
        } else {
          this.#goLive(PLAYING, session);
        }
      }
    } else if (live.state === PLAYING) {
      this.#fadeIn(session);
      this.#goLive(SUSPENDED, session);
    } else if (live.state === STOPPING) {
      void this.#end(session, STOPPED);
    }
  }

  /** Resumes a suspended player; when the browser keeps it suspended, says so. */
  async resume(): Promise<void> {
    const live = this.#live;
    if (live?.state.kind !== "suspended") {
      return;
    }
    try {
      await live.session.until(live.session.context.resume());
    } catch (error) {
      // A context that cannot resume is closing or broken.
      await this.#fail(live.session, "device", error);
      return;
    }
    this.#follow(live.session);
    // Refused exactly when the browser does not run the context: waiting for the clock while
    // it runs is no refusal, and a wait still in flight once it stopped running is no grant.
    if (this.state().kind === "suspended" && live.session.context.state !== "running") {
      this.#goLive(SUSPENDED_REFUSED, live.session);
    }
  }

  /** Stops playing; cancels a start; from "failed", acknowledges it; unsupported stays so. */
  stop(): Promise<void> {
    const live = this.#live;
    if (live === undefined) {
      if (this.#idle.kind === "failed") {
        this.#goIdle(STOPPED);
      }
      return Promise.resolve();
    }
    if (live.state === STARTING) {
      return this.#end(live.session, STOPPED);
    }
    live.session.stopping ??= this.#stop(live.session);
    return live.session.stopping;
  }

  /** Fades `session` out, then ends it. */
  async #stop(session: Session): Promise<void> {
    this.#goLive(STOPPING, session);
    try {
      await this.#fadeOut(session);
    } catch (error) {
      await this.#fail(session, "device", error);
      return;
    }
    await this.#end(session, STOPPED);
  }

  /**
   * Fades `session` out on the audio clock: the context closes only once the ramp has
   * reached silence, so that stopping never cuts a wave mid-cycle (a click). A suspended
   * context makes no sound and its clock does not run: it closes at once; so does a context
   * whose clock is no time, silent at once, untimed. The wait ends with the session, if the
   * browser or a failure ends it first. No fade in is due after it.
   */
  async #fadeOut(session: Session): Promise<void> {
    const { context, gain } = session;
    const due = session.fadeDue();
    session.owesFade(false);
    if (context.state !== "running") {
      return;
    }
    const now = audioTime(context);
    if (now === undefined) {
      // Already silent when a fade in was due.
      if (!due) {
        session.silence();
      }
      return;
    }
    const end = finiteOr(now + this.#options.fade, now);
    gain.gain.cancelScheduledValues(now);
    // A browser may read the gain as no number when its ramp was just scheduled (WebKit, by
    // W-1's likely account): the ramp has then not begun, so the gain is still where the fade
    // in starts it, silence. 0 is that level, not only a value the call accepts.
    gain.gain.setValueAtTime(finiteOr(gain.gain.value, 0), now);
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
    // Dropped: the session ended first, so nobody waits for the fade's end any more.
    await session.until(ended).catch((_sessionEnded: unknown) => undefined);
  }

  /** The frame of the audio clock now, the session's last while its clock is no time; undefined while no frame was read in a session yet. */
  frame(): AudioFrame | undefined {
    return this.#live?.session.frame();
  }

  /** What was played most recently, or undefined when not playing. */
  latest(): Signal | undefined {
    const live = this.#live;
    return live?.state === PLAYING ? { samples: live.session.history.snapshot(), rate: live.session.context.sampleRate } : undefined;
  }

  /** The arithmetic of the audio thread the engine last observed, or undefined before it first has. */
  arithmetic(): SubnormalArithmetic | undefined {
    return this.#arithmetic;
  }

  /** What the output stage replaced since the last start, or undefined while nothing was. */
  replaced(): Required<Replaced> | undefined {
    return this.#replaced;
  }
}
