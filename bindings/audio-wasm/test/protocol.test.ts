import { describe, expect, test } from "vitest";
import { IEEE_SUBNORMALS, READY } from "./fixtures.ts";
import { CommandError, ENGINE_PROCESSOR, MessageError, OptionsError, TAP_PROCESSOR, parseEngineCommand, parseEngineMessage, parseEngineOptions, parseTapMessage, SOURCE_STOPPED } from "../ts/protocol.ts";

describe("parseEngineOptions", () => {
  /** An output stage of the options, every field differing from the defaults. */
  const OUTPUT = { mode: "switchable", maximumLookahead: 2, detector: "mastering", limiter: { ceiling: -3, lookahead: 1, release: 50, adaptivity: 0.25 } } as const;

  test("accepts the module bytes, the patch, the marker choice, the precision and the output stage", () => {
    const wasm = new ArrayBuffer(8);
    const patch = new Uint8Array([1]);
    expect(parseEngineOptions({ wasm, patch, marker: true, precision: "binary32", output: { ...OUTPUT, extra: 1 }, extra: 1 })).toStrictEqual({ wasm, patch, marker: true, precision: "binary32", output: OUTPUT });
    for (const mode of ["protected", "switchable"] as const) {
      expect(parseEngineOptions({ wasm, patch, marker: false, precision: "binary64", output: { ...OUTPUT, mode } })).toStrictEqual({ wasm, patch, marker: false, precision: "binary64", output: { ...OUTPUT, mode } });
    }
  });

  test("reads a raw stage as its mode only: it has no limiter, so no other field is read", () => {
    const options = (output: unknown): unknown => ({ wasm: new ArrayBuffer(0), patch: new Uint8Array(), marker: false, precision: "binary64", output });
    for (const output of [{ mode: "raw" }, { ...OUTPUT, mode: "raw" }, { mode: "raw", maximumLookahead: "5", detector: "exact", limiter: null }]) {
      const read = parseEngineOptions(options(output));
      expect(read.output).toStrictEqual({ mode: "raw" });
      expect(Object.isFrozen(read.output)).toBe(true);
    }
  });

  /** Valid options but for `output`. */
  const withOutput = (output: unknown): unknown => ({ wasm: new ArrayBuffer(0), patch: new Uint8Array(), marker: false, precision: "binary64", output });

  test.each<readonly [unknown, string]>([
    [undefined, "output is not an object"],
    [null, "output is not an object"],
    [{ ...OUTPUT, mode: "loud" }, "output mode is not one of raw, protected, switchable"],
    [{ ...OUTPUT, mode: 1 }, "output mode is not one of raw, protected, switchable"],
    [{ ...OUTPUT, maximumLookahead: "5" }, "output maximumLookahead is not a number"],
    [{ ...OUTPUT, detector: "exact" }, "output detector is not one of standard, precise, mastering"],
    [{ ...OUTPUT, detector: undefined }, "output detector is not one of standard, precise, mastering"],
    [{ ...OUTPUT, limiter: null }, "output limiter is not an object"],
    [{ ...OUTPUT, limiter: 1 }, "output limiter is not an object"],
    ...(["ceiling", "lookahead", "release", "adaptivity"] as const).map((name): readonly [unknown, string] => [{ ...OUTPUT, limiter: { ...OUTPUT.limiter, [name]: "1" } }, `output limiter ${name} is not a number`]),
  ])("rejects the output stage %o", (output, detail) => {
    expect(() => parseEngineOptions(withOutput(output))).toThrow(new OptionsError(detail));
  });

  test.each([
    [undefined, "not an object"],
    [null, "not an object"],
    [3, "not an object"],
    [{ patch: new Uint8Array() }, "wasm is not an ArrayBuffer"],
    [{ wasm: new Uint8Array(), patch: new Uint8Array() }, "wasm is not an ArrayBuffer"],
    [{ wasm: new ArrayBuffer(0) }, "patch is not a Uint8Array"],
    [{ wasm: new ArrayBuffer(0), patch: [1] }, "patch is not a Uint8Array"],
    [{ wasm: new ArrayBuffer(0), patch: new Uint8Array() }, "marker is not a boolean"],
    [{ wasm: new ArrayBuffer(0), patch: new Uint8Array(), marker: 1 }, "marker is not a boolean"],
    [{ wasm: new ArrayBuffer(0), patch: new Uint8Array(), marker: false }, "precision is not one of binary64, binary32"],
    [{ wasm: new ArrayBuffer(0), patch: new Uint8Array(), marker: false, precision: "binary16" }, "precision is not one of binary64, binary32"],
    [{ wasm: new ArrayBuffer(0), patch: new Uint8Array(), marker: false, precision: 64 }, "precision is not one of binary64, binary32"],
  ])("rejects %o", (value, detail) => {
    expect(() => parseEngineOptions(value)).toThrow(new OptionsError(detail));
  });

  test("names its errors", () => {
    expect(new OptionsError("x")).toMatchObject({ name: "OptionsError", message: "invalid processor options: x" });
  });
});

