// Static server for the end-to-end tests: only what `route` allows, for this machine only.
import { readFile } from "node:fs/promises";
import { createServer } from "node:http";
import { extname } from "node:path";
import { fileURLToPath } from "node:url";
import { route } from "./routes.ts";

/** The port chosen for this run by the Playwright configuration. */
const PORT = Number(process.env["ENGINE_WASM_E2E_PORT"]);
if (!Number.isInteger(PORT) || PORT <= 0) {
  throw new Error("ENGINE_WASM_E2E_PORT is not set: run the tests with pnpm test:e2e");
}
/** The only two directories served: this package (for `dist/` and the bench page) and the conformance corpus. */
const ROOTS = {
  package: fileURLToPath(new URL("../", import.meta.url)),
  conformance: fileURLToPath(new URL("../../../conformance/", import.meta.url)),
};
/** The `content-type` of each served extension: anything else is served as `application/octet-stream`. */
const TYPES: Readonly<Record<string, string>> = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".json": "application/json",
  ".wasm": "application/wasm",
};

createServer((request, response) => {
  const target = route(request.url ?? "/", request.headers.host, PORT, ROOTS);
  if ("status" in target) {
    response.writeHead(target.status);
    response.end();
    return;
  }
  readFile(target.file).then(
    (body) => {
      response.writeHead(200, { "content-type": TYPES[extname(target.file)] ?? "application/octet-stream" });
      response.end(body);
    },
    () => {
      response.writeHead(404);
      response.end();
    },
  );
}).listen(PORT, "localhost");
