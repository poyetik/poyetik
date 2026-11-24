/**
 * The square shown with the sound: a canvas showing, before every repaint, the image of the
 * visual library's square that the audio clock is at, outside React's rendering, plus a
 * line saying what it shows.
 *
 * The image follows the audio clock, never the page's own: the page module answers which
 * image a frame of the clock shows (exactly, `page_image_at`), then renders it encoded for
 * display, and the canvas takes its bytes as they are: drawn on the GPU through WebGPU when
 * the browser has one (the library's shaders, `gpu-square.ts`), on the CPU otherwise,
 * the same bytes either way, and the page says which. Between sessions it shows the first
 * image; when the system asks for reduced motion, it shows the first image only, and says
 * so.
 *
 * A failure is announced (a live region); the switch to the CPU after the GPU stops drawing
 * is not: it is no status the listener needs (WCAG 4.1.3), nothing they could act on, and the
 * square still shows the same images. The line under the square says it to whoever reads it.
 */

import { useCallback, useEffect, useMemo, useState, useSyncExternalStore, type JSX } from "react";
import type { AudioFrame, PlayerState } from "./player.ts";
import { type CanvasGpu, type Draw, type Ending, type Gpu, type GpuSource, type Handles, type Refusal, type WebGpu, gpuSquare } from "./gpu-square.ts";
import type { Cause } from "./cause.ts";
import { failure } from "./failure.ts";
import { animationFrames } from "./Oscilloscope.tsx";
import { SQUARE } from "./settings.ts";
import { type LoadStage, SquareLoadError } from "./square-module.ts";

/** What the square needs of the page module: which image a frame shows, and that image for display. */
export interface SquareImages extends GpuSource {
  /** The image a sequence at `rate` images a second shows at frame `frame` of a clock at `sampleRate` hertz. */
  imageAt(frame: bigint, sampleRate: number, rate: readonly [number, number]): bigint;
  /** Image `index`, four bytes a pixel as `ImageData` holds them, valid until the next call. */
  display(index: bigint): Uint8ClampedArray<ArrayBuffer>;
}

/** What the canvas needs of its 2D context: putting an image's pixels in place. */
export type SquareCanvas = Pick<CanvasRenderingContext2D, "putImageData">;

/** The id of the line saying what the canvas shows, which describes it to assistive technologies. */
const SAYS = "square-says";

/** The media query of a system asking for reduced motion. */
export const REDUCED_MOTION = "(prefers-reduced-motion: reduce)";

/** What the line says while the square sweeps with the sound, or can. */
export const SWEEPING = "The square sweeps across the image with the sound, one sweep every two seconds, on the audio clock.";

/**
 * What the line adds to {@link SWEEPING} in each state of a player that can play: whether the
 * square waits for the sound, or for nothing.
 */
export const WAITING: Readonly<Record<Exclude<PlayerState["kind"], "unsupported">, string>> = {
  stopped: " Still until the sound starts.",
  starting: " Still until the sound starts.",
  playing: "",
  suspended: " Still while the sound is paused.",
  stopping: " Still: the sound stops.",
  failed: " Still until the sound plays again.",
};

/** What the line says, instead of {@link SWEEPING}, in a browser that cannot play the sound: the square never sweeps there. */
export const UNPLAYABLE = "The square sweeps with the sound, which this browser cannot play: it stays still.";

/** The canvas's name in each state of what it shows. */
export const NAMES = {
  loading: "The square, loading",
  switching: "The square, switching to the CPU",
  failed: "The square, which cannot be shown",
  stopped: "The square, stopped on an error",
  still: "A white square on black, still",
  sweeping: "A white square on black, sweeping from left to right",
} as const;

/** What the line says when the system asks for reduced motion. */
export const STILL = "The square stays still: your system asks for reduced motion.";

/** What the line says while the page module loads. */
export const LOADING = "Loading the square…";

/** What the line says while the square switches to the CPU, the GPU having stopped drawing it. */
export const SWITCHING = "Switching the square to the CPU…";

