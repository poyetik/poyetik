// Does not compile: the render quantum a patch reports is a number, not the literal 128 of
// Web Audio 1.0's default, which a context of another quantum would not hold.
import type { LoadedPatch } from "../../ts/web.ts";

declare const loaded: LoadedPatch;

export const quantum: 128 = loaded.settings.renderQuantum;
