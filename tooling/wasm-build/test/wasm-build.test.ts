import { describe, expect, test } from "vitest";
import { artifactPath, BuildError, cargoArgs, cargoHomeOf, commitHashOf, encodedRustflags, identical, leakedPaths, privatePaths, reproducibilityProblems, sysrootOf, targetName, type WasmModule } from "../src/wasm.ts";

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
    expect(cargoArgs(OTHER)).toStrictEqual(["build", "--package", "visual-wasm", "--release", "--locked", "--target", "wasm32-unknown-unknown", "--message-format", "json-render-diagnostics"]);
    expect(cargoArgs(ENGINE)).toStrictEqual(["build", "--package", "audio-wasm", "--release", "--locked", "--target", "wasm32-unknown-unknown", "--message-format", "json-render-diagnostics"]);
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
    expect(artifactPath(messages, ENGINE)).toBe("/elsewhere/audio_wasm.wasm");
  });

  test.each([
    ["no message", ""],
    ["no engine", artifact("audio_core", ["/t/x.wasm"])],
    ["no module among the files", artifact("audio_wasm", ["/t/libaudio_wasm.rlib", 3])],
    ["files that are not a list", artifact("audio_wasm", "/t/audio_wasm.wasm")],
    ["another reason", JSON.stringify({ reason: "compiler-message", target: { name: "audio_wasm" }, filenames: ["/t/audio_wasm.wasm"] })],
  ])("explains a build that does not report the module: %s", (_case, messages) => {
    expect(() => artifactPath(messages, ENGINE)).toThrow(new BuildError("cargo did not report the engine module (audio_wasm .wasm)"));
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
  test("finds the paths of this machine in the module bytes, followed by a child of theirs, ignoring empty ones", () => {
    // Binary around the strings, invalid as UTF-8 (0xff), as in a real module.
    const text = new TextEncoder().encode("\u0000panic at /home/u/.cargo/registry/x.rs\u0000/cargo/y.rs");
    const bytes = new Uint8Array([0xff, 0xfe, ...text, 0xff]);
    const cargo = { path: "/home/u/.cargo", children: ["/registry/"] };
    expect(leakedPaths(bytes, [cargo, { path: "/src/repo", children: ["/crates/"] }, { path: "", children: ["/"] }])).toStrictEqual(["/home/u/.cargo"]);
    expect(leakedPaths(bytes, [{ path: "/src/repo", children: ["/crates/"] }])).toStrictEqual([]);
    expect(leakedPaths(bytes, [{ path: "/home/u/.cargo", children: ["/git/", "/registry/"] }]), "any of its children").toStrictEqual(["/home/u/.cargo"]);
    expect(leakedPaths(bytes, [{ path: "/home/u/.cargo", children: [] }]), "no child, no leak").toStrictEqual([]);
  });

  test("tells a leak from the module's own relative paths, in a container too", () => {
    // A repository at /src, whose sources are in crates/*/src/, and a home directory at /.
    const paths = privatePaths("/src", "/cargo-home", "/", "/sysroot");
    // Each path alone, or followed by anything but a child of a leak, as text may hold it.
    const own = new TextEncoder().encode("crates/visual-core/src/image.rs /rustc/abc/library/core/src/num/mod.rs https://x /cargo-home /cargo-home/x /sysroot /sysroot/x /src");
    expect(leakedPaths(own, paths), "relative paths and other text are not leaks").toStrictEqual([]);
    for (const [leak, path] of [
      ["/src/crates/visual-core/src/image.rs", "/src"],
      ["/src/bindings/visual-wasm/src/ffi.rs", "/src"],
      ["/cargo-home/registry/src/index.crates.io-0/sha2-0.11.1/src/lib.rs", "/cargo-home"],
      ["/cargo-home/git/checkouts/x/src/lib.rs", "/cargo-home"],
      ["/sysroot/lib/rustlib/src/rust/library/core/src/lib.rs", "/sysroot"],
    ] as const) {
      expect(leakedPaths(new TextEncoder().encode(leak), paths), leak).toStrictEqual([path]);
    }
    expect(leakedPaths(new TextEncoder().encode("/home/u/x"), privatePaths("/r", "/c", "/home/u", "/s")), "anything of a home directory").toStrictEqual(["/home/u"]);
    expect(leakedPaths(new TextEncoder().encode("/home/user2/x"), privatePaths("/r", "/c", "/home/u", "/s")), "another home directory").toStrictEqual([]);
    expect(privatePaths("/r", "/c", "/", "/s").map(({ path }) => path), "no home directory at /").toStrictEqual(["/r", "/c", "/s"]);
  });
});

describe("cargoHomeOf", () => {
  test("reads CARGO_HOME as cargo does: unset or empty is the home's, relative is from where cargo runs", () => {
    expect(cargoHomeOf({}, "/home/u", "/src/repo")).toBe("/home/u/.cargo");
    expect(cargoHomeOf({ CARGO_HOME: "" }, "/home/u", "/src/repo")).toBe("/home/u/.cargo");
    expect(cargoHomeOf({ CARGO_HOME: "/opt/cargo" }, "/home/u", "/src/repo")).toBe("/opt/cargo");
    expect(cargoHomeOf({ CARGO_HOME: "tools/cargo" }, "/home/u", "/src/repo")).toBe("/src/repo/tools/cargo");
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
    expect(reproducibilityProblems(new Uint8Array([1]), new Uint8Array([2]), [{ path: "/home", children: ["/"] }], OTHER)).toStrictEqual(["two builds of the visual module differ"]);
    expect(reproducibilityProblems(new TextEncoder().encode("/home/x"), new TextEncoder().encode("/home/x"), [{ path: "/home", children: ["/"] }], OTHER)).toStrictEqual(["the visual module contains the path /home"]);
  });
});
