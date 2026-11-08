import { describe, expect, test } from "vitest";
import { ENGINE_PROCESSOR, MessageError, OptionsError, TAP_PROCESSOR, parseEngineMessage, parseEngineOptions, parseTapMessage, SOURCE_STOPPED } from "../ts/protocol.ts";

describe("parseEngineOptions", () => {
  test("accepts the module bytes, the patch and the marker choice", () => {
    const wasm = new ArrayBuffer(8);
    const patch = new Uint8Array([1]);
    expect(parseEngineOptions({ wasm, patch, marker: true, extra: 1 })).toStrictEqual({ wasm, patch, marker: true });
    expect(parseEngineOptions({ wasm, patch, marker: false })).toStrictEqual({ wasm, patch, marker: false });
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
  test("accepts the three messages of the engine processor", () => {
    const detail = new Uint8Array([1]);
    expect(parseEngineMessage({ type: "ready" })).toStrictEqual({ type: "ready" });
    expect(parseEngineMessage({ type: "rejected", detail })).toStrictEqual({ type: "rejected", detail });
    expect(parseEngineMessage({ type: "failed", message: "x" })).toStrictEqual({ type: "failed", message: "x" });
  });

  test.each([
    [null, "engine message is not an object"],
    ["ready", "engine message is not an object"],
    [{ type: "rejected", detail: [1] }, "rejection detail is not a Uint8Array"],
    [{ type: "failed", message: 1 }, "failure message is not a string"],
    [{ type: "other" }, "unknown engine message"],
    [{}, "unknown engine message"],
  ])("rejects %o", (value, detail) => {
    expect(() => parseEngineMessage(value)).toThrow(new MessageError(detail));
  });

  test("names its errors", () => {
    expect(new MessageError("x")).toMatchObject({ name: "MessageError", message: "invalid message from the processor: x" });
  });
});
