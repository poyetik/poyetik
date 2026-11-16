// Does not compile: a protected node's refusals must be heard (onRefused is required).
import { createEngineNode, type LoadedPatch, type ProtectedOutput } from "../../ts/web.ts";

declare const context: BaseAudioContext;
declare const loaded: LoadedPatch<ProtectedOutput>;

export const node = createEngineNode(context, loaded, { onFailure: () => undefined });
