// Unmounts what each test rendered, then holds the leak law of the fake GPU devices, and gives jsdom the media queries and the `ImageData`
// it lacks.
import { cleanup } from "@testing-library/react";
import { afterEach, beforeEach, expect, vi } from "vitest";
import { leaked } from "./gpu-fakes.ts";
import { FakeImageData } from "./image-data.ts";
import { FakeMediaQuery, matchMedia, media } from "./media.ts";

beforeEach(() => {
  // jsdom has no `matchMedia`: a system asking for nothing, which a test may change.
  media.list = new FakeMediaQuery();
  FakeMediaQuery.asked.length = 0;
  vi.stubGlobal("matchMedia", matchMedia);
  vi.stubGlobal("ImageData", FakeImageData);
});

afterEach(() => {
  cleanup();
  // The leak law: every GPU device handed to the page is destroyed once nothing shows it.
  expect(leaked(), "GPU devices left open").toStrictEqual([]);
});
