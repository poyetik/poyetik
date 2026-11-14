// Does not compile: an object shaped like a marked node is not one.
import { captureEngine, type EngineNode } from "../../ts/web.ts";

declare const context: BaseAudioContext;
declare const engine: EngineNode;

export const captured = captureEngine(context, { ...engine, channels: 2 }, 1);
