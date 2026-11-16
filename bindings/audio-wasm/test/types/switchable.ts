// Compiles: a switchable node, marked or not, sets its limiter and switches it live; a node
// of a stage of unknown mode is told apart by its mode, its refusals heard.
import { createEngineNode, loadPatch, PROTECTED_OUTPUT, type LoadedPatch, type Output, type SwitchableOutput } from "../../ts/web.ts";

declare const context: BaseAudioContext;
declare const loaded: LoadedPatch<SwitchableOutput>;
declare const unknown: LoadedPatch<Output>;

const heard = { onFailure: (): void => undefined, onRefused: (): void => undefined };
const node = createEngineNode(context, loaded, heard);
node.setLimiter(PROTECTED_OUTPUT.limiter);
node.setProtected(true);
createEngineNode(context, loaded, heard, true).engine.setProtected(false);
const any = createEngineNode(context, unknown, heard);
if (any.mode === "protected") {
  any.setLimiter(PROTECTED_OUTPUT.limiter);
}
export const typed: Promise<LoadedPatch<SwitchableOutput>> = loadPatch(new ArrayBuffer(0), new Uint8Array(), 48_000, undefined, { ...PROTECTED_OUTPUT, mode: "switchable" });
