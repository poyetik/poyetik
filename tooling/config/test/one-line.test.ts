import { existsSync, readFileSync } from "node:fs";
import { expect, test } from "vitest";
import { firstUnsafe, MAX_LINE_CHARS, oneLine } from "../one-line.ts";

/** Reads a field of a JSON object. */
function field(value: unknown, name: string): unknown {
  return typeof value === "object" && value !== null ? Reflect.get(value, name) : undefined;
}

/** The text a list of `[text, times]` pieces of the vector file stands for. */
function pieces(value: unknown): string {
  if (!Array.isArray(value)) {
    throw new Error("a list of pieces");
  }
  return value
    .map((piece: unknown) => {
      const text: unknown = field(piece, "0");
      const times: unknown = field(piece, "1");
      if (typeof text !== "string" || typeof times !== "number") {
        throw new Error("a piece is [text, times]");
      }
      return text.repeat(times);
    })
    .join("");
}

/** The shared vector file, found by walking up: the tests also run from a copy of the package, deeper in the repository. */
function vectorsFile(): URL {
  for (let directory = new URL("./", import.meta.url); directory.pathname !== "/"; directory = new URL("../", directory)) {
    const candidate = new URL("spec/safe-message-vectors.json", directory);
    if (existsSync(candidate)) {
      return candidate;
    }
  }
  throw new Error("spec/safe-message-vectors.json not found");
}

/** The shared vectors, also read by the tests of `audio_patch::safe_message`. */
const VECTORS: unknown = JSON.parse(readFileSync(vectorsFile(), "utf8"));

test("reproduces the shared vectors of the Rust implementation", () => {
  expect(field(VECTORS, "max_chars")).toBe(1000);
  const vectors = field(VECTORS, "vectors");
  expect(Array.isArray(vectors) && vectors.length).toBe(9);
  for (const vector of Array.isArray(vectors) ? vectors : []) {
    expect(oneLine(pieces(field(vector, "input"))), String(field(vector, "name"))).toBe(pieces(field(vector, "output")));
  }
});

test("bounds a line to the documented length", () => {
  expect(MAX_LINE_CHARS).toBe(1000);
});

test("finds the first character a line must not hold", () => {
  expect(firstUnsafe("plain é 音 𝄞  ")).toBeUndefined();
  expect(firstUnsafe("")).toBeUndefined();
  expect(firstUnsafe("a‮b\u001bc")).toBe("‮");
  expect(firstUnsafe("title\n")).toBe("\n");
  // Every range, at both ends, and the neighbours outside.
  for (const character of ["\u0000", "\u001f", "\u007f", "\u009f", "؜", "‎", "‏", " ", " ", "‪", "‮", "⁦", "⁩"]) {
    expect(firstUnsafe(`x${character}`)).toBe(character);
  }
  for (const character of [" ", "~", " ", "؛", "؝", "‍", "‐", "‧", " ", "⁥", "⁪"]) {
    expect(firstUnsafe(character)).toBeUndefined();
  }
});
