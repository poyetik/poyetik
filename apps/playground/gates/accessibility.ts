// Every accessibility attribute a component sets is asserted by value in its own tests: axe
// proves the attributes valid, never that they say what they should, and the mutation tests
// do not change attribute strings in JSX, so that an attribute no test reads could be
// dropped or changed with every check green (the second deep review of visual block 2 found
// `aria-live` so).
//
// The gate reads the syntax trees of the TypeScript compiler, never the text. In a component,
// an occurrence is a JSX attribute `aria-*` or `role`. A literal value (`aria-live="polite"`)
// needs as many reads of that very value as it has occurrences; a computed value
// (`aria-label={name}`) needs a read of a value other than the attribute's literal ones, and
// a component gives one attribute at most one computed value, since no read can be tied to
// one of two. A read is one of a value: `expect(x.getAttribute(n))` (or an element of an
// array given to `expect`) asserted by `toBe`, `toEqual` or `toStrictEqual` against a string,
// a name or a property of one (or a choice between two such, the same value on both sides
// read once); `toHaveAttribute(n, value)` with such a value; and a role query that throws when
// it finds none (`getByRole("img")`), which reads the role, and, for the occurrences on an
// element of that role, a label (`{ name: … }`) or a description (`{ description: … }`), but
// one an `expect` waits to throw (inside a function given to it, or before its `rejects`). A
// selector is never a read, whatever it matches (`[aria-hidden='true']`): its answer can be
// asserted absent in more ways than a list can name, and a string that looks like one may be
// no selector at all (a test's title, a message), so an attribute is read by `getAttribute`. An element's role is
// its literal `role`, else its tag's ({@link IMPLICIT_ROLES}, a section's only with a label);
// the label of an element of no such role is read by its attribute only. A read of an
// attribute is counted by value, not tied to one element. A presence (`hasAttribute`,
// `not.toBeNull()`, a comparison, `typeof`, a query that answers null) or
// a matcher of anything (`expect.anything()`, `expect.any(…)`) is no read. A role not
// written as a string, a spread on an intrinsic element (which may set any attribute) and an
// accessibility attribute named outside JSX (in a string, such as `setAttribute`'s, or by a
// property that reflects it, `ariaLabel`, as an object's key or assigned) are refused, never
// read as absent, in every source of the page, its folders and `.ts` files included. A string
// that is exactly such a name is refused wherever it is written, a JSX attribute's expression
// (`title={"role"}`) and a method's name included. Beyond a reading of the syntax: markup
// written as a string (`innerHTML`), a name composed at run time (`` `aria-${kind}` ``), and a
// query thrown through a name or a `try` (`const find = () => getByRole(…)`, then
// `expect(find).toThrow()`), which is counted as a read.
import ts from "typescript";

/** The syntax tree of `text`, read as the language of file `file`. */
function parse(text: string, file: string): ts.SourceFile {
  return ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true);
}

/** Every part of the syntax tree from `root`, `root` first, depth first. */
function nodes(root: ts.Node): readonly ts.Node[] {
  const all = [root];
  ts.forEachChild(root, (child) => {
    all.push(...nodes(child));
  });
  return all;
}

/** Whether `name` names an accessibility attribute: `aria-*` or `role`. */
function isAccessibility(name: string): boolean {
  return name === "role" || /^aria-[a-z]+$/u.test(name);
}

/** A value an occurrence sets or a read asserts: a literal string, or an expression computed at run time. */
export type Value = { readonly literal: string } | { readonly computed: string };

/** An attribute a component sets: its value, and the role of the element it is set on, when the source tells it. */
export interface Occurrence {
  /** The value it is set to. */
  readonly value: Value;
  /** The role of its element, undefined when the source does not tell it. */
  readonly role: string | undefined;
}

/** What a component sets: each attribute's values, one per occurrence, and what it writes in a way the gate cannot read. */
export interface Accessibility {
  /** What sets each attribute, one per occurrence, by attribute name (`role` included). */
  readonly attributes: ReadonlyMap<string, readonly Occurrence[]>;
  /** What it writes unreadably, each with the form to write instead. */
  readonly unreadable: readonly string[];
}

/** The value of JSX attribute `attribute`: the string it is set to, or the expression computing it. */
function attributeValue(attribute: ts.JsxAttribute): Value {
  const { initializer } = attribute;
  if (initializer === undefined) {
    return { literal: "true" };
  }
  const expression = ts.isJsxExpression(initializer) ? initializer.expression : initializer;
  return expression !== undefined && ts.isStringLiteralLike(expression) ? { literal: expression.text } : { computed: initializer.getText() };
}

/** The roles of the elements that have one by their tag alone, but `section`, a region only with a label. */
const IMPLICIT_ROLES: ReadonlyMap<string, string> = new Map([
  ["button", "button"],
  ["details", "group"],
  ["ul", "list"],
]);

