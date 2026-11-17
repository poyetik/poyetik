// Reading the exported interface of the engine module (bindings/audio-wasm/src/ffi.rs): its
// memory, its version and each function with its arity, checked once. The one reader of
// that interface in TypeScript: the engine (engine.ts) and the benchmarks' counted
// instance (benchmarks/counter.ts) both drive the module through it, so that a change of
// the interface fails both the same way.

/** A function exported by the module, without arguments. */
type Exported0 = () => number;

/** A function exported by the module, with one argument. */
type Exported1 = (value: number) => number;

/** A function exported by the module, with two arguments. */
type Exported2 = (first: number, second: number) => number;

/** A function exported by the module, with four arguments. */
type Exported4 = (first: number, second: number, third: number, fourth: number) => number;

/** A function exported by the module, with seven arguments. */
type Exported7 = (first: number, second: number, third: number, fourth: number, fifth: number, sixth: number, seventh: number) => number;

/** The exports of the engine module. */
export interface Exports {
  /** The module memory: every address the functions return points into it, and it may grow at any call. */
  readonly memory: WebAssembly.Memory;
  /** `engine_input(len)`: makes the input buffer `len` bytes long and returns its address. */
  readonly input: Exported1;
  /** `engine_load(rate, format)`: loads the patch in the input buffer at `rate` hertz, rendering in the format of code `format` (its index in `RENDER_FORMATS` of `engine.ts`); 1 on success, 0 on failure. */
  readonly load: Exported2;
  /** `engine_message_ptr()`: the address of the UTF-8 explanation of the last failed call. */
  readonly messagePtr: Exported0;
  /** `engine_message_len()`: the length in bytes of that explanation; 0 after a success. */
  readonly messageLen: Exported0;
  /** `engine_channels()`: the number of output channels of the loaded patch. */
  readonly channels: Exported0;
  /** `engine_block_size()`: the number of frames of every block the engine renders. */
  readonly blockSize: Exported0;
  /** `engine_process()`: renders the next block and returns the byte address of its planar samples. */
  readonly process: Exported0;
  /** `engine_max_patch_bytes()`: the longest patch the engine reads, in bytes; a longer one is refused before it is copied in. */
  readonly maxPatchBytes: Exported0;
  /** `engine_subnormals()`: what the arithmetic of the calling thread does with subnormals, observed now, as four bits (see `subnormalArithmetic` in `engine.ts`). */
  readonly subnormals: Exported0;
  /** `engine_replaced()`: the byte address of four little-endian `u32` words, what the output stage replaced since the load: the count (saturating at 2^32 - 1), the low then the high 32 bits of the frame of the first, its channel; zeros while none was. */
  readonly replaced: Exported0;
  /** `engine_output(mode, maximum_lookahead, detector, ceiling, lookahead, release, adaptivity)`: sets the output stage every later load builds (a fresh instance builds a raw one, which is therefore never set), the mode of code `mode` (its index in `OUTPUT_MODES` of `engine.ts`), and for a limited one the maximum lookahead in milliseconds, the detector of code `detector` (its index in `DETECTORS`) and the limiter's settings, the lookahead checked against the maximum in the same call; 1 on success, 0 on failure. */
  readonly output: Exported7;
  /** `engine_limiter(ceiling, lookahead, release, adaptivity)`: changes the loaded engine's limiter, live only (refused when none is loaded or its stage is raw, and for a lookahead above its maximum); 1 on success, 0 on failure. */
  readonly limiter: Exported4;
  /** `engine_protect(protected)`: switches the loaded engine's limiter toward protected (1) or raw (0), by a crossfade (refused when none is loaded or its stage is raw, when its stage is protected and told raw, and for a code other than 0 or 1); 1 on success, 0 on failure. */
  readonly protect: Exported1;
  /** `engine_latency()`: the latency of the loaded engine's output stage, in frames; 0 when it is raw. */
  readonly latency: Exported0;
}

/**
 * The identity and version of the exported interface this code drives (`ABI_VERSION` in
 * `ffi.rs`): "PY" in the high 16 bits, version 5 in the low 16 bits.
 */
export const ABI_VERSION = 0x5059_0005;

/** Thrown when the module does not have the expected interface. */
export class ModuleError extends Error {
  /**
   * Describes a module that is not an engine module.
   * @param detail What is wrong with the module.
   * @param options The underlying error, if any.
   */
  constructor(detail: string, options?: ErrorOptions) {
    super(`not an engine module: ${detail}`, options);
    this.name = "ModuleError";
  }
}

/** The arguments of a function called without any: one shared, frozen, empty list. */
// Stryker disable next-line ArrayDeclaration: the functions called with it take no argument, and WebAssembly ignores extra ones
const NO_ARGUMENTS: readonly number[] = Object.freeze([]);