/** What the line says, and the page announces, when the square cannot be shown, before what to do ({@link REMEDY}); what failed is in its details. */
export const FAILED = "The square cannot be shown.";

/** What the line says instead of {@link FAILED} when the square stops on an error after it drew an image, which the canvas keeps: its name's state ({@link NAMES}). */
export const STOPPED = "The square stopped on an error.";

/**
 * Where showing the square failed ({@link LoadStage}): its module's download, its module
 * that this browser cannot run, a defect of the page's own (its module failing to set up, or
 * throwing while it shows the square), or the canvas (no 2D context to draw on).
 */
export type SquareFailure = LoadStage | "canvas";

/** Whose each failure of the square is ({@link Cause}): the download's the network's, the module this browser cannot run and the canvas this browser's, the defect the page's. */
export const CAUSE: Readonly<Record<SquareFailure, Cause>> = { download: "network", module: "environment", defect: "defect", canvas: "environment" };

/**
 * What to do when the square cannot be shown, by the cause of its failure ({@link CAUSE}):
 * never a remedy that cannot help (a reload for what this browser cannot do, another browser
 * for the page's defect), and one that holds for every error that reaches it (a network
 * error and a server's).
 */
export const REMEDY: Readonly<Record<Cause, string>> = {
  network: "Reload the page; if it fails again, check your connection.",
  environment: "Try another browser or device; its details say what failed.",
  defect: "Reload the page; if it happens again, its details say what failed.",
};

/** What the page logs, as an error, when the square cannot be shown, before what failed. */
export const LOGGED = "The square cannot be shown: ";

/** The summary of the details saying what failed, distinct from the sound's. */
export const FAILURE_DETAILS = "Details of the square's failure";

/**
 * What draws the square: the GPU; the CPU; or the CPU since the GPU stopped drawing it, lost
 * or failed ({@link Ending}).
 */
export type DrawnBy = "gpu" | "cpu" | Ending["kind"];

/** What the line under the square says draws it. */
export const DRAWN_BY: Readonly<Record<DrawnBy, string>> = {
  gpu: "Drawn by the GPU, through WebGPU.",
  cpu: "Drawn by the CPU, in WebAssembly.",
  lost: "Drawn by the CPU, in WebAssembly (the GPU was lost).",
  failed: "Drawn by the CPU, in WebAssembly (the GPU failed).",
};

/** What the line under the square says while the square switches to the CPU, until it draws: how the GPU stopped, never that the CPU draws. */
export const SWITCHING_BY: Readonly<Record<Ending["kind"], string>> = {
  lost: "Switching to the CPU (the GPU was lost).",
  failed: "Switching to the CPU (the GPU failed).",
};

/**
 * What the square's module is, as the page tells it: loading, switching to the CPU (the GPU
 * having stopped drawing it, `by` saying how), ready to draw (by what), or failed at a stage,
 * after an image was drawn (`drawn`: the canvas keeps it) or before.
 */
export type ModuleView =
  | { readonly kind: "loading" }
  | { readonly kind: "switching"; readonly by: Ending["kind"] }
  | { readonly kind: "ready"; readonly by: DrawnBy }
  | { readonly kind: "failed"; readonly stage: SquareFailure; readonly drawn: boolean };

/** What the page tells of the square, every text that says its state, which a law holds to agree: the canvas's name, the line saying what it shows, and what draws it. */
export interface Presentation {
  /** The canvas's accessible name. */
  readonly name: string;
  /** The line describing it, announced when it says a failure. */
  readonly line: string;
  /** What draws it, when something does. */
  readonly drawnBy: string | undefined;
}

/**
 * What the page tells of the square, from its whole state: the module, whether the system
 * asks for reduced motion (`still`), and the player's state. The canvas is named sweeping
 * only while it sweeps: the module ready, no reduced motion, the sound playing.
 */
