import { expect, test } from "vitest";
import { SampleHistory } from "../src/history.ts";

test("starts empty", () => {
  expect(new SampleHistory(4).snapshot()).toStrictEqual(new Float32Array());
});

test("keeps samples in order until full", () => {
  const history = new SampleHistory(4);
  history.push(new Float32Array([1, 2]));
  expect(history.snapshot()).toStrictEqual(new Float32Array([1, 2]));
  history.push(new Float32Array([3]));
  expect(history.snapshot()).toStrictEqual(new Float32Array([1, 2, 3]));
});

test("drops the oldest samples once full", () => {
  const history = new SampleHistory(4);
  history.push(new Float32Array([1, 2, 3]));
  history.push(new Float32Array([4, 5, 6]));
  expect(history.snapshot()).toStrictEqual(new Float32Array([3, 4, 5, 6]));
  history.push(new Float32Array([7]));
  expect(history.snapshot()).toStrictEqual(new Float32Array([4, 5, 6, 7]));
});

test("keeps only the end of a render quantum larger than the history", () => {
  const history = new SampleHistory(3);
  history.push(new Float32Array([9]));
  history.push(new Float32Array([1, 2, 3, 4, 5]));
  expect(history.snapshot()).toStrictEqual(new Float32Array([3, 4, 5]));
  history.push(new Float32Array([6, 7, 8]));
  expect(history.snapshot()).toStrictEqual(new Float32Array([6, 7, 8]));
});

test("ignores empty render quanta", () => {
  const history = new SampleHistory(2);
  history.push(new Float32Array([1]));
  history.push(new Float32Array());
  expect(history.snapshot()).toStrictEqual(new Float32Array([1]));
});
