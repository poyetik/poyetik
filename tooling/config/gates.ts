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

/** The matchers whose regular expression `unmatched` reads: every one the lint holds to a pattern (`toThrow`, `toThrowError`). */
const THROW_MATCHERS = ["toThrow", "toThrowError"];

/**
 * Those of `patterns` that no test of `tests` (each its file's name, which sets the language
 * it is parsed in, and its text) gives a matcher of {@link THROW_MATCHERS} as its whole
 * argument, however wrapped.
 */
export function unmatched(patterns: readonly string[], tests: readonly (readonly [string, string])[]): string[] {
  const matched = new Set<string>();
  const visit = (node: ts.Node): void => {
    if (ts.isCallExpression(node) && ts.isPropertyAccessExpression(node.expression) && THROW_MATCHERS.includes(node.expression.name.text)) {
      const [argument] = node.arguments;
      if (argument !== undefined && ts.isRegularExpressionLiteral(argument)) {
        matched.add(argument.text.slice(1, argument.text.lastIndexOf("/")));
      }
    }
    ts.forEachChild(node, visit);
  };
  for (const [file, text] of tests) {
    visit(ts.createSourceFile(file, text, ts.ScriptTarget.Latest));
  }
  return patterns.filter((pattern) => !matched.has(pattern));
}
