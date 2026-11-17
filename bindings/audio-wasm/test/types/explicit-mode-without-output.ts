// Does not compile: a mode named without the output stage of that mode, which would load raw
// typed as limited.
import { LoadedPatch, loadPatch, type ProtectedOutput, type SwitchableOutput } from "../../ts/web.ts";

declare const wasm: ArrayBuffer;
declare const patch: Uint8Array;

void loadPatch<ProtectedOutput>(wasm, patch, 48_000);
void LoadedPatch.load<SwitchableOutput>(wasm, patch, 48_000, "binary64");
