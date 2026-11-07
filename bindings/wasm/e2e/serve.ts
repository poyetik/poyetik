// Static server for the end-to-end tests: the package at /, the conformance corpus at
// /conformance/. Local only.
import { readFile } from "node:fs/promises";
import { createServer } from "node:http";
import { extname } from "node:path";

const PORT = 4392;
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
