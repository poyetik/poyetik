// The accessibility gate (`gates/accessibility.ts`) over the page's components, and its fixtures.
import { readFileSync, readdirSync } from "node:fs";
import { expect, test } from "vitest";
import { accessibility, unasserted, valueReads } from "../gates/accessibility.ts";
import { findUp } from "./repository.ts";

/** The sources of the page, its folders included, relative to `src/`. */
const SOURCES = readdirSync(findUp("apps/playground/src"), { recursive: true, encoding: "utf8" })
  .filter((file) => /\.tsx?$/u.test(file))
  .sort();

/** The entry point, which renders no element of its own. */
const ENTRY = "main.tsx";

/** The components of the page: each `.tsx` source but the entry point. */
const COMPONENTS = SOURCES.filter((file) => file.endsWith(".tsx") && file !== ENTRY).map((file) => file.replace(/\.tsx$/u, ""));

/** The text of source `file`. */
function textOf(file: string): string {
  return readFileSync(findUp(`apps/playground/src/${file}`), "utf8");
}

test("asserts every accessibility attribute and role of each component in its own tests, by value, once per occurrence", () => {
  expect(COMPONENTS).toStrictEqual(["App", "Oscilloscope", "Square"]);
  for (const component of COMPONENTS) {
    const tests = readFileSync(findUp(`apps/playground/test/${component}.test.tsx`), "utf8");
    expect(unasserted(textOf(`${component}.tsx`), tests), component).toStrictEqual([]);
  }
});

test("finds no accessibility attribute in any other source of the page, in JSX or by a property", () => {
  const others = SOURCES.filter((file) => !file.endsWith(".tsx") || file === ENTRY);
  expect(others, "a source found").toContain("player.ts");
  for (const file of others) {
    expect(accessibility(textOf(file), file), file).toStrictEqual({ attributes: new Map(), unreadable: [] });
  }
});

test("finds an attribute no test reads: the live region before the second deep review of visual block 2", () => {
  // What revealed the rule: Square set `aria-live`, and no test of it read the attribute.
  const source = '<div aria-live="polite" data-testid="square-failure" /><canvas role="img" aria-label={name} />';
  expect(accessibility(source)).toStrictEqual({
    attributes: new Map([
      ["aria-label", [{ value: { computed: "{name}" }, role: "img" }]],
      ["aria-live", [{ value: { literal: "polite" }, role: undefined }]],
      ["role", [{ value: { literal: "img" }, role: "img" }]],
    ]),
    unreadable: [],
  });
  expect(unasserted(source, 'getByRole("img"); expect(screen.getByRole("img").getAttribute("aria-label")).toBe("x");')).toStrictEqual(['aria-live="polite": set 1 times, read by value 0']);
  expect(unasserted(source, 'getByRole("img"); expect(x.getAttribute("aria-label")).toBe("x"); expect(y).toHaveAttribute("aria-live", "polite")')).toStrictEqual([]);
  expect(unasserted(source, "nothing")).toStrictEqual(['aria-live="polite": set 1 times, read by value 0', 'role="img": set 1 times, read by value 0', "aria-label={name}: read by value 0 times"]);
  expect(unasserted(source, "getByRole(\"img\"); querySelector(\"[aria-label='x'][aria-live='polite']\")"), "matched by value in a selector").toStrictEqual([]);
  expect(unasserted(source, 'getByRole("img", { name: "x" }); toHaveAttribute("aria-live", "polite")'), "toHaveAttribute outside expect is no read").toStrictEqual(['aria-live="polite": set 1 times, read by value 0']);
  expect(unasserted(source, 'getByRole("img", { name: NAMES.loading }); expect(region).toHaveAttribute("aria-live", "polite")'), "a name found by a query").toStrictEqual([]);
  expect(unasserted(source, 'getByRole("img"); expect(region.getAttribute("aria-live")).toBe("off"); expect(x.getAttribute("aria-label")).toBe("x")'), "a read of another value").toStrictEqual(['aria-live="polite": set 1 times, read by value 0']);
});

