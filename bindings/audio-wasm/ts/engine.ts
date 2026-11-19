/**
 * The engine WebAssembly module, driven from JavaScript: in an `AudioWorklet`, on the
 * main thread or in Node (the package exports no Node entry point yet: it comes with the
 * SDK, #32 step 15; the tests import this module directly). See `bindings/audio-wasm/src/ffi.rs` for the exported interface.
 */
import { ModuleError, explanation, readExports, succeeded, type Exports } from "./abi.ts";
import { MAX_ENGINE_BLOCK } from "./blocks.ts";

export { ABI_VERSION, ModuleError } from "./abi.ts";

/**
 * Every render format, in the order of the engine's codes (`audio_core::RenderFormat::ALL`):
 * computed and given in binary64; computed in binary64 and given in binary32; computed and
 * given in binary32.
 */
export const RENDER_FORMATS = ["binary64", "binary64-as-binary32", "binary32"] as const;

/** What a render is: the precision that computes it and the format of its samples. */
export type RenderFormat = (typeof RENDER_FORMATS)[number];

/** The render formats whose samples are binary32: what Web Audio takes. */
export type Binary32Format = Exclude<RenderFormat, "binary64">;

/** Every precision a render computes in, the default first: a host setting. */
export const PRECISIONS = ["binary64", "binary32"] as const;

/** The precision a render computes in: binary64 by default, binary32 for performance. */
export type Precision = (typeof PRECISIONS)[number];

/** Whether subnormals survive in one format: as the operands of an operation, and as its result. */
export interface SubnormalsKept {
  /** A subnormal operand is used as it is, not as zero (no denormals-are-zero). */
  readonly operands: boolean;
  /** A subnormal result is given as it is, not as zero (no flush-to-zero). */
  readonly results: boolean;
}

/**
 * What the arithmetic of a thread does with subnormal numbers, in each format, as the
 * module observed it (`audio_core::SubnormalArithmetic`). Every subnormal kept is IEEE
 * arithmetic, the engine's own; a browser may run its audio thread in a hardware flush
 * mode, which this shows when it reaches the module's arithmetic.
 */
export interface SubnormalArithmetic {
  /** In binary32. */
  readonly binary32: SubnormalsKept;
  /** In binary64. */
  readonly binary64: SubnormalsKept;
}

/** The largest code of `engine_subnormals`: its four bits set, IEEE arithmetic. */
const SUBNORMAL_CODES = 0b1111;

/**
 * Reads the code of `engine_subnormals` (`subnormals_code` in `ffi.rs`): four bits, each set
 * when subnormals are kept, 1 binary32 operands, 2 binary32 results, 4 binary64 operands,
 * 8 binary64 results; `undefined` for anything else.
 */
export function subnormalArithmetic(code: number): SubnormalArithmetic | undefined {
  if (!Number.isInteger(code) || code < 0 || code > SUBNORMAL_CODES) {
    return undefined;
  }
  const kept = (shift: number): SubnormalsKept => ({ operands: ((code >> shift) & 1) === 1, results: ((code >> (shift + 1)) & 1) === 1 });
  return { binary32: kept(0), binary64: kept(2) };
}

/**
 * The render format of a render computed in `precision` and given to a host that takes
 * binary32 (Web Audio): a binary64 render is rounded to binary32, a binary32 one given as it
 * is.
 */
export function givenAsBinary32(precision: Precision): Binary32Format {
  return GIVEN_AS_BINARY32[precision];
}

/** The render format of each precision given as binary32: a record, so that a new precision does not compile without one. */
const GIVEN_AS_BINARY32: Readonly<Record<Precision, Binary32Format>> = { binary64: "binary64-as-binary32", binary32: "binary32" };

/**
 * The precision `value` names, read at a boundary (a message, a file): `undefined` when it
 * names none. The one reader of a precision from outside the type system; each caller
 * refuses `undefined` with its own error.
 */
