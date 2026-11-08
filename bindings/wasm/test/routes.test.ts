import { resolve } from "node:path";
import { describe, expect, test } from "vitest";
import { route } from "../e2e/routes.ts";

const ROOTS = { package: "/repo/bindings/wasm", conformance: "/repo/conformance" };
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
