// Unmounts what each test rendered, and gives jsdom the media queries it lacks.
import { cleanup } from "@testing-library/react";
import { afterEach, beforeEach, vi } from "vitest";
import { FakeMediaQuery, matchMedia, media } from "./media.ts";

beforeEach(() => {
  // jsdom has no `matchMedia`: a system asking for nothing, which a test may change.
  media.list = new FakeMediaQuery();
  FakeMediaQuery.asked.length = 0;
  vi.stubGlobal("matchMedia", matchMedia);
});

afterEach(() => {
  cleanup();
});