export function parsePrecision(value: unknown): Precision | undefined {
  return PRECISIONS.find((name) => name === value);
}

/** The sample arrays a render format fills: `Float64Array` for binary64, `Float32Array` otherwise. */
export type Samples = Float32Array | Float64Array;

/** How the samples of a render format are laid out in the module memory. */
interface Layout {
  /** Bytes per sample: 8 or 4. */
  readonly bytes: number;
  /** A view of the whole memory as samples. */
  readonly view: (buffer: ArrayBuffer) => Samples;
}

/** The layout of binary64 samples. */
const BINARY64: Layout = {
  bytes: Float64Array.BYTES_PER_ELEMENT,
  view: (buffer) => new Float64Array(buffer),
};

/** The layout of binary32 samples. */
const BINARY32: Layout = {
  bytes: Float32Array.BYTES_PER_ELEMENT,
  view: (buffer) => new Float32Array(buffer),
};

/**
 * Thrown when the engine refuses to load a patch: the patch is invalid or cannot be
 * planned, but also when it is not UTF-8 or when the engine does not support the sample
 * rate (separate errors are planned with the public API, #32 step 12). `detail` is the
 * UTF-8 explanation: decode it where a `TextDecoder` is available (an `AudioWorklet` may
 * not have one).
 */
export class PatchError extends Error {
  /** The UTF-8 explanation given by the engine. */
  readonly detail: Uint8Array;

  /**
   * Records the explanation of the engine.
   * @param detail The UTF-8 explanation given by the engine.
   */
  constructor(detail: Uint8Array) {
    super("the engine rejected the patch");
    this.name = "PatchError";
    this.detail = detail;
  }
}

/**
 * What the output stage does (`spec/engine-design.md`, decision 12), each mode at the index
 * of its code in the C interface (`engine_output`): raw, without a limiter or latency;
 * protected by the true-peak limiter, never switched to raw; switchable, raw first, with the limiter's latency kept
 * so that it can be protected live ({@link WasmEngine.setProtected}).
 */
export const OUTPUT_MODES = ["raw", "protected", "switchable"] as const;

/** A mode of the output stage, one of {@link OUTPUT_MODES}. */
export type OutputMode = (typeof OUTPUT_MODES)[number];

/**
 * How precisely the limiter estimates the true peak (`spec/engine-design.md`, decision 12),
 * each at the index of its code in the C interface (`engine_output`): standard, BS.1770's
 * own interpolator, the cheapest; precise, the default; mastering, the most precise near
 * half the rate. Fixed with the engine, since it sets part of the latency.
 */
export const DETECTORS = ["standard", "precise", "mastering"] as const;

/** A detector of the limiter's true peak (how precisely it estimates it), one of {@link DETECTORS}. */
export type Detector = (typeof DETECTORS)[number];

/**
 * The longest lookahead of the limiter, in milliseconds, and so the largest maximum
 * lookahead of an output stage: the end of the engine's range of both
 * (`audio_core::Setting::range`), accepted, and the next number up refused, by the tests.
 */
export const MAX_LOOKAHEAD = 5;

/**
 * The settings of the output stage's limiter that change live: the ceiling in dBTP (-20 to
 * 0), the lookahead and the release in milliseconds (0.5 to {@link MAX_LOOKAHEAD}, 10 to
 * 1000), the adaptivity of the release (0, a fixed release, to 1, fully program-dependent).
 * The engine checks them against its ranges (`audio_core::Setting::range`).
 */
export interface LimiterSettings {
  /** The ceiling, in dBTP. */
  readonly ceiling: number;
  /** The lookahead, in milliseconds. */
  readonly lookahead: number;
  /** The release, in milliseconds. */
  readonly release: number;
  /** The adaptivity of the release, from 0 to 1. */
  readonly adaptivity: number;
}

