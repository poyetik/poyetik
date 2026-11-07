import { expect, test } from "vitest";
import { NONE, formatFrequency, formatPeak, formatRate } from "../src/readout.ts";

const square = { samples: new Float32Array([-1, 0.5, 0.5, -1, -1, 0.5, 0.5, -1, -1, 0.5]), rate: 1762 };

test("shows the frequency to a tenth of a hertz", () => {
  expect(formatFrequency(square)).toBe("440.5 Hz");
  expect(formatFrequency({ samples: new Float32Array(4), rate: 48_000 })).toBe(NONE);
  expect(formatFrequency(undefined)).toBe(NONE);
});

test("shows the peak to a hundredth", () => {
  expect(formatPeak({ samples: new Float32Array([0.5, -0.999]), rate: 1 })).toBe("1.00");
  expect(formatPeak({ samples: new Float32Array([0.25]), rate: 1 })).toBe("0.25");
  expect(formatPeak({ samples: new Float32Array(), rate: 1 })).toBe(NONE);
  expect(formatPeak(undefined)).toBe(NONE);
});

test("shows the sample rate", () => {
  expect(formatRate({ samples: new Float32Array(), rate: 44_100 })).toBe("44100 Hz");
  expect(formatRate(undefined)).toBe(NONE);
});

test("shows a dash when there is nothing", () => {
  expect(NONE).toBe("—");
});
