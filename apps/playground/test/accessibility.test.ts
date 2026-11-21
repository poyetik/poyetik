// Every accessibility attribute a component sets is asserted by value in its own tests: axe
// proves the attributes valid, never that they say what they should, and the mutation tests
// do not change attribute strings in JSX, so that an attribute no test reads could be
// dropped or changed with every check green (deep review 2 found `aria-live` so).
//
// The gate reads the syntax trees of the TypeScript compiler, never the text. In a component,
// an occurrence is a JSX attribute `aria-*` or `role`. A literal value (`aria-live="polite"`)
// needs as many reads of that very value as it has occurrences; a computed value
// (`aria-label={name}`) needs a read of a value other than the attribute's literal ones, and
// a component gives one attribute at most one computed value, since no read can be tied to
// one of two. A read is one of a value: `expect(x.getAttribute(n))` (or an element of an
// array given to `expect`) asserted by `toBe`, `toEqual` or `toStrictEqual` against a string,
// a name or a property of one (or a choice between two such); `toHaveAttribute(n, value)`
// with such a value; a selector in a string matching a value (`[aria-hidden='true']`); and
// a role query (`getByRole("img")`, `{ name: … }` for a label, `{ description: … }` for a
// description). A presence (`hasAttribute`, `not.toBeNull()`, a comparison, `typeof`) or
// a matcher of anything (`expect.anything()`, `expect.any(…)`) is no read. A role not
// written as a string, a spread on an intrinsic element (which may set any attribute) and an
// accessibility attribute named in a string outside JSX (`setAttribute`) are refused, never
// read as absent.
import { readFileSync, readdirSync } from "node:fs";
import ts from "typescript";
import { expect, test } from "vitest";
import { findUp } from "./repository.ts";

/** The components of the page: each `src/*.tsx` but the entry point, and its test file. */
const COMPONENTS = readdirSync(findUp("apps/playground/src"))
  .filter((file) => file.endsWith(".tsx") && file !== "main.tsx")
  .map((file) => file.replace(/\.tsx$/u, ""));

