// Telling downloaded WebAssembly bytes apart, the one rule of the square and of the sound:
// bytes that do not start as a module are no module the server meant to send, so their
// remedy is the download's; bytes that start as one and do not compile are a module this
// browser cannot compile, so their remedy is the browser's.

/** The magic every WebAssembly module starts with, `\0asm`, as text of its four bytes. */
const MAGIC = "\0asm";

/**
 * Whether `bytes` start with the magic of a WebAssembly module: its magic alone, whatever
 * version follows, so that a module of another version is one this browser cannot compile,
 * never a download that failed.
 */
export function isWasmModule(bytes: ArrayBuffer): boolean {
  return String.fromCharCode(...new Uint8Array(bytes.slice(0, MAGIC.length))) === MAGIC;
}

/** Whether `error` is an error a module's compiling raised: one whose cause is the browser's `CompileError`. */
export function cannotCompile(error: unknown): boolean {
  return error instanceof Error && error.cause instanceof WebAssembly.CompileError;
}