/** The engine's default limiter settings: -1 dBTP, 1.5 ms, 100 ms, 0.5. */
export const DEFAULT_LIMITER: LimiterSettings = Object.freeze({ ceiling: -1, lookahead: 1.5, release: 100, adaptivity: 0.5 });

/** A raw output stage: non-finite samples replaced, nothing limited, no latency; it has no limiter, so no limiter setting. */
export interface RawOutput {
  /** The mode. */
  readonly mode: "raw";
}

/**
 * What an output stage with the true-peak limiter holds beside its mode: the maximum
 * lookahead in milliseconds (0.5 to {@link MAX_LOOKAHEAD}) and the detector, which fix the limiter's latency for
 * the life of the engine, and the limiter's settings.
 */
export interface LimiterStage {
  /** The maximum lookahead, in milliseconds. */
  readonly maximumLookahead: number;
  /** The limiter's peak detector. */
  readonly detector: Detector;
  /** The limiter's settings. */
  readonly limiter: LimiterSettings;
}

/** A protected output stage: limited from its first sample, never switched to raw (the engine refuses). */
export interface ProtectedOutput extends LimiterStage {
  /** The mode. */
  readonly mode: "protected";
}

/** A switchable output stage: raw first, with the limiter's latency kept, protected live by a crossfade ({@link WasmEngine.setProtected}). */
export interface SwitchableOutput extends LimiterStage {
  /** The mode. */
  readonly mode: "switchable";
}

/** An output stage with the true-peak limiter: protected or switchable. */
export type LimitedOutput = ProtectedOutput | SwitchableOutput;

/** The output stage an engine is built with: raw, or with a limiter and its settings. */
export type Output = RawOutput | LimitedOutput;

/** A raw output stage: non-finite samples replaced, nothing limited, no latency. */
export const RAW_OUTPUT: RawOutput = Object.freeze({ mode: "raw" });

/** A protected output stage, the playground's (decision 12): the default settings and detector, the largest maximum lookahead ({@link MAX_LOOKAHEAD}). */
export const PROTECTED_OUTPUT: ProtectedOutput = Object.freeze({ mode: "protected", maximumLookahead: MAX_LOOKAHEAD, detector: "precise", limiter: DEFAULT_LIMITER });

/** Thrown when the engine refuses a setting of its output stage, with its explanation. */
export class OutputError extends Error {
  /** The UTF-8 explanation given by the engine. */
  readonly detail: Uint8Array;

  /**
   * Records the explanation of the engine.
   * @param detail The UTF-8 explanation given by the engine.
   */
  constructor(detail: Uint8Array) {
    super("the engine refused the output settings");
    this.name = "OutputError";
    this.detail = detail;
  }
}

/**
 * Changes the limiter of the engine module `exports` live (`engine_limiter`): whether it
 * succeeded.
 * @throws {ModuleError} when the module answers neither 1 nor 0.
 */
function limiterSet(exports: Exports, settings: LimiterSettings): boolean {
  const { ceiling, lookahead, release, adaptivity } = settings;
  return succeeded("engine_limiter", exports.limiter(ceiling, lookahead, release, adaptivity));
}

/**
 * Sets the limited output stage `output` for the next load of the engine module `exports`
 * (`engine_output`), its limiter's settings with it: whether it succeeded.
 * @throws {ModuleError} when the module answers neither 1 nor 0.
 */
function outputSet(exports: Exports, output: LimitedOutput): boolean {
  const { ceiling, lookahead, release, adaptivity } = output.limiter;
  return succeeded("engine_output", exports.output(OUTPUT_MODES.indexOf(output.mode), output.maximumLookahead, DETECTORS.indexOf(output.detector), ceiling, lookahead, release, adaptivity));
}

/**
 * Thrown when the engine module traps (a bug, or memory exhausted). The instance is then
 * in an unknown state: the engine refuses to render again, and the patch must be loaded
 * into a new engine.
 */
