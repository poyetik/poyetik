// Driving an instrumented engine module through its exported interface (bindings/wasm/src/
// ffi.rs), and reading how many WebAssembly instructions each call executed. The
// instrumented module computes exactly what the module computes (test/benchmarks/counter.test.ts
// renders every conformance case through both): only the counter is added.
import { ABI_VERSION, RENDER_FORMATS, type RenderFormat } from "../ts/engine.ts";
import { COUNTER, instrument } from "./instrument.ts";

/** Thrown when the instrumented module is not an engine module, or refuses a patch. */
export class CountError extends Error {
  /**
   * Describes what failed.
   * @param message What failed.
   * @param options The cause, when the failure comes from another error.
   */
  constructor(message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = "CountError";
  }
}

/** Reads an exported function of `exports`, by name. */
function exportedFunction(exports: WebAssembly.Exports, name: string): (...args: number[]) => number {
  const value = exports[name];
  if (typeof value !== "function") {
    throw new CountError(`the module has no function ${name}`);
  }
  return (...args) => Number(Reflect.apply(value, undefined, args)) >>> 0;
}

/** One instance of the instrumented engine module, with the counter it increments. */
export class CountedInstance {
  /** The counter of executed instructions. */
  readonly #counter: WebAssembly.Global;
  /** The module memory. */
  readonly #memory: WebAssembly.Memory;
  /** `engine_input(len)`. */
  readonly #input: (len: number) => number;
  /** `engine_load(rate, format)`. */
  readonly #load: (rate: number, format: number) => number;
  /** `engine_message_ptr()`. */
  readonly #messagePtr: () => number;
  /** `engine_message_len()`. */
  readonly #messageLen: () => number;
  /** `engine_process()`. */
  readonly #process: () => number;

  /**
   * Instantiates the instrumented module and checks its interface version.
   * @param module The instrumented, compiled engine module.
   * @throws {CountError} when it is not an instrumented engine module of this version.
   */
  constructor(module: WebAssembly.Module) {
    const { exports } = new WebAssembly.Instance(module, {});
    const counter = exports[COUNTER];
    const memory = exports["memory"];
    if (!(counter instanceof WebAssembly.Global) || !(memory instanceof WebAssembly.Memory)) {
      throw new CountError(`the module does not export its memory and its counter "${COUNTER}"`);
    }
    const version = exportedFunction(exports, "engine_abi_version")();
    if (version !== ABI_VERSION) {
      throw new CountError(`the module has the interface 0x${version.toString(16)}, expected 0x${ABI_VERSION.toString(16)}`);
    }
    this.#counter = counter;
    this.#memory = memory;
    this.#input = exportedFunction(exports, "engine_input");
    this.#load = exportedFunction(exports, "engine_load");
    this.#messagePtr = exportedFunction(exports, "engine_message_ptr");
    this.#messageLen = exportedFunction(exports, "engine_message_len");
    this.#process = exportedFunction(exports, "engine_process");
  }

  /** The instructions `action` executes in the module. */
  #counted(action: () => void): bigint {
    const before = this.#instructions();
    action();
    return this.#instructions() - before;
  }

  /** The counter's value. */
  #instructions(): bigint {
    const value: unknown = this.#counter.value;
    if (typeof value !== "bigint") {
      throw new CountError(`the counter "${COUNTER}" is not an i64`);
    }
    return value;
  }

  /**
   * Loads `patch` at `rate` hertz to render in `format` (parsing it and building its plan)
   * and returns the instructions the load executed, copying the patch into the module
   * included.
   * @throws {CountError} with the engine's explanation when it refuses the patch.
   */
  load(patch: Uint8Array, rate: number, format: RenderFormat): bigint {
    let loaded = 0;
    const instructions = this.#counted(() => {
      const address = this.#input(patch.length);
      new Uint8Array(this.#memory.buffer, address, patch.length).set(patch);
      loaded = this.#load(rate, RENDER_FORMATS.indexOf(format));
    });
    if (loaded !== 1) {
      const message = new Uint8Array(this.#memory.buffer, this.#messagePtr(), this.#messageLen());
      throw new CountError(`the engine refused the patch: ${new TextDecoder().decode(message)}`);
    }
    return instructions;
  }

  /** Renders `blocks` blocks and returns the instructions they executed. */
  process(blocks: number): bigint {
    return this.#counted(() => {
      for (let block = 0; block !== blocks; block += 1) {
        this.#process();
      }
    });
  }
}

/**
 * Instruments the engine module `wasm` and compiles it.
 * @throws {InstrumentError} when it cannot be instrumented.
 */
export function countedModule(wasm: Uint8Array): WebAssembly.Module {
  return new WebAssembly.Module(instrument(wasm));
}

/** An engine module as the benchmarks measure it: instrumented, and the size of its bytes. */
export interface CountedEngine {
  /** The engine module, instrumented and compiled. */
  readonly module: WebAssembly.Module;
  /** The size of the engine module as built (before instrumentation), in bytes. */
  readonly bytes: number;
}

/**
 * The engine module `wasm`, instrumented, with its size: both from the same bytes.
 * @throws {InstrumentError} when it cannot be instrumented.
 */
export function countedEngine(wasm: Uint8Array): CountedEngine {
  return { module: countedModule(wasm), bytes: wasm.length };
}
