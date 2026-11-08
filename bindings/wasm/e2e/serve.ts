// Static server of the test bench (`pnpm bench`, started by the end-to-end tests): only
// what `route` allows, for this machine only, on the port `ENGINE_WASM_E2E_PORT` gives.
import { readFile } from "node:fs/promises";
import { createServer } from "node:http";
import { fileURLToPath } from "node:url";
import { contentType, parsePort, route } from "./routes.ts";

/** The port of this run: chosen by the Playwright configuration, or by whoever runs the bench. */
const PORT = parsePort(process.env["ENGINE_WASM_E2E_PORT"] ?? "");
/** The only two directories served: this package (for `dist/` and the bench page) and the conformance corpus. */
const ROOTS = {
  package: fileURLToPath(new URL("../", import.meta.url)),
  conformance: fileURLToPath(new URL("../../../conformance/", import.meta.url)),
};

if (PORT === undefined) {
  process.stderr.write("error: ENGINE_WASM_E2E_PORT must be a port from 1 to 65535 (see: bindings/wasm/README.md)\n");
  process.exitCode = 2;
} else {
  const port = PORT;
  createServer((request, response) => {
    const target = route(request.url ?? "/", request.headers.host, port, ROOTS);
    if ("status" in target) {
      response.writeHead(target.status);
      response.end();
      return;
    }
    readFile(target.file).then(
      (body) => {
        response.writeHead(200, { "content-type": contentType(target.file) });
        response.end(body);
      },
      () => {
        response.writeHead(404);
        response.end();
      },
    );
  }).listen(port, "localhost");
}
