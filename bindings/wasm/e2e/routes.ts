// What the end-to-end server may serve: a pure function, tested on its own, so that the
// server never reads a file outside the few it is meant to.
import { resolve, sep } from "node:path";

/** Where a request leads: a file to read, or a refusal. */
export type Route = { readonly file: string } | { readonly status: 403 | 404 };

/** The directories the server reads from. */
export interface Roots {
  /** The engine-wasm package. */
  readonly package: string;
  /** The conformance corpus. */
  readonly conformance: string;
}

/** The URL prefixes served, each with the directory it maps to (relative to a root). */
const SERVED: readonly { readonly prefix: string; readonly root: keyof Roots; readonly dir: string }[] = [
  { prefix: "/dist/", root: "package", dir: "dist" },
  { prefix: "/e2e/site/", root: "package", dir: "e2e/site" },
  { prefix: "/conformance/cases/", root: "conformance", dir: "cases" },
  { prefix: "/conformance/patches/", root: "conformance", dir: "patches" },
];

/** The one file served outside those directories. */
const REFERENCES = "/conformance/references.json";

/**
 * Routes a request. Refuses (403) any host but this machine on `port`, which a page of
 * another site could only reach through DNS rebinding; serves only the directories of
 * {@link SERVED} and the references, after decoding and normalizing the path, so that
 * no `..` or encoded separator leaves them (404).
 */
export function route(target: string, host: string | undefined, port: number, roots: Roots): Route {
  if (host !== `localhost:${String(port)}` && host !== `127.0.0.1:${String(port)}`) {
    return { status: 403 };
  }
  let path: string;
  try {
    path = decodeURIComponent(new URL(target, "http://localhost").pathname);
  } catch {
    return { status: 404 };
  }
  if (path === REFERENCES) {
    return { file: resolve(roots.conformance, "references.json") };
  }
  for (const served of SERVED) {
    if (path.startsWith(served.prefix)) {
      const dir = resolve(roots[served.root], served.dir);
      const file = resolve(dir, path.slice(served.prefix.length));
      return file.startsWith(dir + sep) ? { file } : { status: 404 };
    }
  }
  return { status: 404 };
}