export class EngineTrap extends Error {
  /**
   * Records the trap.
   * @param options The trap raised by the WebAssembly engine, if any.
   */
  constructor(options?: ErrorOptions) {
    super("the engine module trapped and cannot be used again", options);
    this.name = "EngineTrap";
  }
}

/**
 * Encodes `text`, which is ASCII, as UTF-8 bytes (the detail of a {@link PatchError} the
 * engine is not asked for): an `AudioWorklet` may have no `TextEncoder`.
 */
function ascii(text: string): Uint8Array {
  return Uint8Array.from(text, (character) => character.charCodeAt(0));
}

/** Instantiates an engine module, explaining why it cannot be. */
function instantiate(module: WebAssembly.Module): WebAssembly.Exports {
  try {
    return new WebAssembly.Instance(module, {}).exports;
  } catch (error) {
    throw new ModuleError(`cannot be instantiated: ${String(error)}`, { cause: error });
  }
}

/** Runs `action` on the engine module, turning a trap into an {@link EngineTrap}. */
function guarded<T>(action: () => T): T {
  try {
    return action();
  } catch (error) {
    throw error instanceof WebAssembly.RuntimeError ? new EngineTrap({ cause: error }) : error;
  }
}

/** The first sample the output stage replaced: the earliest frame, then the lowest channel. */
export interface FirstReplaced {
  /** Its frame, counted from the first frame of the engine (exact up to 2^53). */
  readonly frame: number;
  /** Its channel. */
  readonly channel: number;
}

/**
 * What the output stage of an engine replaced since it was loaded (`spec/engine-design.md`,
 * decision 12): how many non-finite samples (NaN or infinite), each given as zero, and the
 * first of them.
 */
export interface Replaced {
  /** How many, saturating at {@link MAX_REPLACED_COUNT}: a count equal to it is a lower bound. */
  readonly count: number;
  /** The first of them; absent while none was. */
  readonly first?: FirstReplaced;
}

/**
 * The largest count of replaced samples the engine reports, 2^32 - 1 (the count is one
 * `u32` word of `engine_replaced`): it saturates there, so that a count equal to it is a
 * lower bound, and grows no more.
 */
export const MAX_REPLACED_COUNT = 0xff_ff_ff_ff;

/** What an engine that replaced nothing reports: one shared, frozen value. */
const NOTHING_REPLACED: Replaced = Object.freeze({ count: 0 });

/**
 * One patch loaded into its own instance of the engine module, rendering in one render
 * format into arrays of type `A`: `Float64Array` for binary64, `Float32Array` for the two
 * binary32 formats.
 *
 * The engine renders fixed blocks; {@link WasmEngine.render} fills buffers of any length
 * from them, so the result never depends on how the host splits time. Rendering does
 * not allocate when every buffer is a whole number of blocks (Web Audio's quanta with a
 * module whose engine block divides the quantum): it can run on an audio thread without
 * causing garbage collection, before and after the JavaScript engine optimizes it. A part
 * of a block (a buffer of another length, and every Web Audio quantum with a module whose
 * engine block is larger than the quantum) is copied through a view of that part, which
 * allocates the view.
 */
