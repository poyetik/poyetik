// The accessibility gate (`gates/accessibility.ts`) over the page's components, and its fixtures.
import { readFileSync, readdirSync } from "node:fs";
import { expect, test } from "vitest";
import { accessibility, unasserted } from "../gates/accessibility.ts";
import { findUp } from "./repository.ts";

/** The components of the page: each `src/*.tsx` but the entry point, and its test file. */
const COMPONENTS = readdirSync(findUp("apps/playground/src"))
  .filter((file) => file.endsWith(".tsx") && file !== "main.tsx")
  .map((file) => file.replace(/\.tsx$/u, ""));

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
  expect(accessibility(source)).toStrictEqual({
    attributes: new Map([
      ["aria-label", [{ computed: "{name}" }]],
      ["aria-live", [{ literal: "polite" }]],
      ["role", [{ literal: "img" }]],
    ]),
    unreadable: [],
  });
  expect(unasserted(source, 'getByRole("img"); expect(screen.getByRole("img").getAttribute("aria-label")).toBe("x");')).toStrictEqual(['aria-live="polite": set 1 times, read by value 0']);
  expect(unasserted(source, 'getByRole("img"); expect(x.getAttribute("aria-label")).toBe("x"); expect(y).toHaveAttribute("aria-live", "polite")')).toStrictEqual([]);
  expect(unasserted(source, "nothing")).toStrictEqual(["aria-label={name}: read by value 0 times", 'aria-live="polite": set 1 times, read by value 0', 'role="img": set 1 times, read by value 0']);
  expect(unasserted(source, "getByRole(\"img\"); querySelector(\"[aria-label='x'][aria-live='polite']\")"), "matched by value in a selector").toStrictEqual([]);
  expect(unasserted(source, 'getByRole("img", { name: "x" }); toHaveAttribute("aria-live", "polite")'), "toHaveAttribute outside expect is no read").toStrictEqual(['aria-live="polite": set 1 times, read by value 0']);
  expect(unasserted(source, 'getByRole("img", { name: NAMES.loading }); expect(region).toHaveAttribute("aria-live", "polite")'), "a name found by a query").toStrictEqual([]);
  expect(unasserted(source, 'getByRole("img"); expect(region.getAttribute("aria-live")).toBe("off"); expect(x.getAttribute("aria-label")).toBe("x")'), "a read of another value").toStrictEqual(['aria-live="polite": set 1 times, read by value 0']);
});

test("refuses a read of presence alone, of anything, or with no exact matcher (deep reviews 3 and 4)", () => {
  const source = '<p aria-live="polite" />';
  for (const [what, tests] of [
    ["presence", 'expect(p.hasAttribute("aria-live")).toBe(true);'],
    ["toHaveAttribute without a value", 'expect(p).toHaveAttribute("aria-live");'],
    ["a read no matcher asserts", 'const live = p.getAttribute("aria-live");'],
    ["a matcher of presence", 'expect(p.getAttribute("aria-live")).not.toBeNull();'],
    ["a negated exact matcher", 'expect(p.getAttribute("aria-live")).not.toBe("off");'],
    ["a comparison asserted true", 'expect(p.getAttribute("aria-live") !== null).toBe(true);'],
    ["its type asserted", 'expect(typeof p.getAttribute("aria-live")).toBe("string");'],
    ["toHaveAttribute with anything", 'expect(p).toHaveAttribute("aria-live", expect.anything());'],
    ["toHaveAttribute with any string", 'expect(p).toHaveAttribute("aria-live", expect.any(String));'],
    ["toBe with anything", 'expect(p.getAttribute("aria-live")).toBe(expect.anything());'],
    ["absence asserted", 'expect(p.getAttribute("aria-live")).toBe(null);'],
    ["undefined asserted", 'expect(p.getAttribute("aria-live")).toBe(undefined);'],
    ["a call's answer", 'expect(p.getAttribute("aria-live")).toBe(live());'],
    ["a selector of presence", 'querySelector("[aria-live]");'],
    ["the value in a comment", '// expect(p.getAttribute("aria-live")).toBe("polite");'],
  ] as const) {
    expect(unasserted(source, tests), what).toStrictEqual(['aria-live="polite": set 1 times, read by value 0']);
  }
  for (const [what, tests] of [
    ["toBe", 'expect(p.getAttribute("aria-live")).toBe("polite");'],
    ["toStrictEqual, with a message", 'expect(p.getAttribute("aria-live"), "polite").toStrictEqual("polite");'],
    ["an element of an array", 'expect([says(), p.getAttribute("aria-live")]).toStrictEqual([LINE, "polite"]);'],
    ["an element of either array", 'expect([p.getAttribute("aria-live")]).toStrictEqual(late ? ["polite"] : ["polite"]);'],
    ["toHaveAttribute", 'expect(p).toHaveAttribute("aria-live", "polite");'],
  ] as const) {
    expect(unasserted(source, tests), what).toStrictEqual([]);
  }
});

