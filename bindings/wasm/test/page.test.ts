import { describe, expect, test } from "vitest";
import { Action, showFailure, type Control, type Output } from "../e2e/site/page.ts";

/** A plain output, as the page's `<output>` elements are seen. */
function output(value = ""): Output {
  return { value };
}

/** A promise that settles only when the test says, and its two settlers. */
function held(): { readonly promise: Promise<void>; resolve: () => void; reject: (reason: unknown) => void } {
  let resolve: () => void = () => undefined;
  let reject: (reason: unknown) => void = () => undefined;
  const promise = new Promise<void>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { promise, resolve, reject };
}

describe("the bench page's loading", () => {
  test("shows why the page could not load, and still fails the load with that reason", async () => {
    const error = output();
    const reason = new Error("GET /conformance/cases/missing-case.json: 404");
    await expect(showFailure(() => Promise.reject(reason), error)).rejects.toBe(reason);
    expect(error.value).toBe("Error: GET /conformance/cases/missing-case.json: 404");
  });

  test("shows nothing, and gives what it loaded, when the load succeeds", async () => {
    const error = output("left from before");
    await expect(showFailure(() => Promise.resolve(42), error)).resolves.toBe(42);
    expect(error.value).toBe("left from before");
  });
});

describe("an action of the bench page", () => {
  test("disables its control and clears its outputs and the error while it runs, then enables it again", async () => {
    const control: Control = { disabled: false };
    const outputs = [output("an old hash"), output("old times")];
    const error = output("an old error");
    const pending = held();
    const running = new Action(control, outputs, error).run(() => pending.promise);
    expect([control.disabled, outputs.map((shown) => shown.value), error.value]).toStrictEqual([true, ["", ""], ""]);
    pending.resolve();
    await running;
    expect([control.disabled, error.value]).toStrictEqual([false, ""]);
  });

  test("shows why it failed, and enables its control again", async () => {
    const control: Control = { disabled: false };
    const error = output();
    const pending = held();
    const running = new Action(control, [], error).run(() => pending.promise);
    expect(control.disabled).toBe(true);
    pending.reject(new Error("unknown box type: nope~"));
    await expect(running).resolves.toBeUndefined();
    expect([control.disabled, error.value]).toStrictEqual([false, "Error: unknown box type: nope~"]);
  });

  test("leaves no error of an earlier failure after a later success", async () => {
    const error = output();
    const action = new Action({ disabled: false }, [], error);
    await action.run(() => Promise.reject(new Error("first")));
    expect(error.value).toBe("Error: first");
    await action.run(() => Promise.resolve());
    expect(error.value).toBe("");
  });
});