export class WasmEngine<A extends Samples> {
  /** Number of output channels of the patch. */
  readonly channels: number;
  /**
   * The latency of its output stage, in frames (`engine_latency`): its render starts with
   * that many frames of silence; 0 for a raw stage. Fixed when it is created.
   */
  readonly latency: number;
  /**
   * What the arithmetic of the thread that created the engine does with subnormals, as the
   * module observed it then (`engine_subnormals`): on an audio thread, whether the
   * browser's flush mode reaches the engine.
   */
  readonly subnormals: SubnormalArithmetic;
  /** The module instance. */
  readonly #exports: Exports;
  /** Frames per block. */
  readonly #blockSize: number;
  /** How its samples are laid out in the module memory. */
  readonly #layout: Layout;
  /** A view of the module memory as samples, renewed when the memory grows. */
  #memory: Samples;
  /** A view of the module memory as bytes, renewed when the memory grows: the report of the output stage is read through it. */
  #bytes: DataView;
  /**
   * Index in {@link WasmEngine.#memory} of the current block, planar; -1 before the first,
   * so that it builds the views. Always a small integer: a field once holding a fraction
   * (`NaN`) stores numbers boxed, and reading it allocates until the code is optimized.
   */
  #base = -1;
  /**
   * One view per patch channel over the current block, renewed only when the block moves
   * or the memory grows: copying a whole block through them allocates nothing.
   */
  readonly #views: Samples[] = [];
  /** Next frame of the current block to output; `#blockSize` when it is used up. */
  #position: number;
  /** Number of blocks rendered so far. */
  #blocks = 0;
  /** Whether the module failed (a trap, a wrong result): it is never called again. */
  #dead = false;
  /** Length of every output of the current render, in frames. */
  #frames = 0;
  /** Frames of the current render already written. */
  #written = 0;
  /** Frames of the current chunk: copied from the current block, from {@link WasmEngine.#position}. */
  #count = 0;

