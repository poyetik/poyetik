import { describe, expect, test } from "vitest";
import { artifactPath, BuildError, cargoArgs, commitHashOf, encodedRustflags, identical, leakedPaths, reproducibilityProblems, sysrootOf, targetName, type WasmModule } from "../src/wasm.ts";

/** The engine's module, as `bindings/audio-wasm` builds it. */
const ENGINE: WasmModule = { package: "audio-wasm", label: "engine module", file: "engine.wasm" };

/** Another module, so that nothing of the engine's is assumed. */
const OTHER: WasmModule = { package: "visual-wasm", label: "visual module", file: "visual.wasm" };

describe("encodedRustflags", () => {
  test("remaps the Cargo home, the repository and the standard library's sources, separated by U+001F", () => {
    // The sources rust-src installs in the sysroot become the path the standard library
    // was built with, /rustc/<commit>, which a toolchain without rust-src embeds.
    expect(encodedRustflags("/home/u/.cargo", "/src/repo", { sysroot: "/r/1.99.1", commit: "0123456789abcdef0123456789abcdef01234567" })).toBe(
      "--remap-path-prefix\u001f/home/u/.cargo=/cargo\u001f--remap-path-prefix\u001f/src/repo=/poyetik\u001f--remap-path-prefix\u001f/r/1.99.1/lib/rustlib/src/rust=/rustc/0123456789abcdef0123456789abcdef01234567",
    );
  });

  test("builds the release engine for WebAssembly, with JSON messages", () => {
<<<<<<<< HEAD:bindings/audio-wasm/test/wasm-build.test.ts
    expect(CARGO_ARGS).toStrictEqual(["build", "--package", "audio-wasm", "--release", "--locked", "--target", "wasm32-unknown-unknown", "--message-format", "json-render-diagnostics"]);
========
    expect(cargoArgs(OTHER)).toStrictEqual(["build", "--package", "visual-wasm", "--release", "--locked", "--target", "wasm32-unknown-unknown", "--message-format", "json-render-diagnostics"]);
    expect(cargoArgs(ENGINE)).toStrictEqual(["build", "--package", "audio-wasm", "--release", "--locked", "--target", "wasm32-unknown-unknown", "--message-format", "json-render-diagnostics"]);
>>>>>>>> b30d7e6a (refactor(wasm-build): share the WebAssembly module build):tooling/wasm-build/test/wasm-build.test.ts
  });
});

describe("artifactPath", () => {
  const artifact = (name: string, filenames: unknown): string => JSON.stringify({ reason: "compiler-artifact", target: { name }, filenames });

  test("reads the engine module from cargo's messages, wherever the target directory is", () => {
    const messages = [
      "   Compiling audio-wasm",
      JSON.stringify({ reason: "build-finished", success: true }),
      artifact("audio_core", ["/t/libaudio_core.rlib"]),
      artifact("audio_wasm", ["/t/libaudio_wasm.rlib", "/elsewhere/audio_wasm.wasm"]),
      "",
    ].join("\n");
<<<<<<<< HEAD:bindings/audio-wasm/test/wasm-build.test.ts
    expect(artifactPath(messages)).toBe("/elsewhere/audio_wasm.wasm");
========
    expect(artifactPath(messages, ENGINE)).toBe("/elsewhere/audio_wasm.wasm");
>>>>>>>> b30d7e6a (refactor(wasm-build): share the WebAssembly module build):tooling/wasm-build/test/wasm-build.test.ts
  });

  test.each([
    ["no message", ""],
    ["no engine", artifact("audio_core", ["/t/x.wasm"])],
    ["no module among the files", artifact("audio_wasm", ["/t/libaudio_wasm.rlib", 3])],
    ["files that are not a list", artifact("audio_wasm", "/t/audio_wasm.wasm")],
    ["another reason", JSON.stringify({ reason: "compiler-message", target: { name: "audio_wasm" }, filenames: ["/t/audio_wasm.wasm"] })],
  ])("explains a build that does not report the module: %s", (_case, messages) => {
<<<<<<<< HEAD:bindings/audio-wasm/test/wasm-build.test.ts
    expect(() => artifactPath(messages)).toThrow(new BuildError("cargo did not report the engine module (audio_wasm .wasm)"));
========
    expect(() => artifactPath(messages, ENGINE)).toThrow(new BuildError("cargo did not report the engine module (audio_wasm .wasm)"));
>>>>>>>> b30d7e6a (refactor(wasm-build): share the WebAssembly module build):tooling/wasm-build/test/wasm-build.test.ts
  });

  test("names its errors", () => {
    expect(new BuildError("x")).toMatchObject({ name: "BuildError", message: "x" });
  });
});

