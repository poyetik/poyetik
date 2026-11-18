/**
 * The square shown with the sound: a canvas showing, before every repaint, the image of the
 * visual library's square that the audio clock is at, outside React's rendering, plus a
 * line saying what it shows.
 *
 * The image follows the audio clock, never the page's own: the page's module answers which
 * image a frame of the clock shows (exactly, `page_image_at`), then renders it encoded for
 * display, and the canvas takes its bytes as they are: drawn on the GPU through WebGPU when
 * the browser has one (the library's shaders, `gpu-square.ts`), on the processor otherwise,
 * the same bytes either way, and the page says which. Between sessions it shows the first
 * image; when the system asks for reduced motion, it shows the first image only, and says
 * so.
 */

import { useCallback, useEffect, useMemo, useState, useSyncExternalStore, type JSX } from "react";
import type { AudioFrame, PlayerState } from "./player.ts";
import { type CanvasGpu, type Draw, type Gpu, type GpuSource, type Handles, type WebGpu, gpuSquare } from "./gpu-square.ts";
import { animationFrames } from "./Oscilloscope.tsx";
import { SQUARE } from "./settings.ts";

/** What the square needs of the page's module: which image a frame shows, and that image for display. */
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

/** What the line says while the square moves with the sound, or waits for it. */
export const MOVING = "The square sweeps across the image in step with the sound, one sweep every two seconds, on the audio clock. Still until the sound starts.";

/** What the line says when the system asks for reduced motion. */
export const STILL = "The square stays still: your system asks for reduced motion.";

/** What the line says while the page's module loads. */
export const LOADING = "Loading the square…";

/** What the line says when the square cannot be shown, before the reason. */
export const FAILED = "The square cannot be shown:";

/** What draws the square: the GPU, or the processor. */
export type DrawnBy = "gpu" | "processor";

/** What the line under the square says draws it. */
export const DRAWN_BY: Readonly<Record<DrawnBy, string>> = {
  gpu: "Drawn by the GPU, through WebGPU.",
  processor: "Drawn by the processor, in WebAssembly.",
};

/** Where the square is: its module loading, ready to draw, or failed (to load, to answer, or its GPU lost). */
type Module =
  | { readonly kind: "loading" }
  | { readonly kind: "ready"; readonly images: SquareImages; readonly draw: Draw; readonly by: DrawnBy }
  | { readonly kind: "failed"; readonly detail: string };

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

/** Draws on the processor: the module's bytes put on `drawing` as they are (nothing without a 2D context). */
function processorSquare(drawing: SquareCanvas | null, images: SquareImages): Draw {
  return (index) => {
    // The module's bytes as they are, a view of its memory: copied once, into the canvas.
    drawing?.putImageData(new ImageData(images.display(index), SQUARE.width, SQUARE.height), 0, 0);
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
  /** Loads the page's module, once per mount. */
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
  readonly schedule?: (frame: () => void) => () => void;
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
  const still = useReducedMotion(reducedMotion);
  useEffect(() => {
    // Loaded once the canvas exists (right after the first render), to draw on it.
    if (canvas === null) {
      return undefined;
    }
    let current = true;
    const fail = (detail: string): void => {
      if (current) {
        setModule({ kind: "failed", detail });
      }
    };
    const ready = async (loaded: SquareImages): Promise<void> => {
      const onGpu = await gpuSquare(gpu(), () => webgpuContext(canvas), loaded, SQUARE, (reason) => {
        fail(`the GPU was lost: ${reason}`);
      });
      if (current) {
        setModule(onGpu === undefined ? { kind: "ready", images: loaded, draw: processorSquare(context(canvas), loaded), by: "processor" } : { kind: "ready", images: loaded, draw: onGpu, by: "gpu" });
      }
    };
    images()
      .then(ready)
      .catch((error: unknown) => {
        fail(String(error));
      });
    return () => {
      current = false;
    };
  }, [images, canvas, gpu, context]);
  useEffect(() => {
    if (module.kind !== "ready") {
      return undefined;
    }
    let shown: bigint | undefined;
    const paint = (): void => {
      const at = still ? undefined : frame();
      try {
        const index = at === undefined ? 0n : module.images.imageAt(at.frame, at.rate, SQUARE.rate);
        if (index !== shown) {
          module.draw(index);
          shown = index;
        }
      } catch (error) {
        setModule({ kind: "failed", detail: String(error) });
      }
    };
    if (state !== "playing" || still) {
      paint();
      return undefined;
    }
    return schedule(paint);
  }, [state, still, module, frame, schedule]);
  const says = module.kind === "loading" ? LOADING : module.kind === "failed" ? `${FAILED} ${module.detail}` : still ? STILL : MOVING;
  return (
    <section aria-label="Square">
      <canvas ref={setCanvas} className="square" width={SQUARE.width} height={SQUARE.height} role="img" aria-label="A white square on black, sweeping from left to right" aria-describedby={SAYS} />
      <p id={SAYS}>{says}</p>
      {module.kind === "ready" ? <p data-testid="drawn-by">{DRAWN_BY[module.by]}</p> : null}
    </section>
  );
}