  /**
   * Checks the length of one output of the current render. A method made once, for
   * `forEach`: an indexed loop would read the outputs as possibly missing, and an
   * iterator allocates until the code is optimized.
   * @throws {RangeError} when it differs from the length of the first output.
   */
  readonly #checkLength = (output: A): void => {
    if (output.length !== this.#frames) {
      throw new RangeError(`every output must have the same length: ${String(output.length)} instead of ${String(this.#frames)}`);
    }
  };

  /**
   * Copies the current chunk into one output (see {@link WasmEngine.#checkLength} for
   * why it is made once): silence beyond the patch channels; a whole block in one copy,
   * which reads no sample as a number and so allocates nothing; a part of a block in one
   * copy through a view of that part, bit for bit too.
   */
  readonly #copyChunk = (output: A, channel: number): void => {
    const view = this.#views[channel];
    if (view === undefined) {
      output.fill(0, this.#written, this.#written + this.#count);
    } else if (this.#count === this.#blockSize) {
      output.set(view, this.#written);
    } else {
      // A part of the block, through a view of that part: copied bit for bit, as `set`
      // copies between arrays of one type (a NaN payload included), where reading samples
      // as numbers could change a NaN.
      output.set(view.subarray(this.#position, this.#position + this.#count), this.#written);
    }
  };

  /**
   * Wraps an instance with a patch loaded.
   * @param exports The instance, with a patch loaded.
   * @param layout How its samples are laid out.
   * @throws {ModuleError} when its blocks have no frame (rendering would never end) or more
   * than the largest engine block; or when
   * its observation of subnormals is not a code of `engine_subnormals`.
   */
  private constructor(exports: Exports, layout: Layout) {
    this.#exports = exports;
    this.#layout = layout;
    this.channels = exports.channels();
    const code = exports.subnormals();
    const subnormals = subnormalArithmetic(code);
    if (subnormals === undefined) {
      throw new ModuleError(`engine_subnormals returned ${String(code)}, not a code from 0 to ${String(SUBNORMAL_CODES)}`);
    }
    this.subnormals = subnormals;
    this.latency = exports.latency();
    // Read unsigned (`readExports`), so never negative: from 1 frame, or rendering would
    // never end, to the largest engine block. The channels need no such check: the views
    // over them are made once `#next` has found their blocks inside the memory.
    this.#blockSize = exports.blockSize();
    if (this.#blockSize === 0 || this.#blockSize > MAX_ENGINE_BLOCK) {
      throw new ModuleError(`engine_block_size returned ${String(this.#blockSize)}, not 1 to ${String(MAX_ENGINE_BLOCK)} frames`);
    }
    this.#memory = layout.view(exports.memory.buffer);
    this.#bytes = new DataView(exports.memory.buffer);
    this.#position = this.#blockSize;
  }

  /**
   * Instantiates `module` and loads `patch` (UTF-8 text in the patch format) at `rate`
   * hertz, rendering in `format` through the output stage `output` (raw by default,
   * {@link RAW_OUTPUT}): binary64 renders into `Float64Array`s, the two binary32 formats into
   * `Float32Array`s.
   * @throws {RangeError} when `rate` is not a whole number of hertz that fits 32 bits.
   * @throws {ModuleError} when `module` is not an engine module of this version, gives
   * blocks of 0 frames or more than the largest engine block, observes subnormals in a code that is not one, or answers the
   * output stage or the load with a result other than 1 (accepted)
   * or 0 (refused), or places the explanation of a refusal outside its memory.
   * @throws {OutputError} when the engine refuses the output stage: a limiter setting out of
   * range, a maximum lookahead out of range or below the lookahead.
   * @throws {PatchError} when the engine rejects the patch, its bytes or the rate; a patch
   * longer than the engine reads is refused before it is copied into the module, whose
   * memory never shrinks again, with the words of the engine.
   * @throws {EngineTrap} when the module traps.
   */
  static create(module: WebAssembly.Module, patch: Uint8Array, rate: number, format: "binary64", output?: Output): WasmEngine<Float64Array>;
  static create(module: WebAssembly.Module, patch: Uint8Array, rate: number, format: Binary32Format, output?: Output): WasmEngine<Float32Array>;
  static create(module: WebAssembly.Module, patch: Uint8Array, rate: number, format: RenderFormat, output: Output = RAW_OUTPUT): WasmEngine<Float64Array> | WasmEngine<Float32Array> {
    if (!Number.isInteger(rate) || rate < 0 || rate > 0xff_ff_ff_ff) {
      throw new RangeError(`the sample rate must be a whole number of hertz, not ${String(rate)}`);
    }
    const exports = readExports(instantiate(module));
    return guarded(() => {
      const max = exports.maxPatchBytes();
      if (patch.length > max) {
        throw new PatchError(ascii(`patch too large: ${String(patch.length)} bytes (at most ${String(max)})`));
      }
      // A fresh instance builds a raw stage: only a limited one is set, with its limiter's
      // settings, checked against its maximum in the same call.
      if (output.mode !== "raw" && !outputSet(exports, output)) {
        throw new OutputError(explanation(exports));
      }
      // Every call may grow the memory, which detaches its previous buffer: call first,
      // then read `memory.buffer`.
      const input = exports.input(patch.length);
      new Uint8Array(exports.memory.buffer, input, patch.length).set(patch);
      if (!succeeded("engine_load", exports.load(rate, RENDER_FORMATS.indexOf(format)))) {
        throw new PatchError(explanation(exports));
      }
      return format === "binary64" ? new WasmEngine<Float64Array>(exports, BINARY64) : new WasmEngine<Float32Array>(exports, BINARY32);
    });
  }

  /**
   * Changes the settings of its limiter, live: its ceiling, release and adaptivity from the
   * next frame, its lookahead held pending until the limiter's next rest, where it takes it
   * bit for bit as a limiter always at it (under continuous limiting it waits).
   * @throws {OutputError} when the engine refuses them (a value out of range, a lookahead
   * above the maximum it was created with, a raw stage without a limiter); the engine
   * renders on with the settings it had.
   * @throws {ModuleError} when the module answers neither 1 nor 0, or places its explanation
   * outside its memory; the engine is then dead.
   * @throws {EngineTrap} when the module traps, or failed before.
   */
  setLimiter(settings: LimiterSettings): void {
    this.#settle(this.#call(() => limiterSet(this.#exports, settings)));
  }

  /**
   * Starts the crossfade of its limiter toward protected (`true`) or raw (`false`): only a
   * switchable stage goes raw, a protected one staying protected.
   * @throws {OutputError} when its stage is raw, without a limiter, or protected and told to
   * go raw; the engine renders on as it was.
   * @throws {ModuleError} when the module answers neither 1 nor 0, or places its explanation
   * outside its memory; the engine is then dead.
   * @throws {EngineTrap} when the module traps, or failed before.
   */
  setProtected(protect: boolean): void {
    this.#settle(this.#call(() => succeeded("engine_protect", this.#exports.protect(protect ? 1 : 0))));
  }

  /**
   * Throws the engine's refusal of an output call that did not succeed (`done` false) as an
   * {@link OutputError}, its explanation read as every other call of a live module is
   * ({@link WasmEngine.#call}).
   * @throws {ModuleError} when the explanation lies outside the module memory; the engine
   * is then dead.
   * @throws {EngineTrap} when the module traps.
   */
  #settle(done: boolean): void {
    if (!done) {
      throw new OutputError(this.#call(() => explanation(this.#exports)));
    }
  }

  /** Number of frames of every block the module renders (`engine_block_size`), read once when the engine is created. */
  get blockSize(): number {
    return this.#blockSize;
  }

  /**
   * Number of blocks rendered so far. The engine never renders ahead: a block is rendered
   * only when its first frame is output, so that it never depends on how the host splits
   * time.
   */
  get blocks(): number {
    return this.#blocks;
  }

  /**
   * Renders the next frames into `outputs`, one array per channel, all of the same
   * length. Arrays beyond the patch channels receive silence; patch channels beyond
   * the arrays are dropped. Allocates nothing when every chunk is a whole block (see
   * {@link WasmEngine}).
   * @throws {RangeError} when the arrays differ in length (nothing is rendered).
   * @throws {ModuleError} when the module places a block outside its memory; the engine
   * is then dead.
   * @throws {EngineTrap} when the module traps, or failed before.
   */
  render(outputs: readonly A[]): void {
    if (this.#dead) {
      throw new EngineTrap();
    }
    this.#frames = outputs[0]?.length ?? 0;
    outputs.forEach(this.#checkLength);
    this.#written = 0;
    while (this.#written !== this.#frames) {
      if (this.#position === this.#blockSize) {
        this.#next();
      }
      this.#count = Math.min(this.#frames - this.#written, this.#blockSize - this.#position);
      outputs.forEach(this.#copyChunk);
      this.#written += this.#count;
      this.#position += this.#count;
    }
  }

  /**
   * Whether its limiter has a lookahead pending (`engine_lookahead_pending`): one set while
   * the limiter limits takes effect at its next rest, bit for bit as a limiter always at it
   * (`spec/engine-design.md`, decision 12), and is pending until then; false when it is in
   * force, and for a raw stage. Reading it allocates nothing.
   * @throws {ModuleError} when the module answers neither 1 nor 0; the engine is then dead.
   * @throws {EngineTrap} when the module traps, or failed before.
   */
  get lookaheadPending(): boolean {
    const answer = this.#call(this.#exports.lookaheadPending);
    if (answer !== 0 && answer !== 1) {
      this.#dead = true;
      throw new ModuleError(`engine_lookahead_pending returned ${String(answer)}, not 1 (pending) or 0 (in force)`);
    }
    return answer === 1;
  }

  /**
   * How many non-finite samples the output stage replaced since the load, saturating at
   * 2^32 - 1 (the first word of `engine_replaced`). Reading it allocates nothing: the
   * audio thread reads it after every block.
   * @throws {ModuleError} when the module places its report outside its memory; the
   * engine is then dead.
   * @throws {EngineTrap} when the module traps, or failed before.
   */
  get replacedCount(): number {
    // The address first: the call may grow the memory, which renews the view.
    const at = this.#report();
    return this.#bytes.getUint32(at, true);
  }

  /**
   * What the output stage replaced since the load: the count and the first sample, read
   * from one call of `engine_replaced`. Allocates its result when something was replaced.
   * @throws {ModuleError} when the module places its report outside its memory.
   * @throws {EngineTrap} when the module traps, or failed before.
   */
  replaced(): Replaced {
    const at = this.#report();
    const count = this.#bytes.getUint32(at, true);
    return count === 0 ? NOTHING_REPLACED : { count, first: this.#first(at) };
  }

  /**
   * The first sample the output stage replaced, as the module reports it (the words of
   * `engine_replaced` after the count): its frame, from its low then its high 32 bits, and
   * its channel; frame 0 of channel 0 while none was (read
   * {@link WasmEngine.replacedCount} first).
   * @throws {ModuleError} when the module places its report outside its memory.
   * @throws {EngineTrap} when the module traps, or failed before.
   */
  replacedFirst(): FirstReplaced {
    return this.#first(this.#report());
  }

  /** The first sample replaced, from the report at byte address `at` ({@link WasmEngine.replacedFirst}). */
  #first(at: number): FirstReplaced {
    const frame = this.#bytes.getUint32(at + 8, true) * 2 ** 32 + this.#bytes.getUint32(at + 4, true);
    return { frame, channel: this.#bytes.getUint32(at + 12, true) };
  }

  /**
   * Where the module reports what its output stage replaced (`engine_replaced`): the byte
   * address of four aligned words, checked to lie in its memory, whose view is renewed when
   * it grew.
   * @throws {ModuleError} when the address is unaligned or the words are outside the
   * memory; the engine is then dead.
   * @throws {EngineTrap} when the module traps, or failed before.
   */
  #report(): number {
    // Read unsigned, as every export (`readExports`): an address past 2^31 is never negative.
    const pointer = this.#call(this.#exports.replaced);
    const buffer = this.#exports.memory.buffer;
    if (this.#bytes.buffer !== buffer) {
      this.#bytes = new DataView(buffer);
    }
    if (pointer % 4 !== 0 || pointer + 16 > buffer.byteLength) {
      this.#dead = true;
      throw new ModuleError(`engine_replaced returned the address ${String(pointer)}, unaligned or outside the memory`);
    }
    return pointer;
  }

  /**
   * Calls one exported function of a live module, through `exported`: a module that
   * failed is never called again, and a trap or a {@link ModuleError} of `exported` (an
   * answer outside the interface) kills it.
   * @throws {EngineTrap} when the module traps, or failed before.
   * @throws {ModuleError} what `exported` throws when the module breaks the interface.
   */
  #call<T>(exported: () => T): T {
    if (this.#dead) {
      throw new EngineTrap();
    }
    try {
      return exported();
    } catch (error) {
      this.#dead = true;
      throw error instanceof WebAssembly.RuntimeError ? new EngineTrap({ cause: error }) : error;
    }
  }

  /**
   * Renders the next block, in the module memory, and checks where the module put it: a
   * block the views cannot cover entirely is a module error, never read as silence.
   */
  #next(): void {
    let pointer: number;
    try {
      pointer = this.#exports.process();
    } catch (error) {
      this.#dead = true;
      throw error instanceof WebAssembly.RuntimeError ? new EngineTrap({ cause: error }) : error;
    }
    const buffer = this.#exports.memory.buffer;
    const bytes = this.#layout.bytes;
    const base = pointer / bytes;
    if (!Number.isInteger(base) || base + this.channels * this.#blockSize > buffer.byteLength / bytes) {
      this.#dead = true;
      throw new ModuleError(`engine_process returned the block address ${String(pointer)}, unaligned or outside the memory`);
    }
    if (this.#memory.buffer !== buffer || base !== this.#base) {
      if (this.#memory.buffer !== buffer) {
        this.#memory = this.#layout.view(buffer);
      }
      for (let channel = 0; channel !== this.channels; channel += 1) {
        const start = base + channel * this.#blockSize;
        this.#views[channel] = this.#memory.subarray(start, start + this.#blockSize);
      }
      this.#base = base;
    }
    this.#position = 0;
    this.#blocks += 1;
  }
}
