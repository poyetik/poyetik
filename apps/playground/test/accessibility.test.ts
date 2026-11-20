// Every accessibility attribute a component sets is asserted by value in its own tests: axe
// proves the attributes valid, never that they say what they should, and the mutation tests
// do not change attribute strings in JSX, so that an attribute no test reads could be
// dropped or changed with every check green (deep review 2 found `aria-live` so). Each
// occurrence needs a read of its own, a read is one of a value (never of presence alone),
// and a spelling the gate cannot read (a role not written `role="…"`, a spread on an
// intrinsic element, which may set any attribute) is refused, never read as absent.
import { readFileSync, readdirSync } from "node:fs";
import { expect, test } from "vitest";
import { findUp } from "./repository.ts";

/** The components of the page: each `src/*.tsx` but the entry point, and its test file. */
const COMPONENTS = readdirSync(findUp("apps/playground/src"))
  .filter((file) => file.endsWith(".tsx") && file !== "main.tsx")
  .map((file) => file.replace(/\.tsx$/u, ""));

/** How many times `pattern` (global) matches `text`. */
function count(text: string, pattern: RegExp): number {
  return [...text.matchAll(pattern)].length;
}

/** `text` with the characters of regular expressions escaped, to match it as it is. */
function literal(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&");
}

/**
 * The accessibility attributes `source` sets (`aria-*`) and the roles it gives (`role="…"`),
 * each with the number of its occurrences; and what it writes in a way the gate cannot read.
 */
