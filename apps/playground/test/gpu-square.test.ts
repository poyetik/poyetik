import { beforeEach, describe, expect, test } from "vitest";
import { BUFFER, type Ending, type GpuDrawing, type GpuSetup, type GpuSource, ROW_ALIGNMENT, type Refusal, TEXTURE_COPY_DST, gpuSquare } from "../src/gpu-square.ts";
import { FakeCanvas, FakeDevice, fakeGpu, gpuLog, leaked } from "./gpu-fakes.ts";

/** A page module for the GPU: placements counting from the index, two one-line shaders, thresholds 0 to 509. */
const SOURCE: GpuSource = {
  placement: (index) => [Number(index), Number(index) + 1, 2, 3],
  shader: (kind) => `${kind} shader`,
  thresholds: () => Uint32Array.from({ length: 510 }, (_slot, at) => at),
};

/** 320 × 180: rows of 1280 bytes, five times the alignment of a copy. */
const SIZE = { width: 320, height: 180 } as const;

/** The drawing `setup` gives; throws, with the reason, when the GPU refused it. */
function drawn(setup: GpuSetup): GpuDrawing {
  if (setup.kind === "refused") {
    throw new Error(`refused: ${setup.why.reason}`);
  }
  return setup;
}

/** Why `setup` was refused; throws when it gives a drawing. */
function why(setup: GpuSetup): Refusal {
  if (setup.kind === "drawing") {
    throw new Error("a drawing, not a refusal");
  }
  return setup.why;
}

/** Ignores an ending: for the tests that never see one. */
const NO_ENDING = (): void => undefined;

/** Lets the device's promises settle (its loss), as the browser's event loop would. */
async function settled(): Promise<void> {
  for (let turn = 0; turn < 4; turn += 1) {
    await Promise.resolve();
  }
}

beforeEach(() => {
  gpuLog.length = 0;
});