/** The role of the element holding `attribute`: its own `role` (as written when computed, which the gate refuses), else its tag's, else undefined. */
function roleOf(attribute: ts.JsxAttribute): string | undefined {
  const attributes = attribute.parent.properties.filter((each) => ts.isJsxAttribute(each));
  const explicit = attributes.find((each) => each.name.getText() === "role");
  if (explicit !== undefined) {
    const value = attributeValue(explicit);
    return "literal" in value ? value.literal : value.computed;
  }
  const tag = attribute.parent.parent.tagName.getText();
  const labelled = attributes.some((each) => /^aria-label(?:ledby)?$/u.test(each.name.getText()));
  return tag === "section" && labelled ? "region" : IMPLICIT_ROLES.get(tag);
}

/** Whether `tag` is an intrinsic element's: lower case, or namespaced. */
function isIntrinsic(tag: ts.JsxTagNameExpression): boolean {
  return ts.isJsxNamespacedName(tag) || (ts.isIdentifier(tag) && /^[a-z]/u.test(tag.text));
}

/** Whether `name` is a property that reflects an accessibility attribute, `role` or `aria…`: `ariaLabel`. */
function isReflected(name: string): boolean {
  return name === "role" || /^aria[A-Z][A-Za-z]*$/u.test(name);
}

/** Whether `node` names an accessibility attribute outside JSX: a string naming one, or a property that reflects one, as the key of an object's member or assigned. */
function setsOutsideJsx(node: ts.Node): boolean {
  if (ts.isStringLiteralLike(node)) {
    return (isAccessibility(node.text) || isReflected(node.text)) && !ts.isJsxAttribute(node.parent);
  }
  const { parent } = node;
  if (ts.isIdentifier(node)) {
    return ts.isObjectLiteralElementLike(parent) && parent.name === node && isReflected(node.text);
  }
  return (
    ts.isPropertyAccessExpression(node) &&
    isReflected(node.name.text) &&
    ts.isBinaryExpression(parent) &&
    parent.left === node &&
    parent.operatorToken.kind >= ts.SyntaxKind.FirstAssignment &&
    parent.operatorToken.kind <= ts.SyntaxKind.LastAssignment
  );
}

/** The accessibility attributes `source`, the text of file `file` (a component by default), sets, and what it writes in a way the gate cannot read. */
export function accessibility(source: string, file = "source.tsx"): Accessibility {
  const attributes = new Map<string, Occurrence[]>();
  const unreadable: string[] = [];
  for (const node of nodes(parse(source, file))) {
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
      attributes.set(name, [...(attributes.get(name) ?? []), { value, role: roleOf(node) }]);
    } else if (ts.isJsxSpreadAttribute(node) && isIntrinsic(node.parent.parent.tagName)) {
      unreadable.push(`<${node.parent.parent.tagName.getText()} ${node.getText()}>: no spread on an intrinsic element, write each attribute`);
    } else if (setsOutsideJsx(node)) {
      unreadable.push(`${node.getText()}: set an accessibility attribute in JSX, name="…" or name={…}`);
    }
  }
  return { attributes, unreadable };
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
    return arrays
      .filter((array) => ts.isArrayLiteralExpression(array))
      .flatMap((array) => array.elements.flatMap((each, at) => left.elements.slice(at, at + 1).map((element) => [element, each] as const)));
  }
  return [[left, right]];
}

/** The matchers asserting a value exactly. */
const EXACT = new Set(["toBe", "toEqual", "toStrictEqual"]);

/** The queries of a role that throw when they find none. */
const ROLE_QUERY = /^(?:get|find)(?:All)?ByRole$/u;

/** A read by value of an attribute, tied to an element's role when a query read it. */
export interface Read {
  /** The attribute it reads (`role` for a role query). */
  readonly name: string | undefined;
  /** The value it asserts. */
  readonly value: Value;
  /** The role of the element it reads, undefined when the read is tied to none. */
  readonly role: string | undefined;
}

/** The name `callee` is called by, its last part, or none for a computed one. */
function calledBy(callee: ts.Expression): string | undefined {
  if (ts.isIdentifier(callee)) {
    return callee.text;
  }
  return ts.isPropertyAccessExpression(callee) ? callee.name.text : undefined;
}

/** The first argument of `node`, when it is a call of `expect`, or undefined. */
function expectation(node: ts.Node): ts.Expression | undefined {
  return ts.isCallExpression(node) && ts.isIdentifier(node.expression) && node.expression.text === "expect" ? node.arguments[0] : undefined;
}

/**
 * Whether a query at `node` is one an `expect` waits to throw: inside a function (`within`)
 * given to the nearest `expect` around it, or the subject of its `rejects`.
 */
function thrown(node: ts.Node, within = false): boolean {
  if (ts.isSourceFile(node)) {
    return false;
  }
  if (expectation(node) !== undefined) {
    return within || (ts.isPropertyAccessExpression(node.parent) && node.parent.name.text === "rejects");
  }
  return thrown(node.parent, within || ts.isFunctionLike(node));
}

