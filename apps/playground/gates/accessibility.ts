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
import ts from "typescript";

/** The syntax tree of `text`, read as TSX. */
function parse(text: string): ts.SourceFile {
  return ts.createSourceFile("source.tsx", text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
}

/** Every part of the syntax tree from `root`, `root` first, depth first. */
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
export type Value = { readonly literal: string } | { readonly computed: string };

/** What a component sets: each attribute's values, one per occurrence, and what it writes in a way the gate cannot read. */
export interface Accessibility {
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
export function accessibility(source: string): Accessibility {
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

/** A selector matching an accessibility attribute's value, `[aria-hidden='true']`: its attribute, its quote and its value. */
const SELECTED = /\[(aria-[a-z]+)=([\u0022\u0027])(.*?)\2\]/gu;

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
      node.text.replace(SELECTED, (matched: string, name: string, _quote: string, value: string) => {
        read(name, [{ literal: value }]);
        return matched;
      });
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
export function unasserted(source: string, tests: string): readonly string[] {
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
