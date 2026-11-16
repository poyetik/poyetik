import { createHash } from "node:crypto";
import { describe, expect, test } from "vitest";
import { buildMain, guardStdout, ignoreClosedReader, type Io, reproducibleMain, runEntryPoint } from "../src/cli.ts";
import { cargoArgs, targetName, type WasmModule } from "../src/wasm.ts";

/**
 * The modules these tests build: the engine's, as `bindings/audio-wasm` builds it, and
 * one whose every field differs, so that no field of the engine's can be hard-coded.
 */
const MODULES: readonly WasmModule[] = [
  { package: "audio-wasm", label: "engine module", file: "engine.wasm" },
  { package: "demo-wasm", label: "demo module", file: "demo.wasm" },
];

/** One cargo message reporting `module` at `path`. */
function reported(path: string, module: WasmModule): string {
  return `${JSON.stringify({ reason: "compiler-artifact", target: { name: targetName(module) }, filenames: [path] })}\n`;
}

/** The sysroot of the fake toolchain: outside the home directory, as `RUSTUP_HOME` may put it. */
const SYSROOT = "/opt/rustup/toolchains/1.99.1";

/** The commit of the fake toolchain. */
const COMMIT = "58cf8f9c35b355199727627d07ae34a4e2846d67";

/** What the fake `rustc -vV` prints. */
const VERBOSE_VERSION = `rustc 1.99.1 (58cf8f9c3 2026-10-13)\nbinary: rustc\ncommit-hash: ${COMMIT}\ncommit-date: 2026-10-13\nhost: aarch64-apple-darwin\nrelease: 1.99.1\nLLVM version: 23.1.1\n`;

/** The flags every build of the fake world passes, the Cargo home being `cargoHome`. */
function flags(cargoHome: string): string {
  return [
    "--remap-path-prefix",
    `${cargoHome}=/cargo`,
    "--remap-path-prefix",
    "/src/repo=/poyetik",
    "--remap-path-prefix",
    `${SYSROOT}/lib/rustlib/src/rust=/rustc/${COMMIT}`,
  ].join("\u001f");
}

/** The answers of the fake toolchain: `rustc --print sysroot`, then `rustc -vV`. */
const TOOLCHAIN = (args: readonly string[]): string => (args[0] === "--print" ? `${SYSROOT}\n` : VERBOSE_VERSION);

/**
 * A fake outside world: `run` answers `cargo` with `cargo` (given the target directory)
 * and `rustc` with `rustc` (given its arguments), files are read from `files`, and
 * everything done is recorded in `log`.
 */
