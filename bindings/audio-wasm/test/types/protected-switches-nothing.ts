// Does not compile: a protected node offers no setProtected, since the engine never switches a
// protected stage to raw (it refuses, through every interface: test/processors.test.ts); only a
// switchable one is.
import { createEngineNode, type LoadedPatch, type ProtectedOutput } from "../../ts/web.ts";

declare const context: BaseAudioContext;
declare const loaded: LoadedPatch<ProtectedOutput>;

createEngineNode(context, loaded, { onFailure: () => undefined, onRefused: () => undefined }).setProtected(false);
