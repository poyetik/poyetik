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

/**
 * One button of the page and what it shows: while its action runs, the button is disabled
 * (an action never overlaps itself, which would also distort the times it measures), and
 * its outputs and the page's error are cleared (nothing stale stays shown, whatever ran
 * before); a failure is shown in the error; the button is enabled again however it ends.
 */
export class Action {
  /** The button. */
  readonly #control: Control;
  /** What the action shows when it succeeds. */
  readonly #outputs: readonly Output[];
  /** The page's error output. */
  readonly #error: Output;

  /**
   * An action started by `control`, showing its results in `outputs` and its failure in `error`.
   * @param control The button.
   * @param outputs Where the action shows its results.
   * @param error The page's error output.
   */
  constructor(control: Control, outputs: readonly Output[], error: Output) {
    this.#control = control;
    this.#outputs = outputs;
    this.#error = error;
  }

  /** Runs `action` in the states above; never rejects: a failure is shown. */
  async run(action: () => Promise<void>): Promise<void> {
    this.#control.disabled = true;
    for (const output of this.#outputs) {
      output.value = "";
    }
    this.#error.value = "";
    try {
      await action();
    } catch (reason) {
      this.#error.value = String(reason);
    } finally {
      this.#control.disabled = false;
    }
  }
}