test("ties each read to its occurrence: each literal value read as often as it is set, one computed value an attribute (deep review 4)", () => {
  // What revealed it: Square labels its section "Square" and its canvas by its state, and the
  // canvas's many reads stood for any label added beside them.
  const square = '<section aria-label="Square"><canvas role="img" aria-label={name} /></section>';
  const canvasReads = 'getByRole("img"); expect(canvas.getAttribute("aria-label")).toBe(NAMES.loading); expect(canvas.getAttribute("aria-label")).toBe(NAMES.still);';
  expect(unasserted(square, canvasReads), "the section's label unread").toStrictEqual(['aria-label="Square": set 1 times, read by value 0']);
  expect(unasserted(square, `${canvasReads} getByRole("region", { name: "Square" });`), "each read").toStrictEqual([]);
  expect(unasserted(square, 'getByRole("img"); getByRole("region", { name: "Square" }); getByRole("region", { name: "Square" });'), "the canvas's label unread, the section's read twice").toStrictEqual(["aria-label={name}: read by value 0 times"]);
  expect(unasserted('<section aria-label="Square"><canvas aria-label={name} /><p aria-label={more} /></section>', `${canvasReads} getByRole("region", { name: "Square" });`), "a second computed label").toStrictEqual(["aria-label: 2 computed values (aria-label={name}, aria-label={more}), a read ties to one only: give the others a literal value"]);
  const regions = '<p aria-live="polite" /><p aria-live="polite" /><p aria-live="off" />';
  const off = 'expect(c.getAttribute("aria-live")).toBe("off");';
  expect(unasserted(regions, `expect(a.getAttribute("aria-live")).toBe("polite"); ${off}`), "two regions, one read").toStrictEqual(['aria-live="polite": set 2 times, read by value 1']);
  expect(unasserted(regions, `expect(a.getAttribute("aria-live")).toBe("polite"); expect(b).toHaveAttribute("aria-live", "polite"); ${off}`), "each occurrence read").toStrictEqual([]);
});

test("refuses a spelling it cannot read, never reading it as absent (deep reviews 3 and 4)", () => {
  const read = 'getByRole("img"); getByRole("img");';
  expect(unasserted("<canvas role={kind} />", read)).toStrictEqual(['role={kind}: write a role as role="…"']);
  expect(unasserted("<canvas role={`img`} />", read), "a template is a string").toStrictEqual([]);
  expect(unasserted("<canvas role='img' />", read), "single quotes are a string").toStrictEqual([]);
  expect(unasserted("<canvas {...props} />", read)).toStrictEqual(["<canvas {...props}>: no spread on an intrinsic element, write each attribute"]);
  expect(unasserted('<canvas role="img" { ...rest } />', read)).toStrictEqual(['<canvas { ...rest }>: no spread on an intrinsic element, write each attribute']);
  // A spread after an attribute holding `=>`, whose `>` ended the element for the gate of deep review 3.
  expect(unasserted("<canvas ref={(c) => setCanvas(c)} {...rest} />", read)).toStrictEqual(["<canvas {...rest}>: no spread on an intrinsic element, write each attribute"]);
  expect(unasserted("<button onClick={() => go()} {...props}>Go</button>", read)).toStrictEqual(["<button {...props}>: no spread on an intrinsic element, write each attribute"]);
  expect(unasserted("<svg:path {...props} />", read), "a namespaced element").toStrictEqual(["<svg:path {...props}>: no spread on an intrinsic element, write each attribute"]);
  expect(unasserted('<Square {...props} /><canvas role="img" />', read), "a component's own props, its attributes in its own source").toStrictEqual([]);
  expect(unasserted('region.setAttribute("aria-live", "polite");', read), "an attribute set outside JSX").toStrictEqual(['"aria-live": set an accessibility attribute in JSX, name="…" or name={…}']);
  expect(unasserted("const props = { role: kind, 'aria-label': name };", read), "an attribute in an object").toStrictEqual(["'aria-label': set an accessibility attribute in JSX, name=\"…\" or name={…}"]);
});