function fakeIo(
  cargo: (targetDir: string | undefined) => string,
  files: Readonly<Record<string, Uint8Array>> = {},
  env: Readonly<Record<string, string>> = {},
  rustc: (args: readonly string[]) => string = TOOLCHAIN,
) {
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
      return command === "rustc" ? rustc(args) : cargo(options.env["CARGO_TARGET_DIR"]);
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
    temporaryRoot: "/tmp",
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

describe.each(MODULES)("build:wasm of the $label", (module) => {
  const target = targetName(module);
  const artifact = `/t/${target}.wasm`;

  test("builds the module reproducibly and copies it into dist", () => {
    const { io, log, out, err, runs } = fakeIo(() => `   Compiling\n${reported(artifact, module)}`);
    expect(buildMain(io, [], "/pkg/dist", [module])).toBe(0);
    // rustc is asked in the repository, so that it is the toolchain the repository pins.
    expect(runs).toStrictEqual([
      { command: "rustc", args: ["--print", "sysroot"], cwd: "/src/repo", env: { PATH: "/bin" } },
      { command: "rustc", args: ["-vV"], cwd: "/src/repo", env: { PATH: "/bin" } },
      { command: "cargo", args: cargoArgs(module), cwd: "/src/repo", env: { PATH: "/bin", CARGO_ENCODED_RUSTFLAGS: flags("/home/u/.cargo"), CARGO_TARGET_DIR: `/src/repo/target/wasm-build/${module.file}` } },
    ]);
    expect(log).toStrictEqual(["mkdir /pkg/dist", `copy ${artifact} /pkg/dist/${module.file}`]);
    expect([out, err]).toStrictEqual([[], []]);
  });

  test("refuses any argument but help with a usage error made safe on one line, and builds nothing", () => {
    for (const [args, quoted] of [
      [["--help"], "--help"],
      [["help", "x"], "x"],
      [["help", "help"], "help"],
      [["a\u202eb\nc"], "a\\u{202e}b\\u{a}c"],
    ] as const) {
      const { io, out, err, runs } = fakeIo(() => reported(artifact, module));
      expect(buildMain(io, args, "/pkg/dist", [module]), quoted).toBe(2);
      expect([out, err], quoted).toStrictEqual([[], [`error: build:wasm takes no argument but help, not "${quoted}" (see: pnpm build:wasm help)`]]);
      expect(runs).toStrictEqual([]);
    }
    const long = fakeIo(() => reported(artifact, module));
    expect(buildMain(long.io, ["x".repeat(3000)], "/pkg/dist", [module])).toBe(2);
    expect(long.err[0]?.endsWith("… (see: pnpm build:wasm help)"), "cut short, its pointer kept").toBe(true);
    expect(long.err[0]?.length).toBe("error: ".length + 1000 + " (see: pnpm build:wasm help)".length);
  });

  test("prints its usage for help, and builds nothing", () => {
    const { io, out, err, runs } = fakeIo(() => reported(artifact, module));
    expect(buildMain(io, ["help"], "/pkg/dist", [module])).toBe(0);
    expect([out, err, runs]).toStrictEqual([[`usage: pnpm build:wasm [help]\n  build the ${module.label} reproducibly into /pkg/dist/${module.file}`], [], []]);
  });

  test("remaps the Cargo home CARGO_HOME names", () => {
    const { io, runs } = fakeIo(() => reported(artifact, module), {}, { CARGO_HOME: "/opt/cargo" });
    expect(buildMain(io, [], "/pkg/dist", [module])).toBe(0);
    expect(runs[2]?.env).toStrictEqual({ PATH: "/bin", CARGO_HOME: "/opt/cargo", CARGO_ENCODED_RUSTFLAGS: flags("/opt/cargo"), CARGO_TARGET_DIR: `/src/repo/target/wasm-build/${module.file}` });
  });

  test.each([
    [failed(101, null), "failed (exit 101)"],
    [failed(null, "SIGKILL"), "was stopped by signal SIGKILL"],
    [Object.assign(new Error("spawnSync cargo ENOENT"), { code: "ENOENT" }), "could not be run: Error: spawnSync cargo ENOENT"],
  ])("explains a failed cargo in one line naming the module, never a stack trace: %s", (thrown, why) => {
    const { io, log, out, err } = fakeIo(() => {
      throw thrown;
    });
    expect(buildMain(io, [], "/pkg/dist", [module])).toBe(1);
    expect([out, err, log]).toStrictEqual([[], [`error: cargo build of the ${module.label} ${why}`], ["mkdir /pkg/dist"]]);
  });

  test.each([
    [["--print"], failed(1, null), "error: rustc --print sysroot failed (exit 1)"],
    [["-vV"], failed(null, "SIGKILL"), "error: rustc -vV was stopped by signal SIGKILL"],
  ])("explains a failed rustc %s in one line, and builds nothing", (failing, thrown, line) => {
    const { io, log, out, err, runs } = fakeIo(
      () => reported(artifact, module),
      {},
      {},
      (args) => {
        if (args[0] === failing[0]) {
          throw thrown;
        }
        return TOOLCHAIN(args);
      },
    );
    expect(buildMain(io, [], "/pkg/dist", [module])).toBe(1);
    expect([out, err, log]).toStrictEqual([[], [line], []]);
    expect(runs.map((run) => run.command)).not.toContain("cargo");
  });

  test.each([
    [["", VERBOSE_VERSION], 'error: rustc --print sysroot printed no single path: ""'],
    [[`${SYSROOT}\n`, "rustc 1.99.1\ncommit-hash: unknown\n"], "error: rustc -vV reported no commit hash (40 hexadecimal digits)"],
  ])("refuses what rustc prints when it is not understood: %j", ([sysroot, verbose], line) => {
    const { io, err } = fakeIo(
      () => reported(artifact, module),
      {},
      {},
      (args) => String(args[0] === "--print" ? sysroot : verbose),
    );
    expect(buildMain(io, [], "/pkg/dist", [module])).toBe(1);
    expect(err).toStrictEqual([line]);
  });

  test("explains a build that does not report the module, or not in JSON", () => {
    const { io, err } = fakeIo(() => "");
    expect(buildMain(io, [], "/pkg/dist", [module])).toBe(1);
    expect(err).toStrictEqual([`error: cargo did not report the ${module.label} (${target} .wasm)`]);
    const garbled = fakeIo(() => "{not JSON\n");
    expect(buildMain(garbled.io, [], "/pkg/dist", [module])).toBe(1);
    expect(garbled.err).toStrictEqual([expect.stringMatching(/^error: cargo printed a line that is not JSON \(SyntaxError: .+\)$/)]);
  });

  test.each([
    ["makeDirectory", `error: cannot create /pkg/dist: ENOTDIR`],
    ["copy", `error: cannot copy ${artifact} to /pkg/dist/${module.file}: ENOSPC`],
  ] as const)("explains a file it cannot write (%s) in one line naming it", (operation, line) => {
    const { io, out, err } = fakeIo(() => reported(artifact, module));
    const broken: Io = {
      ...io,
      [operation]: () => {
        throw Object.assign(new Error("failed"), { code: operation === "copy" ? "ENOSPC" : "ENOTDIR" });
      },
    };
    expect(buildMain(broken, [], "/pkg/dist", [module])).toBe(1);
    expect([out, err]).toStrictEqual([[], [line]]);
  });

  test("reports even an unexpected failure in one line", () => {
    const { io, err } = fakeIo(() => reported(artifact, module));
    const broken: Io = {
      ...io,
      run: () => {
        throw new TypeError("a bug");
      },
    };
    expect(buildMain(broken, [], "/pkg/dist", [module])).toBe(1);
    expect(err).toStrictEqual(["error: rustc --print sysroot could not be run: TypeError: a bug"]);
    const strange: Io = {
      ...io,
      copy: () => {
        throw new Error("no code");
      },
    };
    expect(buildMain(strange, [], "/pkg/dist", [module])).toBe(1);
    expect(err.at(-1)).toBe(`error: cannot copy ${artifact} to /pkg/dist/${module.file}: Error: no code`);
  });
});

describe.each(MODULES)("test:reproducible of the $label", (module) => {
  const target = targetName(module);
  const [dirA, dirB] = [`/tmp/${module.package}-a-1`, `/tmp/${module.package}-b-2`];

  /** A module of these bytes, built in each target directory. */
  function builds(first: Uint8Array, second: Uint8Array): ReturnType<typeof fakeIo> {
    return fakeIo((targetDir) => reported(`${String(targetDir)}/${target}.wasm`, module), {
      [`${dirA}/${target}.wasm`]: first,
      [`${dirB}/${target}.wasm`]: second,
    });
  }

  test("builds twice in two fresh directories, removes them, and prints the digest", () => {
    const bytes = new Uint8Array([0, 97, 115, 109]);
    const { io, log, out, err, runs } = builds(bytes, bytes.slice());
    expect(reproducibleMain(io, [], [module])).toBe(0);
    expect(runs.filter((run) => run.command === "cargo").map((run) => run.env["CARGO_TARGET_DIR"])).toStrictEqual([dirA, dirB]);
    expect(log).toStrictEqual([`temp ${module.package}-a-`, `temp ${module.package}-b-`, `read ${dirA}/${target}.wasm`, `read ${dirB}/${target}.wasm`, `remove ${dirA}`, `remove ${dirB}`]);
    expect(out).toStrictEqual([`the ${module.label} is reproducible: sha256 ${createHash("sha256").update(bytes).digest("hex")}`]);
    // Independent value: SHA-256 of the bytes 00 61 73 6d (`printf '\0asm' | shasum -a 256`).
    expect(out[0]).toBe(`the ${module.label} is reproducible: sha256 cd5d4935a48c0672cb06407bb443bc0087aff947c6b864bac886982c73b3027f`);
    expect(err).toStrictEqual([]);
  });

  test("refuses any argument but help with a usage error, prints its usage for help, and creates nothing", () => {
    const refused = builds(new Uint8Array(), new Uint8Array());
    expect(reproducibleMain(refused.io, ["now"], [module])).toBe(2);
    expect([refused.err, refused.log]).toStrictEqual([['error: test:reproducible takes no argument but help, not "now" (see: pnpm test:reproducible help)'], []]);
    const helped = builds(new Uint8Array(), new Uint8Array());
    expect(reproducibleMain(helped.io, ["help"], [module])).toBe(0);
    expect([helped.out, helped.err, helped.log]).toStrictEqual([
      [`usage: pnpm test:reproducible [help]\n  build the ${module.label} twice, in two fresh target directories shared by the modules, the second time in reverse order, and check that both builds of each are the same bytes, holding no path of this machine`],
      [],
      [],
    ]);
  });

  test("lists every problem on stdout, then fails with one error line", () => {
    // The standard library's sources, which the rust-src component installs in the
    // sysroot, outside the home directory here.
    const leaking = new TextEncoder().encode(`panic at /src/repo/crates/x.rs and /home/u/.cargo/registry/y.rs and ${SYSROOT}/lib/rustlib/src/rust/library/core/src/str/pattern.rs`);
    const { io, out, err } = builds(leaking, new Uint8Array([1]));
    expect(reproducibleMain(io, [], [module])).toBe(1);
    expect(out).toStrictEqual([
      `two builds of the ${module.label} differ`,
      `the ${module.label} contains the path /src/repo`,
      `the ${module.label} contains the path /home/u/.cargo`,
      `the ${module.label} contains the path /home/u`,
      `the ${module.label} contains the path ${SYSROOT}`,
      `the ${module.label} is not reproducible (5 problem(s))`,
    ]);
    expect(err).toStrictEqual(["error: 1 of 1 module(s) not reproducible"]);
  });

  test("removes both directories when a build fails", () => {
    const { io, log, err } = fakeIo(() => {
      throw failed(101, null);
    });
    expect(reproducibleMain(io, [], [module])).toBe(1);
    expect(err).toStrictEqual([`error: cargo build of the ${module.label} failed (exit 101)`]);
    expect(log).toStrictEqual([`temp ${module.package}-a-`, `temp ${module.package}-b-`, `remove ${dirA}`, `remove ${dirB}`]);
  });

  test("removes the first directory when the second cannot be created, in one error line", () => {
    const { io, log, err } = builds(new Uint8Array(), new Uint8Array());
    let made = 0;
    const broken: Io = {
      ...io,
      temporaryDirectory: (prefix) => {
        made += 1;
        if (made === 2) {
          throw Object.assign(new Error("mkdtemp failed"), { code: "ENOENT" });
        }
        return io.temporaryDirectory(prefix);
      },
    };
    expect(reproducibleMain(broken, [], [module])).toBe(1);
    expect(err).toStrictEqual(["error: cannot create a temporary directory in /tmp: ENOENT"]);
    expect(log).toStrictEqual([`temp ${module.package}-a-`, `remove ${dirA}`]);
  });

  test("says which file it cannot read, and which directory it cannot remove, one line each", () => {
    const { io, err } = builds(new Uint8Array([1]), new Uint8Array([1]));
    const broken: Io = {
      ...io,
      read: () => {
        throw Object.assign(new Error("read failed"), { code: "EACCES" });
      },
      remove: (path) => {
        throw Object.assign(new Error("rm failed"), { code: path === dirA ? "EBUSY" : "EPERM" });
      },
    };
    expect(reproducibleMain(broken, [], [module])).toBe(1);
    expect(err).toStrictEqual([`error: cannot read ${dirA}/${target}.wasm: EACCES`, `error: cannot remove ${dirA}: EBUSY`, `error: cannot remove ${dirB}: EPERM`]);
    const kept = builds(new Uint8Array([1]), new Uint8Array([1]));
    const unremovable: Io = {
      ...kept.io,
      remove: () => {
        throw Object.assign(new Error("rm failed"), { code: "EBUSY" });
      },
    };
    expect(reproducibleMain(unremovable, [], [module]), "reproducible, but a directory left").toBe(1);
  });

  test("makes every error line safe on one line, whatever text it carries", () => {
    const { io, err } = builds(new Uint8Array([1]), new Uint8Array([1]));
    const strange: Io = {
      ...io,
      temporaryRoot: "/tmp/a\nb\u202ec",
      temporaryDirectory: () => {
        throw Object.assign(new Error("failed"), { code: "ENOENT" });
      },
    };
    expect(reproducibleMain(strange, [], [module])).toBe(1);
    expect(err).toStrictEqual(["error: cannot create a temporary directory in /tmp/a\\u{a}b\\u{202e}c: ENOENT"]);
  });

  test("reports even an unexpected failure in one line", () => {
    const { io, err } = builds(new Uint8Array([1]), new Uint8Array([1]));
    const broken: Io = {
      ...io,
      out: () => {
        throw new TypeError("a bug");
      },
    };
    expect(reproducibleMain(broken, [], [module])).toBe(1);
    expect(err).toStrictEqual(["error: unexpected failure: TypeError: a bug"]);
  });
});

describe("the modules of one package", () => {
  /**
   * Two modules of one crate, each built for a size its build names, as the engine's are;
   * the second also copied to two more files.
   */
  const SIZES: readonly [WasmModule, WasmModule] = [
    { package: "audio-wasm", label: "engine module of 16-frame engine blocks", file: "engine-16.wasm", env: { POYETIK_ENGINE_BLOCK: "16" } },
    { package: "audio-wasm", label: "engine module of 64-frame engine blocks", file: "engine-64.wasm", copies: ["engine.wasm", "default.wasm"], env: { POYETIK_ENGINE_BLOCK: "64", EXTRA: "x" } },
  ];
  const artifact = "/t/audio_wasm.wasm";

  test("builds each in a target directory of its own, below the one cargo would use, so that its variables never rebuild another's", () => {
    for (const [environment, base] of [
      [{}, "/src/repo/target"],
      [{ CARGO_TARGET_DIR: "/cache/cargo" }, "/cache/cargo"],
      [{ CARGO_TARGET_DIR: "build/out" }, "/src/repo/build/out"],
    ] as const) {
      const { io, runs } = fakeIo(() => reported(artifact, SIZES[0]), {}, environment);
      expect(buildMain(io, [], "/pkg/dist", SIZES)).toBe(0);
      expect(runs.filter((run) => run.command === "cargo").map((run) => run.env["CARGO_TARGET_DIR"])).toStrictEqual([`${base}/wasm-build/engine-16.wasm`, `${base}/wasm-build/engine-64.wasm`]);
    }
  });

  test("builds each in turn with its own variables, whatever the environment holds, and copies each to its files", () => {
    const { io, log, out, err, runs } = fakeIo(() => reported(artifact, SIZES[0]), {}, { POYETIK_ENGINE_BLOCK: "48" });
    expect(buildMain(io, [], "/pkg/dist", SIZES)).toBe(0);
    const environment = { PATH: "/bin", POYETIK_ENGINE_BLOCK: "48" };
    expect(runs).toStrictEqual([
      { command: "rustc", args: ["--print", "sysroot"], cwd: "/src/repo", env: environment },
      { command: "rustc", args: ["-vV"], cwd: "/src/repo", env: environment },
      { command: "cargo", args: cargoArgs(SIZES[0]), cwd: "/src/repo", env: { PATH: "/bin", POYETIK_ENGINE_BLOCK: "16", CARGO_ENCODED_RUSTFLAGS: flags("/home/u/.cargo"), CARGO_TARGET_DIR: "/src/repo/target/wasm-build/engine-16.wasm" } },
      { command: "cargo", args: cargoArgs(SIZES[1]), cwd: "/src/repo", env: { PATH: "/bin", POYETIK_ENGINE_BLOCK: "64", EXTRA: "x", CARGO_ENCODED_RUSTFLAGS: flags("/home/u/.cargo"), CARGO_TARGET_DIR: "/src/repo/target/wasm-build/engine-64.wasm" } },
    ]);
    expect(log).toStrictEqual([
      "mkdir /pkg/dist",
      `copy ${artifact} /pkg/dist/engine-16.wasm`,
      `copy ${artifact} /pkg/dist/engine-64.wasm`,
      `copy ${artifact} /pkg/dist/engine.wasm`,
      `copy ${artifact} /pkg/dist/default.wasm`,
    ]);
    expect([out, err]).toStrictEqual([[], []]);
  });

  test("names every module and every file in its usage", () => {
    const built = fakeIo(() => reported(artifact, SIZES[0]));
    expect(buildMain(built.io, ["help"], "/pkg/dist", SIZES)).toBe(0);
    expect(built.out).toStrictEqual([
      "usage: pnpm build:wasm [help]\n  build the engine module of 16-frame engine blocks reproducibly into /pkg/dist/engine-16.wasm\n  build the engine module of 64-frame engine blocks reproducibly into /pkg/dist/engine-64.wasm and /pkg/dist/engine.wasm and /pkg/dist/default.wasm",
    ]);
    const checked = fakeIo(() => reported(artifact, SIZES[0]));
    expect(reproducibleMain(checked.io, ["help"], SIZES)).toBe(0);
    expect(checked.out).toStrictEqual([
      "usage: pnpm test:reproducible [help]\n  build the engine module of 16-frame engine blocks, the engine module of 64-frame engine blocks twice, in two fresh target directories shared by the modules, the second time in reverse order, and check that both builds of each are the same bytes, holding no path of this machine",
    ]);
  });

  test("builds each twice in the same two directories, the second time in reverse order, and prints the digest of each", () => {
    const bytes = new Uint8Array([0, 97, 115, 109]);
    const { io, log, out, err, runs } = fakeIo((targetDir) => reported(`${String(targetDir)}/audio_wasm.wasm`, SIZES[0]), {
      "/tmp/audio-wasm-a-1/audio_wasm.wasm": bytes,
      "/tmp/audio-wasm-b-2/audio_wasm.wasm": bytes.slice(),
    });
    expect(reproducibleMain(io, [], SIZES)).toBe(0);
    expect(runs.filter((run) => run.command === "cargo").map((run) => [run.env["POYETIK_ENGINE_BLOCK"], run.env["CARGO_TARGET_DIR"]])).toStrictEqual([
      // Built in the order of the list, then in the reverse order: the bytes of a module never
      // depend on the modules built before it in the same directory.
      ["16", "/tmp/audio-wasm-a-1"],
      ["64", "/tmp/audio-wasm-a-1"],
      ["64", "/tmp/audio-wasm-b-2"],
      ["16", "/tmp/audio-wasm-b-2"],
    ]);
    expect(log).toStrictEqual([
      "temp audio-wasm-a-",
      "temp audio-wasm-b-",
      ...["read /tmp/audio-wasm-a-1/audio_wasm.wasm", "read /tmp/audio-wasm-a-1/audio_wasm.wasm", "read /tmp/audio-wasm-b-2/audio_wasm.wasm", "read /tmp/audio-wasm-b-2/audio_wasm.wasm"],
      "remove /tmp/audio-wasm-a-1",
      "remove /tmp/audio-wasm-b-2",
    ]);
    // Independent value: SHA-256 of the bytes 00 61 73 6d (`printf '\0asm' | shasum -a 256`).
    expect(out).toStrictEqual([
      "the engine module of 16-frame engine blocks is reproducible: sha256 cd5d4935a48c0672cb06407bb443bc0087aff947c6b864bac886982c73b3027f",
      "the engine module of 64-frame engine blocks is reproducible: sha256 cd5d4935a48c0672cb06407bb443bc0087aff947c6b864bac886982c73b3027f",
    ]);
    expect(err).toStrictEqual([]);
  });

  test("checks every module, reports each, then fails with one error line counting those that are not reproducible", () => {
    // Bytes by target directory and engine block: the first module differs, the second as
    // given.
    const cases = [
      [new Uint8Array([3]), ["two builds of the engine module of 64-frame engine blocks differ", "the engine module of 64-frame engine blocks is not reproducible (1 problem(s))"], "2 of 2"],
      [new Uint8Array([1]), ["the engine module of 64-frame engine blocks is reproducible: sha256 4bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459a"], "1 of 2"],
    ] as const;
    for (const [secondB, verdict, counted] of cases) {
      const { io, log, out, err } = fakeIo((targetDir) => String(targetDir), {});
      const read: string[] = [];
      const sized: Io = {
        ...io,
        run: (command, args, options) => {
          const answer = io.run(command, args, options);
          return command === "cargo" ? reported(`${answer}/${String(options.env["POYETIK_ENGINE_BLOCK"])}.wasm`, SIZES[0]) : answer;
        },
        read: (path) => {
          read.push(path);
          const bytes: Readonly<Record<string, Uint8Array>> = {
            "/tmp/audio-wasm-a-1/16.wasm": new Uint8Array([1]),
            "/tmp/audio-wasm-b-2/16.wasm": new Uint8Array([2]),
            "/tmp/audio-wasm-a-1/64.wasm": new Uint8Array([1]),
            "/tmp/audio-wasm-b-2/64.wasm": secondB,
          };
          return bytes[path] ?? new Uint8Array();
        },
      };
      expect(reproducibleMain(sized, [], SIZES)).toBe(1);
      expect(out).toStrictEqual(["two builds of the engine module of 16-frame engine blocks differ", "the engine module of 16-frame engine blocks is not reproducible (1 problem(s))", ...verdict]);
      expect(err).toStrictEqual([`error: ${counted} module(s) not reproducible`]);
      expect(read).toHaveLength(4);
      expect(log.slice(-2)).toStrictEqual(["remove /tmp/audio-wasm-a-1", "remove /tmp/audio-wasm-b-2"]);
    }
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

  test("ignores a closed reader, and reports any other stream error in one line", () => {
    const closed = stream();
    const failures: string[] = [];
    ignoreClosedReader(closed, (line) => failures.push(line));
    expect(closed.events).toStrictEqual(["error"]);
    closed.fail(Object.assign(new Error("write EPIPE"), { code: "EPIPE" }));
    expect(failures).toStrictEqual([]);
    closed.fail(Object.assign(new Error("write EIO"), { code: "EIO" }));
    closed.fail(new Error("broken"));
    expect(failures).toStrictEqual(["error: cannot write to stdout: EIO", "error: cannot write to stdout: Error: broken"]);
  });
});

describe("guardStdout", () => {
  test("writes a stdout error on stderr and sets exit code 1, a closed reader apart", () => {
    let listener: (error: Error) => void = () => undefined;
    const stream = {
      on: (_event: "error", added: (error: Error) => void) => {
        listener = added;
      },
    };
    const lines: string[] = [];
    const codes: number[] = [];
    guardStdout(stream, (line) => lines.push(line), (code) => codes.push(code));
    listener(Object.assign(new Error("write EPIPE"), { code: "EPIPE" }));
    expect([lines, codes]).toStrictEqual([[], []]);
    listener(Object.assign(new Error("write EIO"), { code: "EIO" }));
    expect([lines, codes]).toStrictEqual([["error: cannot write to stdout: EIO"], [1]]);
  });
});

describe("runEntryPoint", () => {
  /** A fake entry process: its stdout's error listener, its stderr lines and its exit codes, in order. */
  function entry(): { process: Parameters<typeof runEntryPoint>[0]; fail: (error: Error) => void; events: string[] } {
    let listener: (error: Error) => void = () => undefined;
    const events: string[] = [];
    return {
      events,
      fail: (error) => {
        listener(error);
      },
      process: {
        stdout: {
          on: (_event: "error", added: (error: Error) => void) => {
            events.push("guard");
            listener = added;
          },
        },
        err: (line) => events.push(line),
        exit: (code) => events.push(`exit ${String(code)}`),
      },
    };
  }

  test("guards stdout before running the command, then exits with its code", () => {
    const run = entry();
    runEntryPoint(run.process, () => {
      run.events.push("main");
      return 2;
    });
    expect(run.events).toStrictEqual(["guard", "main", "exit 2"]);
  });

  test("keeps the command's exit code when its reader is gone, and fails on any other stdout error", () => {
    const closed = entry();
    runEntryPoint(closed.process, () => 0);
    closed.fail(Object.assign(new Error("write EPIPE"), { code: "EPIPE" }));
    expect(closed.events).toStrictEqual(["guard", "exit 0"]);
    closed.fail(Object.assign(new Error("write EIO"), { code: "EIO" }));
    expect(closed.events).toStrictEqual(["guard", "exit 0", "error: cannot write to stdout: EIO", "exit 1"]);
  });
});
