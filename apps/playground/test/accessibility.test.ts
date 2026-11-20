// Every accessibility attribute a component sets is asserted by value in its own tests: axe
// proves the attributes valid, never that they say what they should, and the mutation tests
// do not change attribute strings in JSX, so that an attribute no test reads could be
// dropped or changed with every check green (deep review 2 found `aria-live` so).
import { readFileSync, readdirSync } from "node:fs";
import { expect, test } from "vitest";
import { findUp } from "./repository.ts";

/** The components of the page: each `src/*.tsx` but the entry point, and its test file. */
const COMPONENTS = readdirSync(findUp("apps/playground/src"))
  .filter((file) => file.endsWith(".tsx") && file !== "main.tsx")
  .map((file) => file.replace(/\.tsx$/u, ""));

/** The accessibility attributes `source` sets (`aria-*`), and the roles it gives (`role="…"`). */
function accessibility(source: string): { readonly attributes: readonly string[]; readonly roles: readonly string[] } {
  const attributes = new Set([...source.matchAll(/\b(aria-[a-z]+)=/gu)].map(([, name]) => name ?? ""));
  const roles = new Set([...source.matchAll(/\brole="([a-z]+)"/gu)].map(([, role]) => role ?? ""));
  return { attributes: [...attributes].sort(), roles: [...roles].sort() };
}

/** What `source` sets that `tests` never reads by value: each attribute read through `getAttribute`, `toHaveAttribute` or `hasAttribute`, or matched by value in a selector (`[aria-hidden='true']`), a name also by a query finding the element by it, each role found by `…ByRole`. */
function unasserted(source: string, tests: string): readonly string[] {
  const { attributes, roles } = accessibility(source);
  return [
    // A name is also asserted by value when a query finds the element by it.
    ...attributes.filter((name) => !new RegExp(`(getAttribute|toHaveAttribute|hasAttribute)\\("${name}"|\\[${name}=${name === "aria-label" ? '|ByRole\\("[a-z]+", \\{ name: ' : ""}`, "u").test(tests)),
    ...roles.filter((role) => !new RegExp(`ByRole\\("${role}"`, "u").test(tests)).map((role) => `role="${role}"`),
  ];
}

test("asserts every accessibility attribute and role of each component in its own tests", () => {
  expect(COMPONENTS.length, "the components found").toBeGreaterThanOrEqual(3);
  for (const component of COMPONENTS) {
    const source = readFileSync(findUp(`apps/playground/src/${component}.tsx`), "utf8");
    const tests = readFileSync(findUp(`apps/playground/test/${component}.test.tsx`), "utf8");
    expect(unasserted(source, tests), component).toStrictEqual([]);
  }
});

test("finds an attribute no test reads: the live region before deep review 2", () => {
  // What revealed the rule: Square set `aria-live`, and no test of it read the attribute.
  const source = '<div aria-live="polite" data-testid="square-failure" /><canvas role="img" aria-label={name} />';
  expect(accessibility(source)).toStrictEqual({ attributes: ["aria-label", "aria-live"], roles: ["img"] });
  expect(unasserted(source, 'screen.getByRole("img").getAttribute("aria-label")')).toStrictEqual(["aria-live"]);
  expect(unasserted(source, 'getByRole("img"); x.getAttribute("aria-label"); expect(y).toHaveAttribute("aria-live", "polite")')).toStrictEqual([]);
  expect(unasserted(source, "nothing")).toStrictEqual(["aria-label", "aria-live", 'role="img"']);
  expect(unasserted(source, "getByRole(\"img\"); querySelector(\"[aria-label='x'][aria-live='polite']\")"), "matched by value in a selector").toStrictEqual([]);
  expect(unasserted(source, 'getByRole("img", { name: "x" }); toHaveAttribute("aria-live", "polite")'), "a name found by a query").toStrictEqual([]);
});