export function presentation(module: ModuleView, still: boolean, state: PlayerState["kind"]): Presentation {
  switch (module.kind) {
    case "loading":
      return { name: NAMES.loading, line: LOADING, drawnBy: undefined };
    case "switching":
      return { name: NAMES.switching, line: SWITCHING, drawnBy: SWITCHING_BY[module.by] };
    case "failed": {
      const [name, says] = module.drawn ? [NAMES.stopped, STOPPED] : [NAMES.failed, FAILED];
      return { name, line: `${says} ${REMEDY[CAUSE[module.stage]]}`, drawnBy: undefined };
    }
    case "ready":
      if (still) {
        return { name: NAMES.still, line: STILL, drawnBy: DRAWN_BY[module.by] };
      }
      return { name: state === "playing" ? NAMES.sweeping : NAMES.still, line: state === "unsupported" ? UNPLAYABLE : `${SWEEPING}${WAITING[state]}`, drawnBy: DRAWN_BY[module.by] };
  }
}

/** The summary of the details saying why the CPU draws the square, when the GPU said why. */
export const WHY_NOT_GPU = "Why not the GPU";

/**
 * The state of the square's module: loading, switching to the CPU (the GPU lost or failed,
 * `by`, and why), ready to draw (by what,
 * whether it drew an image on this canvas yet, why not on the GPU when there is a reason to
 * tell, and how to stop: the setup's own end, which releases what it holds), or failed (at a stage, after an image was drawn or before, with
 * what failed).
 */
type Module =
  | { readonly kind: "loading" }
  | { readonly kind: "switching"; readonly by: Ending["kind"]; readonly why: string }
  | { readonly kind: "ready"; readonly images: SquareImages; readonly draw: Draw; readonly drew: () => boolean; readonly by: DrawnBy; readonly why: string | undefined; readonly stop: () => void }
  | { readonly kind: "failed"; readonly stage: SquareFailure; readonly drawn: boolean; readonly detail: string };

/**
 * Logs why the GPU does not draw the square, by kind: a defect, or an error the GPU raised
 * while drawing, as an error (which fails the end-to-end tests); a limit of this machine or
 * a loss as a warning; nothing for a browser without a GPU to give.
 */
function report(kind: Refusal["kind"] | Ending["kind"], reason: string): void {
  const said = `The square is not drawn on the GPU: ${reason}`;
  if (kind === "defect" || kind === "failed") {
    console.error(said);
  } else if (kind !== "absent") {
    console.warn(said);
  }
}

/** The browser's GPU, when it has WebGPU. */
function browserGpu(): Gpu<WebGpu> | undefined {
  return "gpu" in navigator ? navigator.gpu : undefined;
}

/**
 * The WebGPU context of `canvas`, or none: the DOM types its context as any kind, and only
 * WebGPU's configures a device.
 */
function webgpuContext(canvas: HTMLCanvasElement): CanvasGpu<WebGpu> | null {
  const context = canvas.getContext("webgpu");
  return context !== null && "configure" in context ? context : null;
}

/** Draws on the CPU: the module's bytes put on `drawing` as they are. */
function cpuSquare(drawing: SquareCanvas, images: SquareImages): Draw {
  return (index) => {
    // The module's bytes as they are, a view of its memory: copied once, into the canvas.
    drawing.putImageData(new ImageData(images.display(index), SQUARE.width, SQUARE.height), 0, 0);
  };
}

/** The 2D context of a canvas. */
function context2d(canvas: HTMLCanvasElement): SquareCanvas | null {
  return canvas.getContext("2d");
}

/** What the square reads of a media query list: whether it matches, and its changes. */
export type MediaQuery = Pick<MediaQueryList, "matches" | "addEventListener" | "removeEventListener">;

/** The system's reduced-motion preference, as a media query list. */
function reducedMotionQuery(): MediaQuery {
  return matchMedia(REDUCED_MOTION);
}

/** Whether the system asks for reduced motion, following its changes. */
function useReducedMotion(query: () => MediaQuery): boolean {
  const list = useMemo(() => query(), [query]);
  const subscribe = useCallback(
    (listener: () => void) => {
      list.addEventListener("change", listener);
      return () => {
        list.removeEventListener("change", listener);
      };
    },
    [list],
  );
  return useSyncExternalStore(subscribe, () => list.matches);
}

