// Compiles: a node created with the marker channel can be captured from its start.
import { captureEngine, createEngineNode, type LoadedPatch } from "../../ts/web.ts";

declare const context: BaseAudioContext;
declare const loaded: LoadedPatch;

export const captured = captureEngine(context, createEngineNode(context, loaded, { onFailure: () => undefined }, true), 1);
