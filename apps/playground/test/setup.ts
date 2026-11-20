// Unmounts what each test rendered, then holds the leak law of the fake GPU devices (once
// unmounted, and at every failure the square says), and gives jsdom the media queries and
// the `ImageData` it lacks.
import { cleanup } from "@testing-library/react";
import { afterEach, beforeEach, expect, vi } from "vitest";
import { leaked, openDevices, watchFailures } from "./gpu-fakes.ts";
import { FakeImageData } from "./image-data.ts";
import { FakeMediaQuery, matchMedia, media } from "./media.ts";

/** The failures the current test's square said, watched for the devices they held. */
let failures: ReturnType<typeof watchFailures> | undefined;

beforeEach(() => {
  failures = watchFailures(document.body, openDevices);
  // jsdom has no `matchMedia`: a system asking for nothing, which a test may change.
  media.list = new FakeMediaQuery();
  FakeMediaQuery.asked.length = 0;
  vi.stubGlobal("matchMedia", matchMedia);
  vi.stubGlobal("ImageData", FakeImageData);
});

afterEach(() => {
  // The leak law at every failure: a square that says it failed holds no GPU device.
  const held = failures?.held() ?? [];
  failures?.disconnect();
  expect(held, "GPU devices held by a failed square").toStrictEqual([]);
  cleanup();
  // The leak law: every GPU device handed to the page is destroyed once nothing shows it.
  expect(leaked(), "GPU devices left open").toStrictEqual([]);
});
