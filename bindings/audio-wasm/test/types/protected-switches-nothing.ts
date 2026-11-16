// Does not compile: a protected stage is never switched to raw live; only a switchable one is.
import { createEngineNode, type LoadedPatch, type ProtectedOutput } from "../../ts/web.ts";

declare const context: BaseAudioContext;
declare const loaded: LoadedPatch<ProtectedOutput>;

createEngineNode(context, loaded, { onFailure: () => undefined, onRefused: () => undefined }).setProtected(false);
