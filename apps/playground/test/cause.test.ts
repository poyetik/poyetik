// The law of the remedies (`gates/remedies.ts`) over the page's remedies, and its fixtures.
import { expect, test } from "vitest";
import { SOUND_WORDS, SQUARE_WORDS, WORDS, type Words, faults, misfits } from "../gates/remedies.ts";
import type { Cause } from "../src/cause.ts";
import { SOUND_REMEDY, explain, type Failure } from "../src/player.ts";
import { REMEDY, type SquareFailure, presentation } from "../src/Square.tsx";

/** Every cause: a record, so that a cause added does not compile without it. */
const CAUSES: Readonly<Record<Cause, Cause>> = { network: "network", environment: "environment", defect: "defect" };

/** Every failure of the sound, likewise. */
const FAILURES: Readonly<Record<Failure, Failure>> = { load: "load", module: "module", engine: "engine", device: "device" };

/** Every failure of the square, likewise. */
const SQUARE_FAILURES: Readonly<Record<SquareFailure, SquareFailure>> = { download: "download", module: "module", defect: "defect", canvas: "canvas" };

test("gives each failure the remedy of its cause, the square's and the sound's alike", () => {
  expect(Object.values(CAUSES).flatMap((cause) => [...faults(cause, REMEDY[cause]), ...faults(cause, SOUND_REMEDY[cause])])).toStrictEqual([]);
});

test("explains each failure by its whole line, never by words of another failure: the square's held by cause, its module and canvas sharing one text", () => {
  expect(Object.values(FAILURES).flatMap((member) => misfits(member, SOUND_WORDS[member], explain(member)))).toStrictEqual([]);
  // The line the square shows, before an image was drawn and after.
  const lines = Object.values(SQUARE_FAILURES).flatMap((member) => [false, true].map((drawn) => [member, presentation({ kind: "failed", stage: member, drawn }, false, "playing").line] as const));
  expect(lines).toHaveLength(8);
  expect(lines.flatMap(([member, line]) => misfits(member, SQUARE_WORDS[member], line))).toStrictEqual([]);
  expect(SQUARE_WORDS.module, "one text by cause").toStrictEqual(SQUARE_WORDS.canvas);
});

test("finds a clause of the square's line outside its remedy that another cause's member would say (sixth deep review of visual block 2)", () => {
  expect(misfits("module", SQUARE_WORDS.module, "The square cannot be shown: check your connection. Try another browser or device; its details say what failed.")).toStrictEqual(['module: says "connection"']);
  expect(misfits("download", SQUARE_WORDS.download, "The square cannot be shown in this browser. Reload the page; if it fails again, check your connection.")).toStrictEqual(['download: says "browser"']);
  expect([misfits("load", SOUND_WORDS.load, "Reload the page."), misfits("load", SOUND_WORDS.load, "The download failed.")], "a word inside another").toStrictEqual([['load: lacks "could not load"'], ['load: lacks "could not load"']]);
});

test("requires and refuses the words of each row: one that requires nothing holds no text to anything", () => {
  /** The words of every row of `table`, its misfits with a text saying none of them, and with one saying them all. */
  const held = (table: Readonly<Record<string, Words>>): readonly unknown[] =>
    Object.entries(table).map(([member, words]) => [member, misfits(member, words, "."), misfits(member, words, [...words.says, ...words.never].join(" ").toUpperCase())]);
  expect(held(WORDS)).toStrictEqual([
    ["network", ['network: lacks "again"', 'network: lacks "connection"'], ['network: says "browser"', 'network: says "details"']],
    ["environment", ['environment: lacks "try"', 'environment: lacks "another"'], ['environment: says "reload"', 'environment: says "connection"']],
    ["defect", ['defect: lacks "again"', 'defect: lacks "details"'], ['defect: says "browser"', 'defect: says "connection"', 'defect: says "output"']],
  ]);
  expect(held(SOUND_WORDS)).toStrictEqual([
    ["load", ['load: lacks "could not load"'], ['load: says "output"', 'load: says "compile"']],
    ["module", ['module: lacks "compile"'], ['module: says "output"', 'module: says "connection"']],
    ["engine", ['engine: lacks "stopped"'], ['engine: says "output"', 'engine: says "compile"']],
    ["device", ['device: lacks "output"'], ['device: says "compile"', 'device: says "connection"']],
  ]);
  expect(held(SQUARE_WORDS)).toStrictEqual([
    ["download", ['download: lacks "connection"'], ['download: says "output"', 'download: says "start"', 'download: says "browser"']],
    ["module", ['module: lacks "browser"'], ['module: says "output"', 'module: says "start"', 'module: says "connection"']],
    ["defect", ['defect: lacks "details"'], ['defect: says "output"', 'defect: says "start"', 'defect: says "browser"', 'defect: says "connection"']],
    ["canvas", ['canvas: lacks "browser"'], ['canvas: says "output"', 'canvas: says "start"', 'canvas: says "connection"']],
  ]);
});

test("finds the remedy the fifth deep review of visual block 2 found: a module this browser cannot compile told to check the sound output", () => {
  expect(misfits("module", SOUND_WORDS.module, "This browser could not compile the audio engine. Check the sound output and press Start to try again; if it fails again, try another browser or device.")).toStrictEqual(['module: says "output"']);
  expect(faults("environment", "Press Start to try again."), "a retry alone").toStrictEqual(['environment: lacks "another"']);
});

test("finds the remedies the third deep review of visual block 2 found: a defect told to reload, an environment told nothing to do", () => {
  // The sound's defect sent to a reload, never to the details its remedy's cause promises;
  // the square's environment said what failed, and nothing to try.
  expect(faults("defect", "Press Start to try again; if it happens again, reload the page.")).toStrictEqual(['defect: lacks "details"']);
  expect(faults("environment", "This browser cannot show it; its details say what failed.")).toStrictEqual(['environment: lacks "try"', 'environment: lacks "another"']);
});

test("finds the remedies the second deep review of visual block 2 found: by stage, a defect told to try another browser", () => {
  // The module stage held the page's own defects, with this browser's remedy; and the
  // download's remedy sent to the connection first, true of no server's error.
  expect(faults("defect", "This browser cannot show it; its details say what failed.")).toStrictEqual(['defect: lacks "again"', 'defect: says "browser"']);
  expect(faults("network", "Check your connection, then reload the page.")).toStrictEqual(['network: lacks "again"']);
  expect(faults("environment", "Reload the page; if it fails again, check your connection.")).toStrictEqual(['environment: lacks "try"', 'environment: lacks "another"', 'environment: says "reload"', 'environment: says "connection"']);
});
