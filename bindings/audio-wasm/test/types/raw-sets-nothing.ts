// Does not compile: a raw stage has no limiter, so its node sets nothing live.
import { createEngineNode, type LoadedPatch, type RawOutput } from "../../ts/web.ts";

declare const context: BaseAudioContext;
declare const loaded: LoadedPatch<RawOutput>;

createEngineNode(context, loaded, { onFailure: () => undefined }).setLimiter({ ceiling: -1, lookahead: 1.5, release: 100, adaptivity: 0.5 });
