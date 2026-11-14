import { describe, expect, test, vi } from "vitest";
import { Actions, closing, FirstFailure, liveCapture, showFailure, type Closable, type Control, type Output } from "../e2e/site/page.ts";

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
  test("disables every control and clears its outputs and the error while it runs, then enables them again", async () => {
    const control: Control = { disabled: false };
    const other: Control = { disabled: false };
    const outputs = [output("an old hash"), output("old times")];
    const otherOutput = output("another hash");
    const error = output("an old error");
    const actions = new Actions(error);
    const run = actions.add(control, outputs);
    actions.add(other, [otherOutput]);
    const pending = held();
    const running = run(() => pending.promise);
    expect([control.disabled, other.disabled, outputs.map((shown) => shown.value), otherOutput.value, error.value]).toStrictEqual([true, true, ["", ""], "another hash", ""]);
    pending.resolve();
    await running;
    expect([control.disabled, other.disabled, error.value]).toStrictEqual([false, false, ""]);
  });

  test("shows why it failed, and enables every control again", async () => {
    const control: Control = { disabled: false };
    const other: Control = { disabled: false };
    const error = output();
    const actions = new Actions(error);
    const run = actions.add(control, []);
    actions.add(other, []);
    const pending = held();
    const running = run(() => pending.promise);
    expect([control.disabled, other.disabled]).toStrictEqual([true, true]);
    pending.reject(new Error("unknown box type: nope~"));
    await expect(running).resolves.toBeUndefined();
    expect([control.disabled, other.disabled, error.value]).toStrictEqual([false, false, "Error: unknown box type: nope~"]);
  });

  test("leaves no error of an earlier failure after a later success", async () => {
    const error = output();
    const run = new Actions(error).add({ disabled: false }, []);
    await run(() => Promise.reject(new Error("first")));
    expect(error.value).toBe("Error: first");
    await run(() => Promise.resolve());
    expect(error.value).toBe("");
  });

  test("never overlaps another action: a start while one runs does nothing, and the first keeps its error", async () => {
    const control: Control = { disabled: false };
    const other: Control = { disabled: false };
    const otherOutput = output("another hash");
    const error = output();
    const actions = new Actions(error);
    const run = actions.add(control, []);
    const runOther = actions.add(other, [otherOutput]);
    const pending = held();
    const running = run(() => pending.promise);
    let started = false;
    await runOther(() => {
      started = true;
      return Promise.resolve();
    });
    expect([started, control.disabled, other.disabled, otherOutput.value]).toStrictEqual([false, true, true, "another hash"]);
    pending.reject(new Error("first"));
    await running;
    expect([control.disabled, other.disabled, error.value]).toStrictEqual([false, false, "Error: first"]);
    await runOther(() => {
      started = true;
      return Promise.resolve();
    });
    expect([started, error.value]).toStrictEqual([true, ""]);
  });
});

/** A fake audio context: its state, and what closing it does (rejecting once closed, as Web Audio does). */
class FakeContext implements Closable {
  /** The state of the context. */
  state = "running";
  /** How many times it was closed. */
  closes = 0;
  /** Why its close fails while it is open, if it does. */
  refuses: Error | undefined;
  /** Closes the context; rejects when it is already closed, or when it refuses. */
  close(): Promise<void> {
    this.closes += 1;
    if (this.state === "closed") {
      return Promise.reject(new DOMException("already closed", "InvalidStateError"));
    }
    if (this.refuses !== undefined) {
      return Promise.reject(this.refuses);
    }
    this.state = "closed";
    return Promise.resolve();
  }
}

describe("a context the bench page opens", () => {
  test("is closed after the work, which gives its result", async () => {
    const context = new FakeContext();
    await expect(closing(context, (given) => Promise.resolve(given === context ? 7 : 0))).resolves.toBe(7);
    expect([context.state, context.closes]).toStrictEqual(["closed", 1]);
  });

  test("is closed after a failed work, whose failure is kept", async () => {
    const context = new FakeContext();
    const reason = new Error("the engine failed: trap");
    await expect(closing(context, () => Promise.reject(reason))).rejects.toBe(reason);
    expect([context.state, context.closes]).toStrictEqual(["closed", 1]);
  });

  test("is not closed again when the browser closed it: the work's failure is shown, not the close's", async () => {
    const context = new FakeContext();
    const reason = new Error("the context closed before the end of the capture");
    await expect(
      closing(context, () => {
        context.state = "closed";
        return Promise.reject(reason);
      }),
    ).rejects.toBe(reason);
    expect(context.closes).toBe(0);
  });

  test("is not closed again after a work that succeeds once the browser closed it", async () => {
    const context = new FakeContext();
    await expect(
      closing(context, () => {
        context.state = "closed";
        return Promise.resolve("done");
      }),
    ).resolves.toBe("done");
    expect(context.closes).toBe(0);
  });

  test("keeps the work's failure when the close fails too", async () => {
    const context = new FakeContext();
    context.refuses = new Error("cannot close");
    const reason = new Error("first");
    await expect(closing(context, () => Promise.reject(reason))).rejects.toBe(reason);
    expect(context.closes).toBe(1);
  });

  test("fails with the close's failure after a work that succeeded", async () => {
    const context = new FakeContext();
    const refused = new Error("cannot close");
    context.refuses = refused;
    await expect(closing(context, () => Promise.resolve(1))).rejects.toBe(refused);
  });
});

describe("the first failure of an engine", () => {
  test("is kept: a later one does not replace it", () => {
    const failure = new FirstFailure();
    expect(failure.error).toBeUndefined();
    const first = new Error("first");
    failure.report(first);
    failure.report(new Error("second"));
    expect(failure.error).toBe(first);
  });
});

describe("a live capture of the bench page", () => {
  test("gives the capture once the engine is ready", async () => {
    const steps: string[] = [];
    const channels = await liveCapture(Promise.resolve(), Promise.resolve(["a channel"]), new FirstFailure(), () => {
      steps.push("start");
      return Promise.resolve();
    });
    expect([channels, steps]).toStrictEqual([["a channel"], ["start"]]);
  });

  test("fails with ready's reason, the capture's later failure handled: no unhandled rejection", async () => {
    const captured = held();
    const handled = vi.spyOn(captured.promise, "catch");
    const reason = new Error("the engine processor failed");
    const capturing = liveCapture(Promise.reject(reason), captured.promise, new FirstFailure(), () => Promise.resolve());
    await expect(capturing).rejects.toBe(reason);
    // Seen by its handler, not only by Vitest's report of an unhandled rejection, which a
    // mutant would turn into a crash of the run.
    expect(handled).toHaveBeenCalledTimes(1);
    captured.reject(new Error("the context closed before the end of the capture"));
    await new Promise((resolve) => setImmediate(resolve));
  });

  test("fails with the start's reason, the capture's later failure handled", async () => {
    const captured = held();
    const handled = vi.spyOn(captured.promise, "catch");
    const reason = new Error("cannot resume");
    await expect(liveCapture(held().promise, captured.promise, new FirstFailure(), () => Promise.reject(reason))).rejects.toBe(reason);
    expect(handled).toHaveBeenCalledTimes(1);
    captured.reject(new Error("the context closed before the end of the capture"));
    await new Promise((resolve) => setImmediate(resolve));
  });

  test("shows the engine's own failure rather than the capture's end it caused", async () => {
    const failure = new FirstFailure();
    const engine = new Error("the engine failed: trap");
    failure.report(engine);
    await expect(liveCapture(Promise.resolve(), Promise.reject(new Error("the source stopped before the end of the capture")), failure, () => Promise.resolve())).rejects.toBe(engine);
  });
});