/**
 * Reads the result of an exported function. Every export returns an unsigned 32-bit
 * value (addresses included), which JavaScript receives as a signed `i32`: values from
 * 2^31 on come back negative and are read back as unsigned.
 */
function unsigned(name: string, result: unknown): number {
  if (typeof result !== "number") {
    throw new ModuleError(`${name} did not return a number`);
  }
  return result >>> 0;
}

/** Reads one exported function taking `arity` arguments: a caller, given the arguments. */
function exportedFunction(exports: WebAssembly.Exports, name: string, arity: number): (args: readonly number[]) => number {
  const value = exports[name];
  if (typeof value !== "function") {
    throw new ModuleError(`no function ${name}`);
  }
  if (value.length !== arity) {
    throw new ModuleError(`${name} takes ${String(value.length)} argument(s), expected ${String(arity)}`);
  }
  return (args) => unsigned(name, Reflect.apply(value, undefined, args));
}

/** Reads one exported function without arguments. Calling it does not allocate. */
function exported0(exports: WebAssembly.Exports, name: string): Exported0 {
  const call = exportedFunction(exports, name, 0);
  return () => call(NO_ARGUMENTS);
}

/** Reads one exported function with one argument. */
function exported1(exports: WebAssembly.Exports, name: string): Exported1 {
  const call = exportedFunction(exports, name, 1);
  return (argument) => call([argument]);
}

/** Reads one exported function with two arguments. */
function exported2(exports: WebAssembly.Exports, name: string): Exported2 {
  const call = exportedFunction(exports, name, 2);
  return (first, second) => call([first, second]);
}

/**
 * Reads the result of a call of `name` that succeeds or is refused (`engine_load`,
 * `engine_output`, `engine_limiter`, `engine_protect`): true when it succeeded (1), false
 * when the engine refused it (0), its explanation then in `engine_message_ptr` and
 * `engine_message_len`.
 * @throws {ModuleError} for any other result: the module breaks the interface, and its
 * message is never taken for a refusal.
 */
export function succeeded(name: string, result: number): boolean {
  if (result !== 0 && result !== 1) {
    throw new ModuleError(`${name} returned ${String(result)}, not 1 (succeeded) or 0 (refused)`);
  }
  return result === 1;
}

/**
 * A copy of the engine's explanation of its last refusal (`engine_message_ptr`,
 * `engine_message_len`), in UTF-8: it does not keep the module memory alive.
 * @throws {ModuleError} when the explanation lies outside the module memory: the module
 * breaks the interface.
 */
export function explanation(exports: Exports): Uint8Array {
  // The length first, the address last: every address is valid only until the next call
  // (the protocol, `ffi.rs`), so none comes between it and the read.
  const length = exports.messageLen();
  const pointer = exports.messagePtr();
  const buffer = exports.memory.buffer;
  if (pointer + length > buffer.byteLength) {
    throw new ModuleError(`engine_message_ptr returned ${String(pointer)} and engine_message_len ${String(length)}: an explanation outside the memory`);
  }
  return new Uint8Array(buffer, pointer, length).slice();
}

/** Reads one exported function with four arguments. */
function exported4(exports: WebAssembly.Exports, name: string): Exported4 {
  const call = exportedFunction(exports, name, 4);
  return (first, second, third, fourth) => call([first, second, third, fourth]);
}

/** Reads one exported function with seven arguments. */
function exported7(exports: WebAssembly.Exports, name: string): Exported7 {
  const call = exportedFunction(exports, name, 7);
  return (first, second, third, fourth, fifth, sixth, seventh) => call([first, second, third, fourth, fifth, sixth, seventh]);
}

/** Reads and checks the exports of an engine module instance, version first. */
export function readExports(exports: WebAssembly.Exports): Exports {
  const memory = exports["memory"];
  if (!(memory instanceof WebAssembly.Memory)) {
    throw new ModuleError("no memory");
  }
  const version = exported0(exports, "engine_abi_version")();
  if (version !== ABI_VERSION) {
    throw new ModuleError(`interface 0x${version.toString(16)}, expected 0x${ABI_VERSION.toString(16)}`);
  }
  return {
    memory,
    input: exported1(exports, "engine_input"),
    load: exported2(exports, "engine_load"),
    messagePtr: exported0(exports, "engine_message_ptr"),
    messageLen: exported0(exports, "engine_message_len"),
    channels: exported0(exports, "engine_channels"),
    blockSize: exported0(exports, "engine_block_size"),
    process: exported0(exports, "engine_process"),
    maxPatchBytes: exported0(exports, "engine_max_patch_bytes"),
    subnormals: exported0(exports, "engine_subnormals"),
    replaced: exported0(exports, "engine_replaced"),
    output: exported7(exports, "engine_output"),
    limiter: exported4(exports, "engine_limiter"),
    protect: exported1(exports, "engine_protect"),
    latency: exported0(exports, "engine_latency"),
  };
}