test("counts each form of a read it knows, and only its value (deep review 5)", () => {
  const source = '<p aria-live="polite" aria-describedby={SAYS} aria-busy />';
  const others = 'expect(p).toHaveAttribute("aria-busy", "true"); getByRole("note", { description: LINE });';
  for (const [what, tests] of [
    ["toEqual", 'expect(p.getAttribute("aria-live")).toEqual("polite");'],
    ["a choice between two values", 'expect(p.getAttribute("aria-live")).toBe(late ? "polite" : "off");'],
    ["a choice in parentheses", 'expect((p.getAttribute("aria-live"))).toBe((late ? ("off") : "polite"));'],
    ["the first element of a shorter expected array", 'expect([p.getAttribute("aria-live"), p.id]).toStrictEqual(["polite"]);'],
    ["a property of a name", 'expect(p.getAttribute("aria-live")).toBe("polite"); expect(p.getAttribute("aria-describedby")).toBe(IDS.says.line);'],
  ] as const) {
    expect(unasserted(source, `${tests} ${others}`), what).toStrictEqual([]);
  }
  for (const [what, tests] of [
    ["a side of a choice that is no value", 'expect(p.getAttribute("aria-live")).toBe(late ? "polite" : live());'],
    ["the other side that is no value", 'expect(p.getAttribute("aria-live")).toBe(late ? live() : "polite");'],
    ["a side that is a choice", 'expect(p.getAttribute("aria-live")).toBe(late ? "polite" : early ? "off" : "polite");'],
    ["an element past the end of a shorter array", 'expect([p.id, p.getAttribute("aria-live")]).toStrictEqual(["polite"]);'],
    ["an array against what is no array", 'expect([p.getAttribute("aria-live")]).toStrictEqual(polite);'],
    ["no argument", 'expect(p.getAttribute("aria-live")).toBe();'],
    ["two arguments", 'expect(p.getAttribute("aria-live")).toBe("polite", "why");'],
    ["no subject", 'expect().toBe("polite");'],
    ["an attribute not written as a string", 'expect(p.getAttribute(name)).toBe("polite");'],
    ["another call", 'expect(p.getAttributeNS("aria-live")).toBe("polite");'],
    ["a call by its bare name", 'expect(getAttribute("aria-live")).toBe("polite");'],
    ["a matcher by a computed name", 'expect(p.getAttribute("aria-live"))["toBe"]("polite");'],
    ["another subject than expect", 'assume(p.getAttribute("aria-live")).toBe("polite");'],
  ] as const) {
    expect(unasserted(source, `${tests} ${others}`), what).toStrictEqual(['aria-live="polite": set 1 times, read by value 0']);
  }
  expect(unasserted(source, 'expect(p).toHaveAttribute("aria-live", "polite"); getByRole("note", { description: LINE });'), "an attribute without a value is set to true").toStrictEqual(['aria-busy="true": set 1 times, read by value 0']);
  expect(unasserted(source, 'expect(p).toHaveAttribute("aria-live", "polite"); expect(p).toHaveAttribute("aria-busy", "true"); getByRole("note", { descriptions: LINE, ...rest, "description": LINE });'), "a description by another spelling").toStrictEqual(["aria-describedby={SAYS}: read by value 0 times"]);
});
