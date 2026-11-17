// Does not compile: a loaded patch's output stage is a copy the engine reads, of the mode
// given and none of the caller's other fields.
import { LoadedPatch, loadPatch, PROTECTED_OUTPUT } from "../../ts/web.ts";

const labelled = { ...PROTECTED_OUTPUT, label: "master" };
export const label: Promise<string> = loadPatch(new ArrayBuffer(0), new Uint8Array(), 48_000, undefined, labelled).then((loaded) => loaded.output.label);
export const precise: Promise<string> = LoadedPatch.load(new ArrayBuffer(0), new Uint8Array(), 48_000, "binary32", labelled).then((loaded) => loaded.output.label);
