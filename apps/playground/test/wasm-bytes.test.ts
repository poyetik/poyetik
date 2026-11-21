// The one classifier of downloaded WebAssembly bytes, shared by the square and the sound: bytes
// that do not start as a module are no module the server meant to send (a download's fault),
// and an error caused by the browser's `CompileError` is a module this browser cannot compile.
import { describe, expect, test } from "vitest";
import { cannotCompile, isWasmModule } from "../src/wasm-bytes.ts";

/** `bytes` as the `ArrayBuffer` a response's body gives. */
function body(...bytes: readonly number[]): ArrayBuffer {
  return new Uint8Array(bytes).buffer;
}

describe("isWasmModule", () => {
  test("reads a module by its magic alone, `\\0asm`, whatever version follows", () => {
    expect(isWasmModule(body(0x00, 0x61, 0x73, 0x6d)), "the magic alone").toBe(true);
    expect(isWasmModule(body(0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0xff)), "version 1, a bad body").toBe(true);
    expect(isWasmModule(body(0x00, 0x61, 0x73, 0x6d, 0x02, 0x00, 0x00, 0x00)), "another version").toBe(true);
  });

  test("refuses bytes that do not start with the magic: empty, short, others, a page of HTML", () => {
    expect(isWasmModule(body()), "empty").toBe(false);
    expect(isWasmModule(body(0x00, 0x61, 0x73)), "three bytes of the magic").toBe(false);
    expect(isWasmModule(body(1, 2, 3)), "three other bytes").toBe(false);
    expect(isWasmModule(body(0x00, 0x61, 0x73, 0x6e)), "the magic's last byte off").toBe(false);
    expect(isWasmModule(body(0x01, 0x61, 0x73, 0x6d)), "the magic's first byte off").toBe(false);
    expect(isWasmModule(new TextEncoder().encode("<!doctype html><title>Not found</title>").slice().buffer), "a fallback page served 200").toBe(false);
  });
});

describe("cannotCompile", () => {
  test("finds an error caused by the browser's CompileError, and nothing else", () => {
    const compile = new WebAssembly.CompileError("bad section");
    expect(cannotCompile(new Error("cannot be compiled", { cause: compile })), "caused by it").toBe(true);
    expect(cannotCompile(compile), "the CompileError itself, no module's error").toBe(false);
    expect(cannotCompile(new Error("another interface")), "no cause").toBe(false);
    expect(cannotCompile(new Error("a trap", { cause: new WebAssembly.RuntimeError("unreachable") })), "another cause").toBe(false);
    expect(cannotCompile({ cause: compile }), "not an error").toBe(false);
    expect(cannotCompile(undefined), "nothing").toBe(false);
  });
});