/** Properties of the square. */
export interface SquareProps {
  /** Loads the page module, once per mount. */
  readonly images: () => Promise<SquareImages>;
  /**
   * Where the player is: while it plays, the canvas follows the clock before every repaint;
   * in any other state it is drawn once, anew at every change (stopping, its clock still
   * running, then stopped, the first image).
   */
  readonly state: PlayerState["kind"];
  /** The frame of the audio clock now, read at every repaint; undefined between sessions. */
  readonly frame: () => AudioFrame | undefined;
  /** The reduced-motion preference (replaceable in tests). */
  readonly reducedMotion?: () => MediaQuery;
  /** Gets the drawing context of the canvas (replaceable in tests). */
  readonly context?: (canvas: HTMLCanvasElement) => SquareCanvas | null;
  /** Schedules the repaints (replaceable in tests). */
  readonly schedule?: (repaint: () => void) => () => void;
  /** The browser's GPU, if any (replaceable in tests). */
  readonly gpu?: () => Gpu<Handles> | undefined;
}

/**
 * The square and the line saying what it shows. While the sound plays, the canvas shows the
 * image of the clock's frame at every repaint, a new image only when the index changes;
 * otherwise it is drawn once: the image of the clock's frame while a session holds one
 * (suspended, frozen), the first image between sessions or under reduced motion.
 */
