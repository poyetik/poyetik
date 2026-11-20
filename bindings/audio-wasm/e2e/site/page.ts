// The states of the bench page (harness.ts), which runs in a browser: what it shows while
// it loads and while each button runs, measured and mutated here while harness.ts only
// wires them to its elements.

/** Where the page writes text: an `<output>`. */
export interface Output {
  /** The text shown. */
  value: string;
}

/** What starts an action: a `<button>`. */
export interface Control {
  /** Whether it can be clicked. */
  disabled: boolean;
}

/**
 * Loads the page with `load`; when it fails, shows the reason in `element` (the page's
 * error output) and fails with it still, so that the console and the end-to-end tests see
 * the failed load too.
 */
export async function showFailure<T>(load: () => Promise<T>, element: Output): Promise<T> {
  try {
    return await load();
  } catch (reason) {
    element.value = String(reason);
    throw reason;
  }
}

/** Runs an action of the page; never rejects: a failure is shown. */
export type Run = (action: () => Promise<void>) => Promise<void>;

/**
 * The buttons of the page and what they show: while an action runs, every button is
 * disabled and a start is refused (an action never overlaps another, which would distort
 * the times they measure and show one's failure under the other), its outputs and the
 * page's error are cleared (nothing stale stays shown, whatever ran before); a failure is
 * shown in the error; the buttons are enabled again however it ends.
 */
export class Actions {
  /** The page's error output. */
  readonly #error: Output;
  /** The button of every action. */
  readonly #controls: Control[] = [];
  /** Whether an action runs. */
  #running = false;

  /**
   * The actions of a page showing their failures in `error`.
   * @param error The page's error output.
   */
  constructor(error: Output) {
    this.#error = error;
  }

  /**
   * Adds the action `control` starts, showing its results in `outputs`.
   * @param control The button.
   * @param outputs Where the action shows its results.
   * @returns What runs the action, in the states above.
   */
  add(control: Control, outputs: readonly Output[]): Run {
    this.#controls.push(control);
    return (action) => this.#run(outputs, action);
  }

  /** Runs `action`, showing its results in `outputs`, unless an action runs. */
  async #run(outputs: readonly Output[], action: () => Promise<void>): Promise<void> {
    if (this.#running) {
      return;
    }
    this.#running = true;
    this.#enable(false);
    for (const output of outputs) {
      output.value = "";
    }
    this.#error.value = "";
    try {
      await action();
    } catch (reason) {
      this.#error.value = String(reason);
    } finally {
      this.#running = false;
      this.#enable(true);
    }
  }

  /** Enables or disables every button. */
  #enable(enabled: boolean): void {
    for (const control of this.#controls) {
      control.disabled = !enabled;
    }
  }
}

/** What the page closes once its work ends: an `AudioContext`. */
export interface Closable {
  /** The state of the context; `"closed"` once closed, by its owner or the browser. */
  readonly state: string;
  /** Closes the context; rejects when it is already closed. */
  close(): Promise<void>;
}

/**
 * Runs `work` on `context`, then closes the context unless it is already closed (the
 * browser may close it, and closing a closed context rejects); a failed close never
 * replaces the failure of `work`, which is what the page shows.
 * @throws {unknown} what `work` throws; after a successful work, what the close throws.
 */
export async function closing<C extends Closable, T>(context: C, work: (context: C) => Promise<T>): Promise<T> {
  let result: T;
  try {
    result = await work(context);
  } catch (reason) {
    if (context.state !== "closed") {
      // Dropped: the work's own failure is the one thrown; a context that cannot close has nothing left to do.
      await context.close().catch((_unclosable: unknown) => undefined);
    }
    throw reason;
  }
  if (context.state !== "closed") {
    await context.close();
  }
  return result;
}

/** The first failure an engine node reports, kept to explain what it caused. */
export class FirstFailure {
  /** The failure reported first, if any. */
  #first: Error | undefined;

  /** Records `failure` unless one came first: an engine node's `onFailure`. */
  readonly report = (failure: Error): void => {
    this.#first ??= failure;
  };

  /** The failure reported first, if any. */
  get error(): Error | undefined {
    return this.#first;
  }
}

/**
 * Starts the context with `start`, then awaits the engine's `ready` and its capture
 * `captured`: `captured` is handled on every path (a failed start or `ready` leaves no
 * unhandled rejection behind), and a failure the engine reported is thrown in preference
 * to what it caused (the capture's end, `ready`'s rejection).
 * @throws {unknown} the engine's first failure, else what `start`, `ready` or `captured` throws.
 */
export async function liveCapture<T>(ready: Promise<unknown>, captured: Promise<T>, failure: FirstFailure, start: () => Promise<void>): Promise<T> {
  // Not dropped: `captured` is awaited below, its rejection thrown there; this only marks it handled meanwhile.
  captured.catch((_awaitedBelow: unknown) => undefined);
  try {
    await start();
    await ready;
    return await captured;
  } catch (reason) {
    throw failure.error ?? reason;
  }
}
