import { createHash } from "node:crypto";
import { describe, expect, test } from "vitest";
import { buildMain, ignoreClosedReader, type Io, reproducibleMain } from "../scripts/cli.ts";
import { CARGO_ARGS } from "../scripts/wasm.ts";

/** One cargo message reporting the engine module at `path`. */
function reported(path: string): string {
  return `${JSON.stringify({ reason: "compiler-artifact", target: { name: "engine_wasm" }, filenames: [path] })}\n`;
}

/**
 * A fake outside world: `run` answers with `cargo` (given the target directory), files
 * are read from `files`, and everything done is recorded in `log`.
 */
function fakeIo(cargo: (targetDir: string | undefined) => string, files: Readonly<Record<string, Uint8Array>> = {}, env: Readonly<Record<string, string>> = {}) {
  const log: string[] = [];
  const out: string[] = [];
  const err: string[] = [];
  const runs: { command: string; args: readonly string[]; cwd: string; env: Readonly<Record<string, string | undefined>> }[] = [];
  let made = 0;
  const io: Io = {
    env: { PATH: "/bin", ...env },
    home: "/home/u",
    repository: "/src/repo",
    run: (command, args, options) => {
      runs.push({ command, args, cwd: options.cwd, env: options.env });
      return cargo(options.env["CARGO_TARGET_DIR"]);
    },
    read: (path) => {
      log.push(`read ${path}`);
      const file = files[path];
      if (file === undefined) {
        throw new Error(`no file ${path}`);
      }
      return file;
    },
    makeDirectory: (path) => log.push(`mkdir ${path}`),
    copy: (from, to) => log.push(`copy ${from} ${to}`),
    temporaryDirectory: (prefix) => {
      made += 1;
      log.push(`temp ${prefix}`);
      return `/tmp/${prefix}${String(made)}`;
    },
    remove: (path) => log.push(`remove ${path}`),
    out: (line) => out.push(line),
    err: (line) => err.push(line),
  };
  return { io, log, out, err, runs };
}

/** What `execFileSync` throws when the command exits with `status`, or is stopped by `signal`. */
function failed(status: number | null, signal: string | null): Error {
  return Object.assign(new Error("Command failed: cargo build"), { status, signal });
}

describe("build:wasm", () => {
  test("builds the module reproducibly and copies it into dist", () => {
    const { io, log, out, err, runs } = fakeIo(() => `   Compiling\n${reported("/t/engine_wasm.wasm")}`);
    expect(buildMain(io, "/pkg/dist")).toBe(0);
    expect(runs).toStrictEqual([
      {
        command: "cargo",
        args: CARGO_ARGS,
        cwd: "/src/repo",
        env: { PATH: "/bin", CARGO_ENCODED_RUSTFLAGS: "--remap-path-prefix\u001f/home/u/.cargo=/cargo\u001f--remap-path-prefix\u001f/src/repo=/poyetik" },
      },
    ]);
    expect(log).toStrictEqual(["mkdir /pkg/dist", "copy /t/engine_wasm.wasm /pkg/dist/engine.wasm"]);
    expect([out, err]).toStrictEqual([[], []]);
  });

  test("remaps the Cargo home CARGO_HOME names", () => {
    const { io, runs } = fakeIo(() => reported("/t/engine_wasm.wasm"), {}, { CARGO_HOME: "/opt/cargo" });
    expect(buildMain(io, "/pkg/dist")).toBe(0);
    expect(runs[0]?.env).toStrictEqual({
      PATH: "/bin",
      CARGO_HOME: "/opt/cargo",
      CARGO_ENCODED_RUSTFLAGS: "--remap-path-prefix\u001f/opt/cargo=/cargo\u001f--remap-path-prefix\u001f/src/repo=/poyetik",
    });
  });

  test.each([
    [failed(101, null), "error: cargo build failed (exit 101)"],
    [failed(null, "SIGKILL"), "error: cargo build was stopped by signal SIGKILL"],
    [Object.assign(new Error("spawnSync cargo ENOENT"), { code: "ENOENT" }), "error: cargo build could not be run: Error: spawnSync cargo ENOENT"],
  ])("explains a failed cargo in one line, never a stack trace: %s", (thrown, line) => {
    const { io, log, out, err } = fakeIo(() => {
      throw thrown;
    });
    expect(buildMain(io, "/pkg/dist")).toBe(1);
    expect([out, err, log]).toStrictEqual([[], [line], []]);
  });

  test("explains a build that does not report the module", () => {
    const { io, err } = fakeIo(() => "");
    expect(buildMain(io, "/pkg/dist")).toBe(1);
    expect(err).toStrictEqual(["error: cargo did not report the engine module (engine_wasm .wasm)"]);
  });

  test("lets an unexpected error through", () => {
    const { io } = fakeIo(() => reported("/t/engine_wasm.wasm"));
    const broken: Io = {
      ...io,
      copy: () => {
        throw new Error("disk full");
      },
    };
    expect(() => buildMain(broken, "/pkg/dist")).toThrow(new Error("disk full"));
  });
});