/** The reads of a role query by `role`, with the options `options`: the role, and the label and the description it asks for. */
function queryReads(role: string, options: ts.Expression | undefined): readonly Read[] {
  const asked = options !== undefined && ts.isObjectLiteralExpression(options) ? options.properties.filter((property) => ts.isPropertyAssignment(property)) : [];
  /** The reads of attribute `name` by option `option`. */
  const by = (option: string, name: string): readonly Read[] =>
    asked.filter((property) => property.name.getText() === option).flatMap((property) => expected(property.initializer).map((value) => ({ name, value, role })));
  return [{ name: "role", value: { literal: role }, role: undefined }, ...by("name", "aria-label"), ...by("description", "aria-describedby")];
}

/** Whether `left` and `right` are the same value. */
function equal(left: Value, right: Value): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}

/** The reads of `actual` asserted exactly `wanted`: each value an element is asserted to be, once an element whatever the choice. */
function exactReads(actual: ts.Expression, wanted: ts.Expression): readonly Read[] {
  const asserted = pairs(actual, wanted).flatMap(([element, each]) => {
    const name = attributeRead(element);
    return name === undefined ? [] : expected(each).map((value) => ({ element, name, value, role: undefined }));
  });
  return asserted
    .filter(({ element, value }, at) => asserted.findIndex((other) => other.element === element && equal(other.value, value)) === at)
    .map(({ name, value, role }) => ({ name, value, role }));
}

/** The reads of call `node`, matched on an `expect` or a role query. */
function callReads(node: ts.CallExpression): readonly Read[] {
  const called = String(calledBy(node.expression));
  const [first, second] = node.arguments;
  if (first === undefined) {
    return [];
  }
  if (ROLE_QUERY.test(called) && ts.isStringLiteralLike(first)) {
    return thrown(node.parent) ? [] : queryReads(first.text, second);
  }
  const actual = ts.isPropertyAccessExpression(node.expression) ? expectation(node.expression.expression) : undefined;
  if (actual === undefined) {
    return [];
  }
  if (EXACT.has(called) && node.arguments.length === 1) {
    return exactReads(actual, first);
  }
  return called === "toHaveAttribute" && ts.isStringLiteralLike(first) && second !== undefined ? expected(second).map((value) => ({ name: first.text, value, role: undefined })) : [];
}

/** The reads by value of `tests`. */
export function valueReads(tests: string): readonly Read[] {
  return nodes(parse(tests, "tests.tsx")).flatMap((node) => (ts.isCallExpression(node) ? callReads(node) : []));
}

/** How `name` set to `value` is written, for the messages. */
function spelled(name: string, value: Value): string {
  return "literal" in value ? `${name}="${value.literal}"` : `${name}=${value.computed}`;
}

/** How many of `occurrences`, each of value `value`, `reads` read: those tied to a role, its elements only. */
function readOf(value: Value, occurrences: readonly Occurrence[], reads: readonly Read[]): number {
  const matching = reads.filter((read) => equal(read.value, value));
  const untied = matching.filter(({ role }) => role === undefined).length;
  const tied = [...new Set(occurrences.map(({ role }) => role))].map((role) => {
    const set = occurrences.filter((occurrence) => occurrence.role === role).length;
    return role === undefined ? 0 : Math.min(set, matching.filter((read) => read.role === role).length);
  });
  return tied.reduce((sum, count) => sum + count, untied);
}

/** What `source` sets that `tests` does not read by value as many times as it sets it (reads counted by value, tied to no element), then what it writes unreadably. */
export function unasserted(source: string, tests: string): readonly string[] {
  const { attributes, unreadable } = accessibility(source);
  const reads = valueReads(tests);
  const faults: string[] = [];
  for (const [name, occurrences] of attributes) {
    // The first occurrence of each literal value.
    const literals = occurrences.filter(({ value }, at) => "literal" in value && occurrences.findIndex((other) => equal(other.value, value)) === at);
    const readsOf = reads.filter((read) => read.name === name);
    for (const { value } of literals) {
      const set = occurrences.filter((occurrence) => equal(occurrence.value, value));
      const read = readOf(value, set, readsOf);
      if (read < set.length) {
        faults.push(`${spelled(name, value)}: set ${String(set.length)} times, read by value ${String(read)}`);
      }
    }
    const computed = occurrences.filter(({ value }) => "computed" in value);
    const [only] = computed;
    if (computed.length > 1) {
      faults.push(`${name}: ${String(computed.length)} computed values (${computed.map(({ value }) => spelled(name, value)).join(", ")}), a read ties to one only: give the others a literal value`);
    } else if (only !== undefined && !readsOf.some(({ value, role }) => (role === undefined || role === only.role) && !literals.some((literal) => equal(literal.value, value)))) {
      faults.push(`${spelled(name, only.value)}: read by value 0 times`);
    }
  }
  return [...faults, ...unreadable];
}