describe("parseTapMessage", () => {
  test("accepts one Float32Array per channel", () => {
    const block = [new Float32Array(2), new Float32Array(2)];
    expect(parseTapMessage(block)).toStrictEqual({ type: "quantum", channels: block });
    expect(parseTapMessage([])).toStrictEqual({ type: "quantum", channels: [] });
  });

  test("accepts the source stopping", () => {
    expect(SOURCE_STOPPED).toBe("source stopped");
    expect(parseTapMessage("source stopped")).toStrictEqual({ type: "stopped" });
  });

  test("rejects anything else", () => {
    expect(() => parseTapMessage("stop")).toThrow(new MessageError("render quantum is not an array"));
    expect(() => parseTapMessage([new Float32Array(1), [0]])).toThrow(new MessageError("tap channel is not a Float32Array"));
  });
});

test("names the processors: a contract with every page that loads them", () => {
  expect(ENGINE_PROCESSOR).toBe("poyetik-engine");
  expect(TAP_PROCESSOR).toBe("poyetik-tap");
});

describe("parseEngineMessage", () => {
  test("accepts the four messages of the engine processor", () => {
    const detail = new Uint8Array([1]);
    expect(parseEngineMessage(READY)).toStrictEqual(READY);
    const mixed = { binary32: { operands: false, results: true }, binary64: { operands: true, results: false } };
    expect(parseEngineMessage({ type: "ready", subnormals: mixed })).toStrictEqual({ type: "ready", subnormals: mixed });
    expect(parseEngineMessage({ type: "rejected", detail })).toStrictEqual({ type: "rejected", detail });
    expect(parseEngineMessage({ type: "failed", message: "x" })).toStrictEqual({ type: "failed", message: "x" });
    for (const replaced of [
      { count: 1, first: { frame: 0, channel: 0 } },
      { count: Number.MAX_SAFE_INTEGER, first: { frame: Number.MAX_SAFE_INTEGER, channel: 3 } },
    ]) {
      expect(parseEngineMessage({ type: "replaced", replaced })).toStrictEqual({ type: "replaced", replaced });
    }
  });

  test.each([
    [null, "engine message is not an object"],
    ["ready", "engine message is not an object"],
    [{ type: "rejected", detail: [1] }, "rejection detail is not a Uint8Array"],
    [{ type: "failed", message: 1 }, "failure message is not a string"],
    [{ type: "other" }, "unknown engine message"],
    [{ type: "ready" }, "subnormal arithmetic is not an object"],
    [{ type: "ready", subnormals: null }, "subnormal arithmetic is not an object"],
    [{ type: "ready", subnormals: 15 }, "subnormal arithmetic is not an object"],
    [{ type: "ready", subnormals: { binary64: IEEE_SUBNORMALS.binary64 } }, "binary32 subnormals are not an object"],
    [{ type: "ready", subnormals: { binary32: IEEE_SUBNORMALS.binary32 } }, "binary64 subnormals are not an object"],
    [{ type: "ready", subnormals: { ...IEEE_SUBNORMALS, binary32: null } }, "binary32 subnormals are not an object"],
    [{ type: "ready", subnormals: { ...IEEE_SUBNORMALS, binary32: { operands: 1, results: true } } }, "binary32 subnormals are not two booleans"],
    [{ type: "ready", subnormals: { ...IEEE_SUBNORMALS, binary32: { operands: true } } }, "binary32 subnormals are not two booleans"],
    [{ type: "ready", subnormals: { ...IEEE_SUBNORMALS, binary64: { operands: true, results: "yes" } } }, "binary64 subnormals are not two booleans"],
    [{}, "unknown engine message"],
    [{ type: "replaced" }, "replaced samples are not an object"],
    [{ type: "replaced", replaced: null }, "replaced samples are not an object"],
    [{ type: "replaced", replaced: { count: 1 } }, "the first replaced sample is not an object"],
    [{ type: "replaced", replaced: { count: 1, first: null } }, "the first replaced sample is not an object"],
    [{ type: "replaced", replaced: { count: 0, first: { frame: 0, channel: 0 } } }, "the replaced count is not a whole number from 1"],
    [{ type: "replaced", replaced: { count: 1.5, first: { frame: 0, channel: 0 } } }, "the replaced count is not a whole number from 1"],
    [{ type: "replaced", replaced: { count: "1", first: { frame: 0, channel: 0 } } }, "the replaced count is not a whole number from 1"],
    [{ type: "replaced", replaced: { count: 2 ** 53, first: { frame: 0, channel: 0 } } }, "the replaced count is not a whole number from 1"],
    [{ type: "replaced", replaced: { count: 1, first: { frame: -1, channel: 0 } } }, "the first replaced frame is not a whole number from 0"],
    [{ type: "replaced", replaced: { count: 1, first: { frame: Number.NaN, channel: 0 } } }, "the first replaced frame is not a whole number from 0"],
    [{ type: "replaced", replaced: { count: 1, first: { frame: 0, channel: -1 } } }, "the first replaced channel is not a whole number from 0"],
    [{ type: "replaced", replaced: { count: 1, first: { frame: 0 } } }, "the first replaced channel is not a whole number from 0"],
  ])("rejects %o", (value, detail) => {
    expect(() => parseEngineMessage(value)).toThrow(new MessageError(detail));
  });

  test("names its errors", () => {
    expect(new MessageError("x")).toMatchObject({ name: "MessageError", message: "invalid message from the processor: x" });
  });
});

