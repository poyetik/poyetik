// What no lint rule holds of the policy's own words: the rules its messages cite, and the tests
// its reviewed patterns serve.
import ts from "typescript";

/** Each mention of `CLAUDE.md` in `messages`, up to the parenthesis that closes it, if any. */
export function citations(messages: string): string[] {
  return [...messages.matchAll(/\(CLAUDE\.md[^)]*\)|CLAUDE\.md/gu)].map(([cited]) => cited);
}

/**
 * Those of `cited` ({@link citations}) that are not `(CLAUDE.md: <title>)`, the title of a rule
 * of `rules` (the text of `CLAUDE.md`, a rule a bold entry of a list), in any case.
 */
export function uncited(cited: readonly string[], rules: string): string[] {
  const titles = new Set([...rules.matchAll(/^- \*\*([^*]+?)\.?\*\*/gmu)].map(([, title]) => `(claude.md: ${String(title).toLowerCase()})`));
  return cited.filter((citation) => !titles.has(citation.toLowerCase()));
}

/** The patterns `source`, a test file, gives `toThrow` as its whole argument, however wrapped. */
function thrown(source: string): string[] {
  const patterns: string[] = [];
  const visit = (node: ts.Node): void => {
    if (ts.isCallExpression(node) && ts.isPropertyAccessExpression(node.expression) && node.expression.name.text === "toThrow") {
      const [argument] = node.arguments;
      if (argument !== undefined && ts.isRegularExpressionLiteral(argument)) {
        patterns.push(argument.text.slice(1, argument.text.lastIndexOf("/")));
      }
    }
    ts.forEachChild(node, visit);
  };
  visit(ts.createSourceFile("", source, ts.ScriptTarget.Latest));
  return patterns;
}

/** Those of `patterns` that no source of `tests` gives `toThrow`. */
export function unmatched(patterns: readonly string[], tests: readonly string[]): string[] {
  const matched = new Set(tests.flatMap(thrown));
  return patterns.filter((pattern) => !matched.has(pattern));
}
