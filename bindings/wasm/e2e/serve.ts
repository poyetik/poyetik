// Static server for the end-to-end tests: the package at /, the conformance corpus at
// /conformance/. Local only.
import { readFile } from "node:fs/promises";
import { createServer } from "node:http";
import { extname } from "node:path";

/** The port chosen for this run by the Playwright configuration. */
const PORT = Number(process.env["ENGINE_WASM_E2E_PORT"]);
if (!Number.isInteger(PORT) || PORT <= 0) {
  throw new Error("ENGINE_WASM_E2E_PORT is not set: run the tests with pnpm test:e2e");
}
const PACKAGE = new URL("../", import.meta.url);
const CONFORMANCE = new URL("../../../conformance/", import.meta.url);
const TYPES: Readonly<Record<string, string>> = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".json": "application/json",
  ".wasm": "application/wasm",
};

createServer((request, response) => {
  const path = new URL(request.url ?? "/", "http://localhost").pathname;
  const file = path.startsWith("/conformance/")
    ? new URL(path.slice("/conformance/".length), CONFORMANCE)
    : new URL(path.slice(1), PACKAGE);
  readFile(file).then(
    (body) => {
      response.writeHead(200, { "content-type": TYPES[extname(file.pathname)] ?? "application/octet-stream" });
      response.end(body);
    },
    () => {
      response.writeHead(404);
      response.end();
    },
  );
}).listen(PORT, "localhost");
