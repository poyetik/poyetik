import { expect, test, vi } from "vitest";
import type * as Inputs from "../src/inputs.ts";
import { judge } from "../src/oracle.ts";

// A case reader that fails as a bug would: with an error that is not an input error.
vi.mock("../src/inputs.ts", async (original) => ({
  ...(await original<typeof Inputs>()),
  parseCase: (): never => {
    throw new RangeError("boom");
  },
}));

test("lets an error of the case reader that is not an input error through", () => {
  const sine = { sine: { frequency: 1000, amplitude: 1, phase: 0 } };
  expect(() =>
    judge({
      cases: new Map([["sine", "{}"]]),
      references: { sine: { binary64: "0".repeat(64), "binary64-as-binary32": "0".repeat(64), binary32: "0".repeat(64) } },
      expectations: { sine: { sample_rate: 8000, frames: 64, channels: [sine] } },
      wavs: new Map(),
    }),
  ).toThrow(new RangeError("boom"));
});