describe("sysrootOf", () => {
  test("reads the one path rustc --print sysroot prints", () => {
    expect(sysrootOf("/r/1.99.1\n")).toBe("/r/1.99.1");
    expect(sysrootOf("/r/with space\n")).toBe("/r/with space");
  });

  test.each(["", "\n", "/r/1.99.1", "/r/1.99.1\n\n", "/a\n/b\n", "/r/1.99.1\r\n", "/r/\u007f\n"])("refuses anything else: %j", (printed) => {
    expect(() => sysrootOf(printed)).toThrow(new BuildError(`rustc --print sysroot printed no single path: ${JSON.stringify(printed)}`));
  });
});

describe("commitHashOf", () => {
  const commit = "58cf8f9c35b355199727627d07ae34a4e2846d67";

  test("reads the commit hash rustc -vV prints", () => {
    expect(commitHashOf(`rustc 1.99.1 (58cf8f9c3 2026-10-13)\nbinary: rustc\ncommit-hash: ${commit}\ncommit-date: 2026-10-13\n`)).toBe(commit);
  });

  test.each([
    "rustc 1.99.1\n",
    "commit-hash: unknown\n",
    `commit-hash: ${commit.slice(1)}\n`,
    `commit-hash: ${commit}0\n`,
    `commit-hash: ${commit.toUpperCase()}\n`,
    `commit-hash:  ${commit}\n`,
    ` commit-hash: ${commit}\n`,
    `commit-hash: ${commit} \n`,
    `commit-hash: ${commit.slice(0, 39)}g\n`,
  ])("refuses a missing, truncated, padded or malformed hash: %j", (verbose) => {
    expect(() => commitHashOf(verbose)).toThrow(new BuildError("rustc -vV reported no commit hash (40 hexadecimal digits)"));
  });
});

describe("leakedPaths", () => {
  test("finds the paths of this machine in the module bytes, ignoring empty ones", () => {
    // Binary around the strings, invalid as UTF-8 (0xff), as in a real module.
    const text = new TextEncoder().encode("\u0000panic at /home/u/.cargo/registry/x.rs\u0000/cargo/y.rs");
    const bytes = new Uint8Array([0xff, 0xfe, ...text, 0xff]);
    expect(leakedPaths(bytes, ["/home/u/.cargo", "/src/repo", ""])).toStrictEqual(["/home/u/.cargo"]);
    expect(leakedPaths(bytes, ["/src/repo"])).toStrictEqual([]);
  });
});

describe("identical", () => {
  test("compares every byte and the length", () => {
    expect(identical(new Uint8Array([1, 2]), new Uint8Array([1, 2]))).toBe(true);
    expect(identical(new Uint8Array([1, 2]), new Uint8Array([1, 3]))).toBe(false);
    expect(identical(new Uint8Array([1, 2]), new Uint8Array([1, 2, 0]))).toBe(false);
    expect(identical(new Uint8Array([1, 2, 0]), new Uint8Array([1, 2]))).toBe(false);
  });
});

describe("each module", () => {
  test("is named by its own package", () => {
    expect([targetName(ENGINE), targetName(OTHER), targetName({ package: "a-b-c", label: "x", file: "x.wasm" })]).toStrictEqual(["audio_wasm", "visual_wasm", "a_b_c"]);
    const reported = JSON.stringify({ reason: "compiler-artifact", target: { name: "visual_wasm" }, filenames: ["/t/visual_wasm.wasm"] });
    expect(artifactPath(reported, OTHER)).toBe("/t/visual_wasm.wasm");
    expect(() => artifactPath(reported, ENGINE)).toThrow(new BuildError("cargo did not report the engine module (audio_wasm .wasm)"));
    expect(() => artifactPath("", OTHER)).toThrow(new BuildError("cargo did not report the visual module (visual_wasm .wasm)"));
  });

  test("is named in its reproducibility problems", () => {
    expect(reproducibilityProblems(new Uint8Array([1]), new Uint8Array([2]), ["/home"], OTHER)).toStrictEqual(["two builds of the visual module differ"]);
    expect(reproducibilityProblems(new TextEncoder().encode("/home/x"), new TextEncoder().encode("/home/x"), ["/home"], OTHER)).toStrictEqual(["the visual module contains the path /home"]);
  });
});