function accessibility(source: string): { readonly attributes: ReadonlyMap<string, number>; readonly roles: ReadonlyMap<string, number>; readonly unreadable: readonly string[] } {
  const tally = (pattern: RegExp): Map<string, number> => {
    const counts = new Map<string, number>();
    for (const [, name] of source.matchAll(pattern)) {
      if (name !== undefined) {
        counts.set(name, (counts.get(name) ?? 0) + 1);
      }
    }
    return new Map([...counts].sort(([left], [right]) => (left < right ? -1 : 1)));
  };
  const unreadable = [
    ...[...source.matchAll(/\brole=(?!")\S*/gu)].map(([spelled]) => `${spelled}: write a role as role="…"`),
    ...[...source.matchAll(/<[a-z][a-z0-9]*\b[^>]*?\{\s*\.\.\.[^}]*\}/gu)].map(([spelled]) => `${spelled}: no spread on an intrinsic element, write each attribute`),
  ];
  return { attributes: tally(/\b(aria-[a-z]+)=/gu), roles: tally(/\brole="([a-z]+)"/gu), unreadable };
}

/**
 * The reads by value of attribute `name` in `tests`: `toHaveAttribute` with a value,
 * `getAttribute` in a statement ending in an exact matcher (`toBe`, `toStrictEqual`), a
 * selector matching a value (`[aria-hidden='true']`), and for a name or a description, a
 * query finding the element by it.
 */
function valueReads(name: string, tests: string): number {
  const by = name === "aria-label" ? "name" : name === "aria-describedby" ? "description" : undefined;
  return (
    count(tests, new RegExp(`toHaveAttribute\\("${literal(name)}", `, "gu")) +
    count(tests, new RegExp(`getAttribute\\("${literal(name)}"\\)[^;]*\\)\\.(toBe|toStrictEqual)\\(`, "gu")) +
    count(tests, new RegExp(`\\[${literal(name)}=['"][^'"]*['"]\\]`, "gu")) +
    (by === undefined ? 0 : count(tests, new RegExp(`ByRole\\("[a-z]+", \\{ ${by}: `, "gu")))
  );
}

/** What `source` sets that `tests` does not read by value once per occurrence, then what it writes unreadably. */
function unasserted(source: string, tests: string): readonly string[] {
  const { attributes, roles, unreadable } = accessibility(source);
  const short = (what: string, set: number, read: number): readonly string[] => (read < set ? [`${what}: set ${String(set)} times, read by value ${String(read)}`] : []);
  return [
    ...[...attributes].flatMap(([name, set]) => short(name, set, valueReads(name, tests))),
    ...[...roles].flatMap(([role, set]) => short(`role="${role}"`, set, count(tests, new RegExp(`ByRole\\("${literal(role)}"`, "gu")))),
    ...unreadable,
  ];
}

test("asserts every accessibility attribute and role of each component in its own tests, by value, once per occurrence", () => {
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
  expect(accessibility(source)).toStrictEqual({ attributes: new Map([["aria-label", 1], ["aria-live", 1]]), roles: new Map([["img", 1]]), unreadable: [] });
  expect(unasserted(source, 'expect(screen.getByRole("img").getAttribute("aria-label")).toBe("x");')).toStrictEqual(["aria-live: set 1 times, read by value 0"]);
  expect(unasserted(source, 'getByRole("img"); expect(x.getAttribute("aria-label")).toBe("x"); expect(y).toHaveAttribute("aria-live", "polite")')).toStrictEqual([]);
  expect(unasserted(source, "nothing")).toStrictEqual(["aria-label: set 1 times, read by value 0", "aria-live: set 1 times, read by value 0", 'role="img": set 1 times, read by value 0']);
  expect(unasserted(source, "getByRole(\"img\"); querySelector(\"[aria-label='x'][aria-live='polite']\")"), "matched by value in a selector").toStrictEqual([]);
  expect(unasserted(source, 'getByRole("img", { name: "x" }); toHaveAttribute("aria-live", "polite")'), "a name found by a query").toStrictEqual([]);
});

test("refuses a read of presence alone, a read with no exact matcher, and fewer reads than occurrences (deep review 3)", () => {
  const source = '<button aria-describedby={id} /><p aria-live="polite" /><p aria-live="off" />';
  const live = 'expect(a.getAttribute("aria-live")).toBe("polite"); expect(b.getAttribute("aria-live")).toBe("off");';
  expect(unasserted(source, `expect(button.hasAttribute("aria-describedby")).toBe(true); ${live}`), "presence alone").toStrictEqual(["aria-describedby: set 1 times, read by value 0"]);
  expect(unasserted(source, `expect(button).toHaveAttribute("aria-describedby"); ${live}`), "toHaveAttribute without a value").toStrictEqual(["aria-describedby: set 1 times, read by value 0"]);
  expect(unasserted(source, `const id = button.getAttribute("aria-describedby"); ${live}`), "a read no matcher asserts").toStrictEqual(["aria-describedby: set 1 times, read by value 0"]);
  expect(unasserted(source, `expect(button.getAttribute("aria-describedby")).not.toBeNull(); ${live}`), "a matcher of presence").toStrictEqual(["aria-describedby: set 1 times, read by value 0"]);
  expect(unasserted(source, 'expect(button).toHaveAttribute("aria-describedby", "alert"); expect(a.getAttribute("aria-live")).toBe("polite");'), "two regions, one read").toStrictEqual(["aria-live: set 2 times, read by value 1"]);
  expect(unasserted(source, `expect(button).toHaveAttribute("aria-describedby", "alert"); ${live}`), "each occurrence read").toStrictEqual([]);
});

test("refuses a spelling it cannot read, never reading it as absent (deep review 3)", () => {
  const read = 'getByRole("img"); getByRole("img");';
  expect(unasserted('<canvas role={kind} />', read)).toStrictEqual(["role={kind}: write a role as role=\"…\""]);
  expect(unasserted("<canvas role='img' />", read)).toStrictEqual(["role='img': write a role as role=\"…\""]);
  expect(unasserted("<canvas {...props} />", read)).toStrictEqual(["<canvas {...props}: no spread on an intrinsic element, write each attribute"]);
  expect(unasserted('<canvas role="img" { ...rest } />', read)).toStrictEqual(['<canvas role="img" { ...rest }: no spread on an intrinsic element, write each attribute']);
  expect(unasserted('<Square {...props} /><canvas role="img" />', read), "a component's own props, its attributes in its own source").toStrictEqual([]);
});