describe("test:reproducible", () => {
  /** A module of these bytes, built in each target directory. */
  function builds(first: Uint8Array, second: Uint8Array): ReturnType<typeof fakeIo> {
    return fakeIo((targetDir) => reported(`${String(targetDir)}/engine_wasm.wasm`), {
      "/tmp/engine-wasm-a-1/engine_wasm.wasm": first,
      "/tmp/engine-wasm-b-2/engine_wasm.wasm": second,
    });
  }

  test("builds twice in two fresh directories, removes them, and prints the digest", () => {
    const module = new Uint8Array([0, 97, 115, 109]);
    const { io, log, out, err, runs } = builds(module, module.slice());
    expect(reproducibleMain(io)).toBe(0);
    expect(runs.map((run) => run.env["CARGO_TARGET_DIR"])).toStrictEqual(["/tmp/engine-wasm-a-1", "/tmp/engine-wasm-b-2"]);
    expect(log).toStrictEqual([
      "temp engine-wasm-a-",
      "temp engine-wasm-b-",
      "read /tmp/engine-wasm-a-1/engine_wasm.wasm",
      "read /tmp/engine-wasm-b-2/engine_wasm.wasm",
      "remove /tmp/engine-wasm-a-1",
      "remove /tmp/engine-wasm-b-2",
    ]);
    expect(out).toStrictEqual([`the engine module is reproducible: sha256 ${createHash("sha256").update(module).digest("hex")}`]);
    // Independent value: SHA-256 of the bytes 00 61 73 6d (`printf '\0asm' | shasum -a 256`).
    expect(out[0]).toBe("the engine module is reproducible: sha256 cd5d4935a48c0672cb06407bb443bc0087aff947c6b864bac886982c73b3027f");
    expect(err).toStrictEqual([]);
  });

  test("lists every problem on stdout, then fails with one error line", () => {
    const leaking = new TextEncoder().encode("panic at /src/repo/x.rs and /home/u/.cargo/y.rs");
    const { io, out, err } = builds(leaking, new Uint8Array([1]));
    expect(reproducibleMain(io)).toBe(1);
    expect(out).toStrictEqual([
      "two builds of the engine module differ",
      "the engine module contains the path /src/repo",
      "the engine module contains the path /home/u/.cargo",
      "the engine module contains the path /home/u",
    ]);
    expect(err).toStrictEqual(["error: the engine module is not reproducible (4 problem(s))"]);
  });

  test("removes both directories when a build fails", () => {
    const { io, log, err } = fakeIo(() => {
      throw failed(101, null);
    });
    expect(reproducibleMain(io)).toBe(1);
    expect(err).toStrictEqual(["error: cargo build failed (exit 101)"]);
    expect(log).toStrictEqual(["temp engine-wasm-a-", "temp engine-wasm-b-", "remove /tmp/engine-wasm-a-1", "remove /tmp/engine-wasm-b-2"]);
  });
});

describe("ignoreClosedReader", () => {
  /** A stream whose error listener the test calls. */
  function stream(): { on: (event: "error", listener: (error: Error) => void) => void; fail: (error: Error) => void; events: string[] } {
    let listener: (error: Error) => void = () => undefined;
    const events: string[] = [];
    return {
      events,
      on: (event, added) => {
        events.push(event);
        listener = added;
      },
      fail: (error) => {
        listener(error);
      },
    };
  }

  test("ignores a closed reader, and only that", () => {
    const closed = stream();
    ignoreClosedReader(closed);
    expect(closed.events).toStrictEqual(["error"]);
    expect(() => {
      closed.fail(Object.assign(new Error("write EPIPE"), { code: "EPIPE" }));
    }).not.toThrow();
    const other = Object.assign(new Error("write EIO"), { code: "EIO" });
    expect(() => {
      closed.fail(other);
    }).toThrow(other);
    const plain = new Error("broken");
    expect(() => {
      closed.fail(plain);
    }).toThrow(plain);
  });
});
