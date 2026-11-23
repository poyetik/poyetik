// The law of the remedies (`gates/remedies.ts`) over the page's remedies, and its fixtures.
import { expect, test } from "vitest";
import { WORDS, faults, isCause } from "../gates/remedies.ts";
import { SOUND_REMEDY } from "../src/player.ts";
import { REMEDY } from "../src/Square.tsx";

test("gives each failure the remedy of its cause, the square's and the sound's alike", () => {
  for (const remedies of [REMEDY, SOUND_REMEDY]) {
    expect(Object.entries(remedies).flatMap(([cause, remedy]) => (isCause(cause) ? faults(cause, remedy) : [`not a cause: ${cause}`]))).toStrictEqual([]);
  }
});

test("requires of every cause a word its remedy must say: a row that requires nothing holds no remedy to anything", () => {
  expect(Object.entries(WORDS).filter(([, { says }]) => says.length === 0).map(([cause]) => cause)).toStrictEqual([]);
});

test("finds the remedies review 3 left: a defect told to reload, an environment told nothing to do", () => {
  // The sound's defect sent to a reload, never to the details its remedy's cause promises;
  // the square's environment said what failed, and nothing to try.
  expect(faults("defect", "Press Start to try again; if it happens again, reload the page.")).toStrictEqual(['defect: lacks "details"']);
  expect(faults("environment", "This browser cannot show it; its details say what failed.")).toStrictEqual(['environment: lacks "try"']);
});

test("finds the remedies review 2 left: by stage, a defect told to try another browser", () => {
  // The module stage held the page's own defects, with this browser's remedy; and the
  // download's remedy sent to the connection first, true of no server's error.
  expect(faults("defect", "This browser cannot show it; its details say what failed.")).toStrictEqual(['defect: lacks "again"', 'defect: says "browser"']);
  expect(faults("network", "Check your connection, then reload the page.")).toStrictEqual(['network: lacks "again"']);
  expect(faults("environment", "Reload the page; if it fails again, check your connection.")).toStrictEqual(['environment: lacks "try"', 'environment: says "reload"', 'environment: says "connection"']);
});