test("refuses a read of presence alone, of anything, or with no exact matcher (third and fourth deep reviews of visual block 2)", () => {
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
    ["a property of expect", 'expect(p.getAttribute("aria-live")).toBe(expect.anything);'],
    ["another matcher of two strings", 'expect(p).toContain("aria-live", "polite");'],
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

test("ties each read to its occurrence: each literal value read as often as it is set, one computed value an attribute (fourth deep review of visual block 2)", () => {
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

test("refuses a spelling it cannot read, never reading it as absent (third and fourth deep reviews of visual block 2)", () => {
  const read = 'getByRole("img"); getByRole("img");';
  expect(unasserted("<canvas role={kind} />", read)).toStrictEqual(['role={kind}: write a role as role="…"']);
  expect(unasserted("<canvas role={`img`} />", read), "a template is a string").toStrictEqual([]);
  expect(unasserted("<canvas role='img' />", read), "single quotes are a string").toStrictEqual([]);
  expect(unasserted("<canvas {...props} />", read)).toStrictEqual(["<canvas {...props}>: no spread on an intrinsic element, write each attribute"]);
  expect(unasserted('<canvas role="img" { ...rest } />', read)).toStrictEqual(['<canvas { ...rest }>: no spread on an intrinsic element, write each attribute']);
  // A spread after an attribute holding `=>`, whose `>` ended the element for the gate of the
  // third deep review of visual block 2.
  expect(unasserted("<canvas ref={(c) => setCanvas(c)} {...rest} />", read)).toStrictEqual(["<canvas {...rest}>: no spread on an intrinsic element, write each attribute"]);
  expect(unasserted("<button onClick={() => go()} {...props}>Go</button>", read)).toStrictEqual(["<button {...props}>: no spread on an intrinsic element, write each attribute"]);
  expect(unasserted("<svg:path {...props} />", read), "a namespaced element").toStrictEqual(["<svg:path {...props}>: no spread on an intrinsic element, write each attribute"]);
  expect(unasserted('<Square {...props} /><Kit.square {...props} /><canvas role="img" />', read), "a component's own props, its attributes in its own source").toStrictEqual([]);
  expect(unasserted('region.setAttribute("aria-live", "polite");', read), "an attribute set outside JSX").toStrictEqual(['"aria-live": set an accessibility attribute in JSX, name="…" or name={…}']);
  const refused = (form: string): string => `${form}: set an accessibility attribute in JSX, name="…" or name={…}`;
  expect(unasserted("const props = { role: kind, 'aria-label': name };", read), "an attribute in an object").toStrictEqual([refused("role"), refused("'aria-label'")]);
  // What revealed it (the fifth deep review of visual block 2): an identifier key, and a
  // property that reflects the attribute, set it unseen.
  for (const [what, form] of [
    ["const props = { ariaLabel: name };", "ariaLabel"],
    ["const props = { role };", "role"],
    ["const props = { ariaHidden };", "ariaHidden"],
    ["const props = { get role() { return kind; } };", "role"],
    ["const props = { role() { return kind; } };", "role"],
    ["element.role = kind;", "element.role"],
    ["element.ariaLabel = name;", "element.ariaLabel"],
    ["element.ariaLabel ??= name;", "element.ariaLabel"],
    ["element.ariaLabel ^= name;", "element.ariaLabel"],
    ["element['role'] = kind;", "'role'"],
    ["element['aria-label'] = name;", "'aria-label'"],
    ["element[`aria-label`] = name;", "`aria-label`"],
    ["const props = { ['ariaLabel']: name };", "'ariaLabel'"],
    ["element['ariaLabel'] = name;", "'ariaLabel'"],
  ] as const) {
    expect(accessibility(what, "element.ts").unreadable, what).toStrictEqual([refused(form)]);
  }
  for (const what of [
    "const props = { roles: kind, variant: aria, arial: font };",
    "const role = element.role; const same = element.ariaLabel === name;",
    "element.roles = kind; element.variant = aria;",
    "const { role, ariaLabel } = element;",
    "type Props = { role: string };",
    "element.role in table; kind = element.role; element.title = kind;",
    "const props = { kind: role, label: ariaLabel };",
    "const ariaLabel2 = { ariaLabel2: 1, 'aria-live off': 2, 'unaria-live': 3 };",
    "const region = 'region'; const aria = 'ariaria';",
    "const unariaLabel = { unariaLabel: 1, myAria: 2, unrole: 3 };",
  ]) {
    expect(accessibility(what, "element.ts"), what).toStrictEqual({ attributes: new Map(), unreadable: [] });
  }
  expect(accessibility("const set = <T>(value: T): T => value; const wide = <string>value; element.role = kind;", "element.ts").unreadable, "read as TypeScript, not as TSX").toStrictEqual([refused("element.role")]);
});

test("counts each form of a read it knows, and only its value (fifth deep review of visual block 2)", () => {
  const source = '<p aria-live="polite" aria-describedby={SAYS} aria-busy role="note" />';
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

test("counts no absence, no test's title and no negated selector as a read (fifth deep review of visual block 2)", () => {
  // What revealed it: App's absent alert, `queryByRole("alert")` asserted null, read its role.
  const alert = '<p role="alert" />';
  const unread = ['role="alert": set 1 times, read by value 0'];
  expect(unasserted(alert, 'expect(screen.queryByRole("alert")).toBeNull();'), "an absent role").toStrictEqual(unread);
  for (const query of ["queryByRole", "queryAllByRole", "getByRoles", "forgetByRole", "getByText"]) {
    expect(unasserted(alert, `screen.${query}("alert");`), query).toStrictEqual(unread);
  }
  for (const query of ["getByRole", "getAllByRole", "findByRole", "findAllByRole"]) {
    expect(unasserted(alert, `screen.${query}("alert");`), query).toStrictEqual([]);
    expect(unasserted(alert, `${query}("alert", { hidden: true });`), `${query}, bare`).toStrictEqual([]);
  }
  expect(unasserted(alert, "screen.getByRole(kind); getByRole(); screen[`getByRole`]('alert');"), "no role, or a computed call").toStrictEqual(unread);
  const hidden = '<span aria-hidden="true" />';
  for (const [what, tests] of [
    ["toBeNull", "expect(document.querySelector(\"[aria-hidden='true']\")).toBeNull();"],
    ["toBeUndefined", "expect(one(\"[aria-hidden='true']\")).toBeUndefined();"],
    ["toBeFalsy", "expect(one(\"[aria-hidden='true']\")).toBeFalsy();"],
    ["not", "expect(one(\"[aria-hidden='true']\")).not.toBe(sizer);"],
    ["toBe(null)", "expect(one(\"[aria-hidden='true']\")?.textContent ?? null).toBe(null);"],
    ["toBe(undefined)", "expect(one(\"[aria-hidden='true']\")?.textContent).toBe(undefined);"],
    ["toBe(false)", "expect(it.matches(\"[aria-hidden='true']\")).toBe(false);"],
    ["toHaveLength(0)", "expect(all(\"[aria-hidden='true']\")).toHaveLength(0);"],
    ["toStrictEqual([])", "expect([...all(\"[aria-hidden='true']\")]).toStrictEqual([]);"],
    ["a test's title", "test(\"hides [aria-hidden='true']\", () => {});"],
    ["an it's title", "it(\"hides [aria-hidden='true']\", () => {});"],
    ["a describe's title", "describe(\"[aria-hidden='true']\", () => {});"],
    ["a string given to no call", "const selector = \"[aria-hidden='true']\";"],
    ["a string given to a computed call", "document['querySelector'](\"[aria-hidden='true']\");"],
    ["a negation", "one(\"span:not([aria-hidden='true']).sizer\");"],
    ["unlike quotes", "one(\"[aria-hidden='true`]\");"],
  ] as const) {
    expect(unasserted(hidden, tests), what).toStrictEqual(['aria-hidden="true": set 1 times, read by value 0']);
  }
  for (const [what, tests] of [
    ["a matcher of a value", "expect(one(\"[aria-hidden='true']\").textContent).toBe(\"Start\");"],
    ["a length", "expect(all(\"[aria-hidden='true']\")).toHaveLength(3);"],
    ["no matcher's argument", "expect(one(\"[aria-hidden='true']\")).toBeDefined();"],
    ["an expect no matcher follows", "keep(expect(one(\"[aria-hidden='true']\")));"],
    ["an expect whose property is not called", "keep(expect(one(\"[aria-hidden='true']\")).resolves);"],
    ["another call's negation", "assume(one(\"[aria-hidden='true']\")).not.toBe(sizer);"],
    ["beside a negation", "one(\"p:not([hidden]) [aria-hidden='true']\");"],
    ["double quotes", "one('[aria-hidden=\"true\"]');"],
  ] as const) {
    expect(unasserted(hidden, tests), what).toStrictEqual([]);
  }
  expect(unasserted('<span aria-hidden="true" /><p aria-live="off" />', "one(\"[aria-hidden='true'][aria-live='off']\");"), "each attribute of a selector").toStrictEqual([]);
});

test("counts one read an element, whatever the choice it is asserted against (fifth deep review of visual block 2)", () => {
  const regions = '<p aria-live="polite" /><p aria-live="polite" />';
  const once = ['aria-live="polite": set 2 times, read by value 1'];
  expect(unasserted(regions, 'expect([says(), p.getAttribute("aria-live")]).toStrictEqual(late ? [A, "polite"] : [B, "polite"]);'), "either array").toStrictEqual(once);
  expect(unasserted(regions, 'expect(p.getAttribute("aria-live")).toBe(late ? "polite" : "polite");'), "either value").toStrictEqual(once);
  expect(unasserted(regions, 'expect([a.getAttribute("aria-live"), b.getAttribute("aria-live")]).toStrictEqual(["polite", "polite"]);'), "two elements").toStrictEqual([]);
  expect(unasserted('<p aria-live="polite" /><p aria-live="off" />', 'expect(p.getAttribute("aria-live")).toBe(late ? "polite" : "off");'), "two values").toStrictEqual([]);
});

test("ties a label or a description read by a query to the role queried (fifth deep review of visual block 2)", () => {
  // What revealed it: App's button named by its text, `getByRole("button", { name: said })`,
  // read any computed label beside it, and a region's name any other element's label.
  for (const [element, role] of [
    ["<button aria-label={name} />", "button"],
    ["<details aria-label={name} />", "group"],
    ["<ul aria-label={name} />", "list"],
    ["<section aria-label={name} />", "region"],
  ] as const) {
    expect(unasserted(element, `getByRole("${role}", { name: said });`), `${element}, its own role`).toStrictEqual([]);
    expect(unasserted(element, `getByRole("heading", { name: said });`), `${element}, another role`).toStrictEqual(["aria-label={name}: read by value 0 times"]);
    expect(unasserted(element, `expect(x.getAttribute("aria-label")).toBe(said);`), `${element}, by its attribute`).toStrictEqual([]);
  }
  for (const element of ["<p aria-label={name} />", "<canvas aria-label={name} />", "<section aria-describedby={name} />"]) {
    expect(unasserted(element, 'getByRole("paragraph", { name: said, description: said }); getByRole("region", { name: said, description: said });'), `${element}, of no role`).toStrictEqual([`${element.slice(element.indexOf("aria"), -3)}: read by value 0 times`]);
  }
  expect(unasserted("<section aria-labelledby={id} aria-describedby={says} />", 'getByRole("region", { description: said }); expect(x).toHaveAttribute("aria-labelledby", id);'), "a section labelled by another element").toStrictEqual([]);
  expect(Object.fromEntries(accessibility('<button {...rest} role={kind} aria-label={name} aria-live={/* none */} aria-busy={`true`} data-aria-live="off" />').attributes), "a computed role, as written, and the values").toStrictEqual({
    "aria-label": [{ value: { computed: "{name}" }, role: "{kind}" }],
    "aria-live": [{ value: { computed: "{/* none */}" }, role: "{kind}" }],
    "aria-busy": [{ value: { literal: "true" }, role: "{kind}" }],
  });
  for (const other of ["data-aria-label", "aria-labels", "aria-labelled"]) {
    expect(unasserted(`<section ${other}="x" aria-describedby={says} />`, 'getByRole("region", { description: said });'), `no label, ${other}`).toContain("aria-describedby={says}: read by value 0 times");
  }
  expect(unasserted("<button role={kind} aria-label={name} />", 'getByRole("button", { name: said });'), "a computed role").toStrictEqual(["aria-label={name}: read by value 0 times", 'role={kind}: write a role as role="…"']);
  const lists = '<ul aria-label="A" /><button aria-label="A" />';
  expect(unasserted(lists, 'getByRole("list", { name: "A" }); getByRole("list", { name: "A" });'), "one element read twice").toStrictEqual(['aria-label="A": set 2 times, read by value 1']);
  expect(unasserted(lists, 'getByRole("list", { name: "A" }); getByRole("button", { name: "A" });'), "each element read").toStrictEqual([]);
  expect(unasserted(lists, 'getByRole("list", { name: "A" }); expect(x).toHaveAttribute("aria-label", "A");'), "one by its attribute").toStrictEqual([]);
  expect(unasserted('<p aria-label="A" /><ul aria-label="A" />', 'getByRole("list", { name: "A" }); getByRole("paragraph", { name: "A" });'), "an element of no role").toStrictEqual(['aria-label="A": set 2 times, read by value 1']);
  expect(unasserted("<ul aria-label={name} /><ul aria-label=\"A\" />", 'getByRole("list", { name: "A" }); getByRole("list", { name: "A" });'), "the literal read twice, the computed never").toStrictEqual(["aria-label={name}: read by value 0 times"]);
  expect(unasserted("<ul aria-label={name} />", 'getByRole("list", { ...named, name: said }); getByRole("list", options);'), "beside a spread").toStrictEqual([]);
});

test("reads exactly the reads of the tests, and nothing of the rest", () => {
  for (const nothing of [
    'f(); f()(); f()("x"); f("x"); screen.getByText("alert"); "s"; const t = g(1); `[aria-live="off"]`;',
    'expect(); expect(a); expect(a).toBe(); expect(says()).toBe("x"); expect([says()]).toEqual(["x", "y"]);',
    'expect(p.getAttribute()).toBe("x"); expect(p.getAttribute(role)).toBe("img"); expect(p).toHaveAttribute(); expect(p).toHaveAttribute(role, "img"); expect(p).toHaveAttribute("role");',
    "one(\"p[hidden]\"); one(\"[aria-live]\"); one(\"p:not([aria-live='off']):not([aria-busy='true'])\");",
    'expect([a.getAttribute("role"), b.id]).toStrictEqual(late ? [] : other);',
  ]) {
    expect(valueReads(nothing), nothing).toStrictEqual([]);
  }
  expect(valueReads('render(<p>it\'s {f("x")}</p>); getByRole("list", { name: "A", description: SAYS, level: 1 });'), "a query, after JSX").toStrictEqual([
    { name: "role", value: { literal: "list" }, role: undefined },
    { name: "aria-label", value: { literal: "A" }, role: "list" },
    { name: "aria-describedby", value: { computed: "SAYS" }, role: "list" },
  ]);
  expect(valueReads("one(\"[aria-live='off']:not([hidden])[aria-busy='true']\"); const kept = expect(one(\"[aria-live='on']\")).resolves;"), "selectors").toStrictEqual([
    { name: "aria-live", value: { literal: "off" }, role: undefined },
    { name: "aria-busy", value: { literal: "true" }, role: undefined },
    { name: "aria-live", value: { literal: "on" }, role: undefined },
  ]);
  expect(valueReads('expect([a.getAttribute("role"), b.id]).toStrictEqual(late ? ["img", B] : (["list", B])); expect(p).toHaveAttribute("aria-live", late ? "off" : LIVE);'), "matchers").toStrictEqual([
    { name: "role", value: { literal: "img" }, role: undefined },
    { name: "role", value: { literal: "list" }, role: undefined },
    { name: "aria-live", value: { literal: "off" }, role: undefined },
    { name: "aria-live", value: { computed: "LIVE" }, role: undefined },
  ]);
});
