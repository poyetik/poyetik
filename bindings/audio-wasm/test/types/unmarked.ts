// Does not compile: a node created without the marker channel cannot be captured from its start.
import { captureEngine, createEngineNode, type LoadedPatch } from "../../ts/web.ts";

declare const context: BaseAudioContext;
declare const loaded: LoadedPatch;

export const captured = captureEngine(context, createEngineNode(context, loaded, () => undefined), 1);
