// Driving an instrumented engine module through its exported interface (bindings/audio-wasm/src/
// ffi.rs), and reading how many WebAssembly instructions each call executed. The
// instrumented module computes exactly what the module computes (test/benchmarks/counter.test.ts
// renders every conformance case through both): only the counter is added.
import { explanation, readExports, succeeded, type Exports } from "../ts/abi.ts";
import { RENDER_FORMATS, type RenderFormat } from "../ts/engine.ts";
import { COUNTER, instrument } from "./instrument.ts";

/** Thrown when the instrumented module has no counter, or refuses a patch. */
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

/** One instance of the instrumented engine module, with the counter it increments. */
export class CountedInstance {
  /** The counter of executed instructions. */
  readonly #counter: WebAssembly.Global;
  /** The engine's exports, read and checked as the engine reads them. */
  readonly #exports: Exports;

  /**
   * Instantiates the instrumented module and reads its interface as the engine does
   * (`readExports`: its version, and each function with its arity).
   * @param module The instrumented, compiled engine module.
   * @throws {CountError} when it does not export its counter.
   * @throws {ModuleError} when it is not an engine module of this interface.
   */
  constructor(module: WebAssembly.Module) {
    const { exports } = new WebAssembly.Instance(module, {});
    const counter = exports[COUNTER];
    if (!(counter instanceof WebAssembly.Global)) {
      throw new CountError(`the module does not export its counter "${COUNTER}"`);
    }
    this.#counter = counter;
    this.#exports = readExports(exports);
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
   * and returns the WebAssembly instructions `engine_input` and `engine_load` executed (the
   * copy of the patch into the module memory is JavaScript, not counted).
   * @throws {CountError} with the engine's explanation when it refuses the patch.
   * @throws {ModuleError} when `engine_load` returns neither 1 nor 0.
   */
  load(patch: Uint8Array, rate: number, format: RenderFormat): bigint {
    let loaded = 0;
    const instructions = this.#counted(() => {
      const address = this.#exports.input(patch.length);
      new Uint8Array(this.#exports.memory.buffer, address, patch.length).set(patch);
      loaded = this.#exports.load(rate, RENDER_FORMATS.indexOf(format));
    });
    if (!succeeded("engine_load", loaded)) {
      throw new CountError(`the engine refused the patch: ${new TextDecoder().decode(explanation(this.#exports))}`);
    }
    return instructions;
  }

  /** Renders `blocks` blocks and returns the instructions they executed. */
  process(blocks: number): bigint {
    return this.#counted(() => {
      for (let block = 0; block !== blocks; block += 1) {
        this.#exports.process();
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