export function Square({ images, state, frame, reducedMotion = reducedMotionQuery, context = context2d, schedule = animationFrames, gpu = browserGpu }: SquareProps): JSX.Element {
  const [canvas, setCanvas] = useState<HTMLCanvasElement | null>(null);
  const [module, setModule] = useState<Module>({ kind: "loading" });
  // How the GPU stopped drawing, if it did: the square then draws on the CPU, on a fresh
  // canvas (a canvas that held a WebGPU context gives no 2D one), the GPU never asked again.
  const [gpuEnded, setGpuEnded] = useState<Ending | undefined>(undefined);
  const still = useReducedMotion(reducedMotion);
  useEffect(() => {
    // Loaded once the canvas exists (right after the first render), to draw on it.
    if (canvas === null) {
      return undefined;
    }
    let current = true;
    // The GPU's drawing, released when the page no longer shows it: nothing until it is set up.
    let release = (): void => undefined;
    // The setup's end, once: from its cleanup, or from a failure while drawing, so that a
    // failed square holds no GPU and hears no more of it (a later loss changes nothing).
    const stop = (): void => {
      if (current) {
        current = false;
        release();
      }
    };
    // Whether this canvas holds an image: one drawn since the setup began, whatever repaint drew it.
    let drew = false;
    /** The square ready to draw with `draw`, by what, and why not on the GPU. */
    const readyWith = (loaded: SquareImages, draw: Draw, by: DrawnBy, why: string | undefined): void => {
      const drawing: Draw = (index) => {
        draw(index);
        drew = true;
      };
      setModule({ kind: "ready", images: loaded, draw: drawing, drew: () => drew, by, why, stop });
    };
    const fail = (stage: SquareFailure, detail: string): void => {
      if (current) {
        console.error(`${LOGGED}${detail}`);
        setModule({ kind: "failed", stage, drawn: false, detail });
      }
    };
    const ended = (ending: Ending): void => {
      if (current) {
        report(ending.kind, ending.reason);
        setModule({ kind: "switching", by: ending.kind, why: ending.reason });
        setGpuEnded(ending);
      }
    };
    /** Draws on the CPU, `by` saying why: a canvas that refuses a 2D context (one of another kind, or memory short) draws nothing, a failure to say, never a blank canvas said drawn. */
    const onCpu = (loaded: SquareImages, by: DrawnBy, why: string | undefined): void => {
      const drawing = context(canvas);
      if (drawing === null) {
        fail("canvas", "the canvas refused a 2D context");
        return;
      }
      readyWith(loaded, cpuSquare(drawing, loaded), by, why);
    };
    const ready = async (loaded: SquareImages): Promise<void> => {
      // The GPU ended before: the CPU draws, the GPU never asked again.
      if (gpuEnded !== undefined) {
        if (current) {
          onCpu(loaded, gpuEnded.kind, gpuEnded.reason);
        }
        return;
      }
      // The GPU's drawing, or why there is none.
      const setup = await gpuSquare(gpu(), () => webgpuContext(canvas), loaded, SQUARE, ended, () => current);
      switch (setup.kind) {
        case "drawing":
          // A drawing set up for a square gone meanwhile is released at once; else when the setup ends.
          if (!current) {
            setup.release();
            return;
          }
          ({ release } = setup);
          readyWith(loaded, setup.draw, "gpu", undefined);
          return;
        case "refused":
          if (current) {
            report(setup.why.kind, setup.why.reason);
            onCpu(loaded, "cpu", setup.why.kind === "absent" ? undefined : setup.why.reason);
          }
          return;
      }
    };
    images()
      .then(ready)
      .catch((error: unknown) => {
        // A load of the page's own (`loadSquare`) names its stage; any other failure is a defect.
        fail(error instanceof SquareLoadError ? error.stage : "defect", failure(error));
      });
    return stop;
  }, [images, canvas, gpu, context, gpuEnded]);
  useEffect(() => {
    if (module.kind !== "ready") {
      return undefined;
    }
    let shown: bigint | undefined;
    const paint = (): void => {
      const at = still ? undefined : frame();
      // A clock that reads no time while an image is shown keeps that image: image 0 drawn
      // for a moment would flash the square back to its start.
      if (at === undefined && !still && shown !== undefined) {
        return;
      }
      try {
        const index = at === undefined ? 0n : module.images.imageAt(at.frame, at.rate, SQUARE.rate);
        if (index !== shown) {
          module.draw(index);
          shown = index;
        }
      } catch (error) {
        // A defect of the page's own, the same in every browser. Nothing is drawn after: the
        // GPU released, its loss no longer heard; the canvas keeps the image it showed, if any
        // (drawn by any repaint since its setup, not only this effect's).
        module.stop();
        const detail = failure(error);
        console.error(`${LOGGED}${detail}`);
        setModule({ kind: "failed", stage: "defect", drawn: module.drew(), detail });
      }
    };
    if (state !== "playing" || still) {
      paint();
      return undefined;
    }
    return schedule(paint);
  }, [state, still, module, frame, schedule]);
  const { name, line, drawnBy } = presentation(module, still, state);
  return (
    <section aria-label="Square">
      {/* A fresh canvas once the GPU ended: one that held a WebGPU context gives no 2D one. */}
      <canvas key={gpuEnded?.kind} ref={setCanvas} className="square" width={SQUARE.width} height={SQUARE.height} role="img" aria-label={name} aria-describedby={SAYS} />
      {module.kind === "failed" ? null : <p id={SAYS}>{line}</p>}
      {/* What draws it, and why not the GPU when it said why: for those who debug, on demand.
          Both stay in place while the square switches to the CPU, the line saying so, so that
          they are never removed and put back. */}
      {drawnBy === undefined ? null : <p data-testid="drawn-by">{drawnBy}</p>}
      {(module.kind === "ready" || module.kind === "switching") && module.why !== undefined ? (
        <details data-testid="why-not-gpu">
          <summary>{WHY_NOT_GPU}</summary>
          <code>{module.why}</code>
        </details>
      ) : null}
      {/* A live region present from the first render, so that a failure is announced, once:
          the line itself, which still describes the canvas, never a copy of it. */}
      <div aria-live="polite" data-testid="square-failure">
        {module.kind === "failed" ? <p id={SAYS}>{line}</p> : null}
      </div>
      {/* What exactly failed, for those who debug: shown on demand, never announced. */}
      {module.kind === "failed" ? (
        <details>
          <summary>{FAILURE_DETAILS}</summary>
          <code>{module.detail}</code>
        </details>
      ) : null}
    </section>
  );
}