/** The syntax tree of `text`, read as TSX. */
function parse(text: string): ts.SourceFile {
  return ts.createSourceFile("source.tsx", text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
}

/** Every node of `root`, depth first. */
function nodes(root: ts.Node): readonly ts.Node[] {
  const all: ts.Node[] = [];
  const visit = (node: ts.Node): void => {
    all.push(node);
    ts.forEachChild(node, visit);
  };
  visit(root);
  return all;
}

/** Whether `name` names an accessibility attribute: `aria-*` or `role`. */
function isAccessibility(name: string): boolean {
  return name === "role" || /^aria-[a-z]+$/u.test(name);
}

/** A value an occurrence sets or a read asserts: a literal string, or an expression computed at run time. */
type Value = { readonly literal: string } | { readonly computed: string };

/** What a component sets: each attribute's values, one per occurrence, and what it writes in a way the gate cannot read. */
interface Accessibility {
  /** The values of each attribute, one per occurrence, by attribute name (`role` included). */
  readonly attributes: ReadonlyMap<string, readonly Value[]>;
  /** What it writes unreadably, each with the form to write instead. */
  readonly unreadable: readonly string[];
}

/** The value of JSX attribute `attribute`: the string it is set to, or the expression computing it. */
function attributeValue(attribute: ts.JsxAttribute): Value {
  const { initializer } = attribute;
  if (initializer === undefined) {
    return { literal: "true" };
  }
  if (ts.isStringLiteral(initializer)) {
    return { literal: initializer.text };
  }
  if (ts.isJsxExpression(initializer) && initializer.expression !== undefined && ts.isStringLiteralLike(initializer.expression)) {
    return { literal: initializer.expression.text };
  }
  return { computed: initializer.getText() };
}

/** Whether `tag` is an intrinsic element's: lower case, or namespaced. */
function isIntrinsic(tag: ts.JsxTagNameExpression): boolean {
  return ts.isJsxNamespacedName(tag) || (ts.isIdentifier(tag) && /^[a-z]/u.test(tag.text));
}

/** The accessibility attributes `source` sets, and what it writes in a way the gate cannot read. */
function accessibility(source: string): Accessibility {
  const attributes = new Map<string, Value[]>();
  const unreadable: string[] = [];
  for (const node of nodes(parse(source))) {
    if (ts.isJsxAttribute(node)) {
      const name = node.name.getText();
      if (!isAccessibility(name)) {
        continue;
      }
      const value = attributeValue(node);
      if (name === "role" && "computed" in value) {
        unreadable.push(`${node.getText()}: write a role as role="…"`);
        continue;
      }
      attributes.set(name, [...(attributes.get(name) ?? []), value]);
    } else if (ts.isJsxSpreadAttribute(node) && isIntrinsic(node.parent.parent.tagName)) {
      unreadable.push(`<${node.parent.parent.tagName.getText()} ${node.getText()}>: no spread on an intrinsic element, write each attribute`);
    } else if (ts.isStringLiteralLike(node) && isAccessibility(node.text) && !ts.isJsxAttribute(node.parent)) {
      unreadable.push(`${node.getText()}: set an accessibility attribute in JSX, name="…" or name={…}`);
    }
  }
  return { attributes: new Map([...attributes].sort(([left], [right]) => (left < right ? -1 : 1))), unreadable };
}

/** `node` without the parentheses around it. */
function unwrap(node: ts.Expression): ts.Expression {
  return ts.isParenthesizedExpression(node) ? unwrap(node.expression) : node;
}

/** Whether `node` is a name or a property of one, never `undefined` nor a property of `expect` (its matchers of anything). */
function isNamed(node: ts.Expression): boolean {
  if (ts.isIdentifier(node)) {
    return node.text !== "undefined" && node.text !== "expect";
  }
  return ts.isPropertyAccessExpression(node) && isNamed(node.expression);
}

/** The values `node` asserts, as an expected value: a string, a name or a property of one, or both sides of a choice between them; none for anything else. */
function expected(given: ts.Expression): readonly Value[] {
  const node = unwrap(given);
  if (ts.isStringLiteralLike(node)) {
    return [{ literal: node.text }];
  }
  if (isNamed(node)) {
    return [{ computed: node.getText() }];
  }
  if (ts.isConditionalExpression(node)) {
    const sides = [expected(node.whenTrue), expected(node.whenFalse)];
    return sides.every((side) => side.length === 1) ? sides.flat() : [];
  }
  return [];
}

/** The attribute `node` reads, when it is `x.getAttribute("<name>")`. */
function attributeRead(given: ts.Expression): string | undefined {
  const node = unwrap(given);
  if (!ts.isCallExpression(node) || !ts.isPropertyAccessExpression(node.expression) || node.expression.name.text !== "getAttribute") {
    return undefined;
  }
  const [name] = node.arguments;
  return name !== undefined && ts.isStringLiteralLike(name) ? name.text : undefined;
}

/** The pairs of a value asserted and its expected value: `actual` itself, or each element of an array against the same element of each array `wanted` can be. */
function pairs(actual: ts.Expression, wanted: ts.Expression): readonly (readonly [ts.Expression, ts.Expression])[] {
  const left = unwrap(actual);
  const right = unwrap(wanted);
  if (ts.isArrayLiteralExpression(left)) {
    const arrays = ts.isConditionalExpression(right) ? [unwrap(right.whenTrue), unwrap(right.whenFalse)] : [right];
    return arrays.flatMap((array) =>
      ts.isArrayLiteralExpression(array)
        ? left.elements.flatMap((element, at) => {
            const same = array.elements[at];
            return same === undefined ? [] : [[element, same] as const];
          })
        : [],
    );
  }
  return [[left, right]];
}

/** The matchers asserting a value exactly. */
const EXACT = new Set(["toBe", "toEqual", "toStrictEqual"]);

/** The reads by value of `tests`, each with the attribute it reads (`role` for a role query). */
function valueReads(tests: string): readonly (readonly [string, Value])[] {
  const reads: [string, Value][] = [];
  const read = (name: string, values: readonly Value[]): void => {
    for (const value of values) {
      reads.push([name, value]);
    }
  };
  for (const node of nodes(parse(tests))) {
    if (ts.isStringLiteralLike(node)) {
      for (const [, name, , value] of node.text.matchAll(/\[(aria-[a-z]+)=(['"])(.*?)\2\]/gu)) {
        if (name !== undefined && value !== undefined) {
          read(name, [{ literal: value }]);
        }
      }
      continue;
    }
    if (!ts.isCallExpression(node)) {
      continue;
    }
    const callee = node.expression;
    const called = ts.isIdentifier(callee) ? callee.text : ts.isPropertyAccessExpression(callee) ? callee.name.text : "";
    const [first, second] = node.arguments;
    if (called.endsWith("ByRole") && first !== undefined && ts.isStringLiteralLike(first)) {
      read("role", [{ literal: first.text }]);
      if (second !== undefined && ts.isObjectLiteralExpression(second)) {
        for (const property of second.properties) {
          if (ts.isPropertyAssignment(property) && ts.isIdentifier(property.name)) {
            const by = property.name.text === "name" ? "aria-label" : property.name.text === "description" ? "aria-describedby" : undefined;
            if (by !== undefined) {
              read(by, expected(property.initializer));
            }
          }
        }
      }
      continue;
    }
    if (!ts.isPropertyAccessExpression(callee)) {
      continue;
    }
    const subject = callee.expression;
    if (!ts.isCallExpression(subject) || !ts.isIdentifier(subject.expression) || subject.expression.text !== "expect") {
      continue;
    }
    const [actual] = subject.arguments;
    if (actual === undefined || first === undefined) {
      continue;
    }
    if (EXACT.has(called) && node.arguments.length === 1) {
      for (const [element, wanted] of pairs(actual, first)) {
        const name = attributeRead(element);
        if (name !== undefined) {
          read(name, expected(wanted));
        }
      }
    } else if (called === "toHaveAttribute" && ts.isStringLiteralLike(first) && second !== undefined) {
      read(first.text, expected(second));
    }
  }
  return reads;
}

/** How `name` set to `value` is written, for the messages. */
function spelled(name: string, value: Value): string {
  return "literal" in value ? `${name}="${value.literal}"` : `${name}=${value.computed}`;
}

/** What `source` sets that `tests` does not read by value once per occurrence, then what it writes unreadably. */
function unasserted(source: string, tests: string): readonly string[] {
  const { attributes, unreadable } = accessibility(source);
  const reads = valueReads(tests);
  const faults: string[] = [];
  for (const [name, values] of attributes) {
    const literals = values.flatMap((value) => ("literal" in value ? [value.literal] : []));
    const computed = values.filter((value) => "computed" in value);
    const readsOf = reads.filter(([read]) => read === name).map(([, value]) => value);
    for (const literal of new Set(literals)) {
      const set = literals.filter((each) => each === literal).length;
      const read = readsOf.filter((value) => "literal" in value && value.literal === literal).length;
      if (read < set) {
        faults.push(`${spelled(name, { literal })}: set ${String(set)} times, read by value ${String(read)}`);
      }
    }
    if (computed.length > 1) {
      faults.push(`${name}: ${String(computed.length)} computed values (${computed.map((value) => spelled(name, value)).join(", ")}), a read ties to one only: give the others a literal value`);
    } else if (computed[0] !== undefined) {
      const read = readsOf.filter((value) => !("literal" in value && literals.includes(value.literal))).length;
      if (read === 0) {
        faults.push(`${spelled(name, computed[0])}: read by value 0 times`);
      }
    }
  }
  return [...faults, ...unreadable];
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
  expect(unasserted('node.setAttribute("aria-live", "polite");', read), "an attribute set outside JSX").toStrictEqual(['"aria-live": set an accessibility attribute in JSX, name="…" or name={…}']);
  expect(unasserted("const props = { role: kind, 'aria-label': name };", read), "an attribute in an object").toStrictEqual(["'aria-label': set an accessibility attribute in JSX, name=\"…\" or name={…}"]);
});