describe("parseEngineCommand", () => {
  test("accepts limiter settings and a protection", () => {
    const settings = { ceiling: -3, lookahead: 1, release: 50, adaptivity: 0.25 };
    expect(parseEngineCommand({ type: "limiter", settings: { ...settings, extra: 1 } })).toStrictEqual({ type: "limiter", settings });
    expect(parseEngineCommand({ type: "protect", protect: true })).toStrictEqual({ type: "protect", protect: true });
    expect(parseEngineCommand({ type: "protect", protect: false })).toStrictEqual({ type: "protect", protect: false });
  });

  test.each([
    [undefined, "not an object"],
    [null, "not an object"],
    ["protect", "not an object"],
    [{ type: "mute" }, "unknown command"],
    [{ type: "protect", protect: 1 }, "protect is not a boolean"],
    [{ type: "limiter" }, "limiter is not an object"],
    [{ type: "limiter", settings: { ceiling: -1, lookahead: 1.5, release: 100 } }, "limiter adaptivity is not a number"],
  ])("rejects %o", (value, detail) => {
    expect(() => parseEngineCommand(value)).toThrow(new CommandError(detail));
  });

  test("names its errors", () => {
    expect(new CommandError("x")).toMatchObject({ name: "CommandError", message: "invalid command to the engine processor: x" });
  });
});

describe("parseEngineMessage refusals", () => {
  test("accepts a refusal with its explanation as bytes, and rejects one without", () => {
    const detail = new Uint8Array([104, 105]);
    expect(parseEngineMessage({ type: "refused", detail })).toStrictEqual({ type: "refused", detail });
    expect(() => parseEngineMessage({ type: "refused", detail: "hi" })).toThrow(new MessageError("refusal detail is not a Uint8Array"));
    expect(() => parseEngineMessage({ type: "rejected", detail: "hi" })).toThrow(new MessageError("rejection detail is not a Uint8Array"));
  });
});
