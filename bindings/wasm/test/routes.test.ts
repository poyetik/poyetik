import { resolve } from "node:path";
import { describe, expect, test } from "vitest";
import { contentType, parsePort, route } from "../e2e/routes.ts";

/** Fake directories served: no file is read, `route` only resolves paths. */
const ROOTS = { package: "/repo/bindings/wasm", conformance: "/repo/conformance" };
/** The `Host` header of a request to this machine on the served port (4000). */
const HOST = "localhost:4000";

describe("route", () => {
  test.each([
    ["/dist/engine.wasm", "/repo/bindings/wasm/dist/engine.wasm"],
    ["/e2e/site/index.html?case=sine-440", "/repo/bindings/wasm/e2e/site/index.html"],
    ["/conformance/cases/sine-440.json", "/repo/conformance/cases/sine-440.json"],
    ["/conformance/patches/sine%2D440.json", "/repo/conformance/patches/sine-440.json"],
    ["/conformance/references.json", "/repo/conformance/references.json"],
  ])("serves %s", (target, file) => {
    expect(route(target, HOST, 4000, ROOTS)).toStrictEqual({ file: resolve(file) });
  });

  test.each([
    "/",
    "/package.json",
    "/e2e/serve.ts",
    "/conformance/expectations.json",
    "/conformance//etc/hosts",
    "/conformance/cases/../../etc/hosts",
    "/conformance/cases/%2e%2e/%2e%2e/etc/hosts",
    "/conformance/cases/..%2f..%2fetc%2fhosts",
    "/dist/",
    "/dist/%2e%2e",
    "/dist/%E0%A4%A",
  ])("refuses %s", (target) => {
    expect(route(target, HOST, 4000, ROOTS)).toStrictEqual({ status: 404 });
  });

  test.each([undefined, "evil.example:4000", "localhost:4001", "localhost", "127.0.0.2:4000"])("refuses the host %s", (host) => {
    expect(route("/dist/engine.wasm", host, 4000, ROOTS)).toStrictEqual({ status: 403 });
  });

  test("accepts this machine by name or address", () => {
    expect(route("/dist/a.js", "127.0.0.1:4000", 4000, ROOTS)).toStrictEqual({ file: resolve("/repo/bindings/wasm/dist/a.js") });
  });
});

describe("contentType", () => {
  test.each([
    ["/a/index.html", "text/html; charset=utf-8"],
    ["/a/harness.js", "text/javascript; charset=utf-8"],
    ["/a/sine.json", "application/json"],
    ["/a/engine.wasm", "application/wasm"],
    ["/a/favicon.svg", "image/svg+xml"],
    ["/a/notes.txt", "application/octet-stream"],
    ["/a/README", "application/octet-stream"],
  ])("serves %s as %s", (file, type) => {
    expect(contentType(file)).toBe(type);
  });
});

describe("parsePort", () => {
  test.each([
    ["1", 1],
    ["4392", 4392],
    ["65535", 65_535],
  ])("reads the port %s", (value, port) => {
    expect(parsePort(value)).toBe(port);
  });

  test.each(["", "0", "-1", "1.5", "65536", "080", " 80", "80 ", "1e3", "0x50", "port"])("refuses %o", (value) => {
    expect(parsePort(value)).toBeUndefined();
  });
});