describe("gpuSquare", () => {
  test("sets the library's shaders up on the GPU, then draws each image in two passes and one copy to the canvas", async () => {
    const device = new FakeDevice("device");
    const canvas = new FakeCanvas();
    const draw = drawn(await gpuSquare(fakeGpu(device), () => canvas, SOURCE, SIZE, NO_ENDING));
    const pixels = 320 * 180;
    expect(gpuLog).toStrictEqual([
      "adapter",
      "device",
      "listen uncapturederror",
      "push validation",
      "push out-of-memory",
      "push internal",
      "module of square shader",
      "pipeline of module square shader at main, layout auto",
      "module of display shader",
      "pipeline of module display shader at main, layout auto",
      `buffer 1: 32 bytes, usage ${String(BUFFER.UNIFORM | BUFFER.COPY_DST)}`,
      `buffer 2: ${String(pixels * 16)} bytes, usage ${String(BUFFER.STORAGE)}`,
      `buffer 3: 2040 bytes, usage ${String(BUFFER.STORAGE | BUFFER.COPY_DST)}`,
      `buffer 4: 16 bytes, usage ${String(BUFFER.UNIFORM | BUFFER.COPY_DST)}`,
      `buffer 5: ${String(pixels * 4)} bytes, usage ${String(BUFFER.STORAGE | BUFFER.COPY_SRC)}`,
      "write buffer 3 at 0: 510 words from 0",
      // 57 600 pixels, 900 workgroups of 64 in one row.
      "write buffer 4 at 0: [57600, 57600, 0, 0]",
      "group of pipeline module square shader layout 0: 0 buffer 1, 1 buffer 2",
      "group of pipeline module display shader layout 0: 0 buffer 4, 1 buffer 2, 2 buffer 3, 3 buffer 5",
      "pop internal",
      "pop out-of-memory",
      "pop validation",
      `configure device, rgba8unorm, opaque, usage ${String(TEXTURE_COPY_DST)}`,
    ]);
    gpuLog.length = 0;
    draw.draw(7n);
    draw.draw(8n);
    const image = (index: number, texture: number): string[] => [
      `write buffer 1 at 0: [${String(index)}, ${String(index + 1)}, 2, 3, 320, 0, 180, 0]`,
      "encoder",
      "  pass",
      "  pipeline pipeline module square shader",
      "  group 0 group of pipeline module square shader layout 0",
      // 40 × 23 workgroups of 8 × 8 cover 320 × 180.
      "  dispatch 40 x 23",
      "  end",
      "  pass",
      "  pipeline pipeline module display shader",
      "  group 0 group of pipeline module display shader layout 0",
      "  dispatch 900 x 1",
      "  end",
      `  copy buffer 5, rows of 1280 bytes, to texture ${String(texture)}, 320 x 180`,
      "submit commands",
    ];
    expect(gpuLog).toStrictEqual([...image(7, 1), ...image(8, 2)]);
    expect([BUFFER, TEXTURE_COPY_DST, ROW_ALIGNMENT]).toStrictEqual([{ COPY_SRC: 4, COPY_DST: 8, UNIFORM: 64, STORAGE: 128 }, 2, 256]);
    draw.release();
  });

  test("covers an image whose sides are not multiples of a workgroup's", async () => {
    const draw = drawn(await gpuSquare(fakeGpu(new FakeDevice("device")), () => new FakeCanvas(), SOURCE, { width: 64, height: 9 }, NO_ENDING));
    expect(gpuLog).toContain("write buffer 4 at 0: [576, 576, 0, 0]");
    gpuLog.length = 0;
    draw.draw(0n);
    expect(gpuLog.filter((line) => line.startsWith("  dispatch"))).toStrictEqual(["  dispatch 8 x 2", "  dispatch 9 x 1"]);
    draw.release();
  });

  test("says there is no GPU to give, without asking for more than it needs, the device destroyed", async () => {
    const canvas = new FakeCanvas();
    const cases: readonly (readonly [string, () => Promise<GpuSetup>, readonly string[]])[] = [
      ["this browser has no WebGPU", () => gpuSquare(undefined, () => canvas, SOURCE, SIZE, NO_ENDING), []],
      ["rows of 400 bytes, not a multiple of the 256 a copy to a canvas takes", () => gpuSquare(fakeGpu(new FakeDevice("device")), () => canvas, SOURCE, { width: 100, height: 9 }, NO_ENDING), []],
      ["no adapter", () => gpuSquare(fakeGpu(null), () => canvas, SOURCE, SIZE, NO_ENDING), ["adapter"]],
      ["the adapter request was rejected: Error: no adapter for this page", () => gpuSquare(fakeGpu(new FakeDevice("device"), undefined, true), () => canvas, SOURCE, SIZE, NO_ENDING), ["adapter"]],
      ["the canvas refused a WebGPU context", () => gpuSquare(fakeGpu(new FakeDevice("device")), () => null, SOURCE, SIZE, NO_ENDING), ["adapter", "device", "destroy device"]],
      ["the page no longer shows the square", () => gpuSquare(fakeGpu(new FakeDevice("device")), () => canvas, SOURCE, SIZE, NO_ENDING, () => false), ["adapter", "device", "destroy device"]],
    ];
    for (const [reason, setup, asked] of cases) {
      gpuLog.length = 0;
      expect(why(await setup()), reason).toStrictEqual({ kind: "absent", reason });
      expect(gpuLog.filter((line) => ["adapter", "device", "destroy device", "configure"].some((start) => line.startsWith(start))), `${reason}: what was asked`).toStrictEqual(asked);
    }
  });

  test("gives the canvas to the CPU, the device destroyed, whenever the GPU's setup fails, and says why", async () => {
    /** A device broken by `breaking`. */
    const broken = (breaking: (device: FakeDevice) => void): FakeDevice => {
      const device = new FakeDevice("device");
      breaking(device);
      return device;
    };
    const cases: readonly (readonly [Refusal, FakeDevice, boolean])[] = [
      [{ kind: "defect", reason: "a pipeline was refused: Error: the shader does not compile" }, broken((device) => Object.assign(device, { rejectPipelines: true })), true],
      [{ kind: "defect", reason: "the validation error scope caught: a binding of the wrong size" }, broken((device) => device.refusals.set("validation", "a binding of the wrong size")), true],
      [{ kind: "environment", reason: "the out-of-memory error scope caught: no room" }, broken((device) => device.refusals.set("out-of-memory", "no room")), true],
      [{ kind: "defect", reason: "the internal error scope caught: no reason given" }, broken((device) => device.refusals.set("internal", "")), true],
      // Scopes popped innermost first: the internal error is the one said.
      [{ kind: "defect", reason: "the internal error scope caught: the driver failed" }, broken((device) => device.refusals.set("out-of-memory", "no room").set("internal", "the driver failed")), true],
      [{ kind: "environment", reason: "900 workgroups in a dispatch, more than the device's 899" }, broken((device) => Object.assign(device, { limits: { ...device.limits, maxComputeWorkgroupsPerDimension: 899 } })), false],
      [{ kind: "environment", reason: `a binding of ${String(320 * 180 * 16)} bytes, more than the device's ${String(320 * 180 * 16 - 1)}` }, broken((device) => Object.assign(device, { limits: { ...device.limits, maxStorageBufferBindingSize: 320 * 180 * 16 - 1 } })), false],
      [{ kind: "environment", reason: "the device was lost while it was set up: no reason given" }, broken((device) => { device.lose(""); }), true],
      [{ kind: "environment", reason: "the device was lost while it was set up: the GPU process crashed" }, broken((device) => { device.lose("the GPU process crashed"); }), true],
    ];
    for (const [refusal, device, scoped] of cases) {
      gpuLog.length = 0;
      const endings: Ending[] = [];
      expect(why(await gpuSquare(fakeGpu(device), () => new FakeCanvas(), SOURCE, SIZE, (ending) => endings.push(ending))), refusal.reason).toStrictEqual(refusal);
      expect(gpuLog.at(-1), refusal.reason).toBe("destroy device");
      expect(gpuLog.some((line) => line.startsWith("configure")), `${refusal.reason}: the canvas left free`).toBe(false);
      expect(gpuLog.includes("push validation"), `${refusal.reason}: within scopes, or refused before`).toBe(scoped);
      device.raise("after the refusal");
      await settled();
      expect(endings, `${refusal.reason}: never an ending, it never drew`).toStrictEqual([]);
    }
    gpuLog.length = 0;
    expect(why(await gpuSquare(fakeGpu(new FakeDevice("device"), new Error("refused")), () => new FakeCanvas(), SOURCE, SIZE, NO_ENDING))).toStrictEqual({ kind: "environment", reason: "the device was refused: Error: refused" });
    expect(gpuLog, "nothing to destroy").toStrictEqual(["adapter", "device"]);
    const exact = new FakeDevice("device");
    exact.limits = { maxComputeWorkgroupsPerDimension: 900, maxStorageBufferBindingSize: 320 * 180 * 16 };
    drawn(await gpuSquare(fakeGpu(exact), () => new FakeCanvas(), SOURCE, SIZE, NO_ENDING)).release();
  });

  test("destroys the device and throws on what the page module throws", async () => {
    const trap = new Error("the visual module trapped and cannot be used again");
    for (const [what, source] of [
      ["a shader", { ...SOURCE, shader: () => { throw trap; } }],
      ["the thresholds", { ...SOURCE, thresholds: () => { throw trap; } }],
    ] as const satisfies readonly (readonly [string, GpuSource])[]) {
      gpuLog.length = 0;
      await expect(gpuSquare(fakeGpu(new FakeDevice("device")), () => new FakeCanvas(), source, SIZE, NO_ENDING), what).rejects.toBe(trap);
      expect(gpuLog.at(-1), what).toBe("destroy device");
    }
  });

  test("ends the drawing once, failed, when the GPU raises an error while it draws", async () => {
    for (const [message, reason] of [["a texture of the wrong size", "a texture of the wrong size"], ["", "no reason given"]] as const) {
      const device = new FakeDevice("device");
      const endings: Ending[] = [];
      const drawing = drawn(await gpuSquare(fakeGpu(device), () => new FakeCanvas(), SOURCE, SIZE, (ending) => endings.push(ending)));
      device.raise(message);
      device.raise("a second error");
      device.lose("lost after");
      await settled();
      expect(endings, message).toStrictEqual([{ kind: "failed", reason }]);
      drawing.release();
    }
  });

  test("ends the drawing once, lost, with the reason the GPU gave or its absence", async () => {
    for (const [message, reason] of [["the GPU process crashed", "the GPU process crashed"], ["", "no reason given"]] as const) {
      const device = new FakeDevice("device");
      const endings: Ending[] = [];
      const drawing = drawn(await gpuSquare(fakeGpu(device), () => new FakeCanvas(), SOURCE, SIZE, (ending) => endings.push(ending)));
      device.lose(message);
      await settled();
      device.raise("an error after the loss");
      expect(endings, message).toStrictEqual([{ kind: "lost", reason }]);
      drawing.release();
    }
  });

  test("never reports the end of a drawing the page released itself", async () => {
    const device = new FakeDevice("device");
    const endings: Ending[] = [];
    const drawing = drawn(await gpuSquare(fakeGpu(device), () => new FakeCanvas(), SOURCE, SIZE, (ending) => endings.push(ending)));
    gpuLog.length = 0;
    drawing.release();
    expect(gpuLog).toStrictEqual(["destroy device"]);
    // A browser loses a device it destroys.
    device.lose("destroyed");
    device.raise("an error after the release");
    await settled();
    expect(endings).toStrictEqual([]);
  });
});

test("the leak law names every device handed to the page and left open, once", async () => {
  // What the law catches: a device the page received and never destroyed, as a setup that
  // threw left one before the page module's throws were caught.
  const open = new FakeDevice("left open");
  const closed = new FakeDevice("closed");
  for (const device of [open, closed]) {
    await (await fakeGpu(device).requestAdapter())?.requestDevice();
  }
  closed.destroy();
  expect(leaked()).toStrictEqual(["left open"]);
  expect(leaked(), "forgotten once named").toStrictEqual([]);
});
