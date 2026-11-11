// The strict linting policy shared by every TypeScript package (see CLAUDE.md): no escape
// hatch can weaken the type system. Each package's eslint.config.js calls `strict`.
import js from "@eslint/js";
import type { Linter } from "eslint";
import { defineConfig, type Config } from "eslint/config";
import jsdoc from "eslint-plugin-jsdoc";
import tseslint from "typescript-eslint";

/**
 * What `jsdoc/require-jsdoc` requires a comment on, besides functions, classes and their
 * methods: every other named item, exported or not, as Rust's `missing_docs` and
 * `missing_docs_in_private_items` do. A policy fixture triggers each one
 * (`test/policy.test.ts` fails on a context no fixture triggers).
 */
export const JSDOC_CONTEXTS: readonly string[] = [
  "TSInterfaceDeclaration",
  "TSTypeAliasDeclaration",
  "PropertyDefinition",
  // The members of an interface, as the members of a class (named object types are
  // interfaces: consistent-type-definitions). Not those of an inline object type, such as
  // a parameter's, which are no more items than inline callbacks are.
  "TSInterfaceBody > TSPropertySignature",
  "TSInterfaceBody > TSMethodSignature",
  // Constants at the top level of a module, or of a module declaration (`declare module`).
  ":matches(Program, TSModuleBlock) > VariableDeclaration",
  "ExportNamedDeclaration[declaration.type='VariableDeclaration']",
];

/** Why `fetch` is refused outside {@link FETCH_HELPERS}. */
const FETCH_MESSAGE = "Fetch through the package's checked helper (its fetch.ts), which refuses a response that is not 2xx and names the URL.";

/**
 * Why reading a clock is refused outside {@link CLOCK_READERS}: tests are deterministic, and
 * the engine counts time in samples.
 */
const CLOCK_MESSAGE = "Read no clock (CLAUDE.md: tests are deterministic): only the files of CLOCK_READERS read one.";

/** The restricted syntax that is not a clock read: what a clock reader keeps refusing. */
const SYNTAX: readonly { readonly selector: string; readonly message: string }[] = [
  { selector: "TSEnumDeclaration", message: "Use a union of string literals or an `as const` object." },
  { selector: "TSNamespaceExportDeclaration, TSModuleDeclaration[global=true]", message: "No global augmentation." },
  // A focused or skipped test drops the others or itself silently (Vitest and
  // Playwright: `test.only`, `describe.skip`, `test.describe.fixme`, `it.todo`…).
  {
    selector:
      "MemberExpression[property.name=/^(only|skip|todo|fixme|skipIf|runIf)$/]:matches([object.name=/^(describe|it|test|suite)$/], [object.object.name=/^(describe|it|test|suite)$/])",
    message: "Never focus or skip a test: run it, fix it, or discuss it (CLAUDE.md, testing policy).",
  },
];

/**
 * The clock reads written as syntax: the current date and time read without a timestamp
 * (`new Date()`, `Date()`), directly or through the global object (`globalThis`, `window`,
 * `self`), and `performance.now` and `Date.now` through the global object, which
 * `no-restricted-properties` cannot see (it matches only a bare object name). A clock
 * reached through an alias is beyond a syntax rule.
 */
const CLOCK_SYNTAX: readonly { readonly selector: string; readonly message: string }[] = [
  { selector: "NewExpression[callee.name='Date'][arguments.length=0], CallExpression[callee.name='Date']", message: CLOCK_MESSAGE },
  { selector: ":matches(NewExpression[arguments.length=0], CallExpression)[callee.property.name='Date'][callee.object.name=/^(globalThis|window|self)$/]", message: CLOCK_MESSAGE },
  { selector: "MemberExpression[property.name='now'][object.property.name=/^(performance|Date)$/][object.object.name=/^(globalThis|window|self)$/]", message: CLOCK_MESSAGE },
];

/** The restricted properties that are not a clock read: what a clock reader keeps refusing. */
const PROPERTIES: readonly { readonly object: string; readonly property: string; readonly message: string }[] = [
  // The global `fetch` reached through its object, for the reason given at `no-restricted-globals`.
  ...["globalThis", "window", "self"].map((object) => ({ object, property: "fetch", message: FETCH_MESSAGE })),
  {
    object: "Promise",
    property: "withResolvers",
    message: "It is missing from browsers the friendly checks still explain (Safari before 17.4, Firefox before 121): use `new Promise`.",
  },
];

/** The clock reads written as properties of a global named directly. */
const CLOCK_PROPERTIES: readonly { readonly object: string; readonly property: string; readonly message: string }[] = [
  { object: "performance", property: "now", message: CLOCK_MESSAGE },
  { object: "Date", property: "now", message: CLOCK_MESSAGE },
  { object: "process", property: "hrtime", message: CLOCK_MESSAGE },
  { object: "process", property: "uptime", message: CLOCK_MESSAGE },
];

/**
 * The two restriction rules, with the clock reads unless `readsClock`: the one source of
 * both the general rules and the exemption of {@link CLOCK_READERS}, which can therefore
 * drop only the clock reads, every other restriction staying.
 */
function restrictions(readsClock: boolean): Linter.RulesRecord {
  return {
    "no-restricted-syntax": ["error", ...SYNTAX, ...(readsClock ? [] : CLOCK_SYNTAX)],
    "no-restricted-properties": ["error", ...PROPERTIES, ...(readsClock ? [] : CLOCK_PROPERTIES)],
  };
}

/**
 * The rules the policy sets itself, on top of the recommended and strict configurations
 * it starts from. Every rule set here, unless it is turned off, is proven by a policy
 * fixture that breaks it (`test/policy.test.ts` fails on a rule no fixture breaks).
 */
export const RULES: Linter.RulesRecord = {
  // No type assertions (`x as T`, `<T>x`): prove types with type guards instead. `as const` stays allowed.
  "@typescript-eslint/consistent-type-assertions": ["error", { assertionStyle: "never" }],
  "@typescript-eslint/no-unsafe-type-assertion": "error",
  "@typescript-eslint/no-non-null-assertion": "error",
  // Rest arguments included (`...args: any[]`), and never fixed silently to `unknown`: the defaults.
  "@typescript-eslint/no-explicit-any": "error",
  "@typescript-eslint/ban-ts-comment": ["error", { "ts-expect-error": true, "ts-ignore": true, "ts-nocheck": true }],
  // Conditions must be real booleans: no truthiness on strings, numbers or nullable values.
  "@typescript-eslint/strict-boolean-expressions": ["error", { allowString: false, allowNumber: false, allowNullableObject: false }],
  "@typescript-eslint/switch-exhaustiveness-check": ["error", { considerDefaultExhaustiveForUnions: false, requireDefaultForNonUnion: true }],
  "@typescript-eslint/explicit-module-boundary-types": "error",
  "@typescript-eslint/prefer-readonly": "error",
  "@typescript-eslint/consistent-type-imports": "error",
  "@typescript-eslint/no-import-type-side-effects": "error",
  // Restricted syntax and properties: what is refused everywhere (an enum, a global
  // augmentation, a focused or skipped test, `fetch` through the global object, and, in
  // code that runs in browsers, APIs newer than what the friendly capability checks
  // explain: the playground names a missing AudioWorklet, not a TypeError; they are banned
  // here one by one until a baseline is declared), and the clock reads.
  ...restrictions(false),
  // Every named item is documented, exported or not, as in Rust (missing_docs and
  // missing_docs_in_private_items): functions, classes, interfaces and their members,
  // types, and module-level constants. Inline callbacks are not items and need none.
  "jsdoc/require-jsdoc": [
    "error",
    {
      publicOnly: false,
      require: { FunctionDeclaration: true, ClassDeclaration: true, MethodDefinition: true },
      checkConstructors: false,
      checkGetters: true,
      checkSetters: true,
      contexts: JSDOC_CONTEXTS,
    },
  ],
  "jsdoc/require-description": "error",
  "jsdoc/require-param": "off", // parameters are typed; describe them in the description when useful
  "jsdoc/require-returns": "off", // return types are typed
  eqeqeq: "error",
  // Names say what they hold (`error`, not `e`), as Clippy's min_ident_chars in Rust:
  // only the conventional loop and coordinate names are shorter.
  "id-length": ["error", { min: 2, exceptions: ["i", "j", "x", "y"], properties: "never" }],
  // A name means one thing in a scope: an inner declaration never hides an outer one.
  "no-shadow": "off",
  "@typescript-eslint/no-shadow": ["error", { hoist: "all" }], // even a name declared further down
  // Parameters are never reassigned nor mutated, except the mutable-by-design objects
  // received to be drawn on or written into: canvas contexts and DOM elements.
  "no-param-reassign": ["error", { props: true, ignorePropertyModificationsFor: ["canvas", "element"] }],
  // A response is read only once its status is checked: `fetch` resolves on a 404 too,
  // whose empty body would then be parsed as JSON or compiled as WebAssembly, an error
  // naming neither the URL nor the status. Each package fetches through one checked helper,
  // in its file named `fetch.ts` (FETCH_HELPERS), tested to throw on a status that is not 2xx.
  "no-restricted-globals": ["error", { name: "fetch", message: FETCH_MESSAGE }],
  "prefer-const": "error",
  "no-var": "error",
};

/**
 * The files allowed to call `fetch`: each package's one helper, which checks the status
 * of every response before returning it (and is tested to). Nowhere else.
 */
export const FETCH_HELPERS: readonly string[] = ["**/fetch.ts"];

/**
 * The files allowed to read a clock, each for its reason; nowhere else (tests are
 * deterministic, CLAUDE.md):
 * - the bench page's timing module, for the wall-time benchmarks of spec/benchmarks.md,
 *   asserted only as upper bounds with a derived margin, in the end-to-end tests;
 * - the roadmap command's entry point, which stamps today's date on what it changes (its
 *   tests give it a fixed date).
 */
export const CLOCK_READERS: readonly string[] = ["**/e2e/site/timing.ts", "**/src/cli/bin.ts"];

/**
 * The shared configuration, for the package in `directory` (its `import.meta.dirname`),
 * ignoring `ignores` (build outputs and reports, relative to it), with the package's
 * `extra` configuration objects appended last.
 */
export function strict(directory: string, ignores: readonly string[], ...extra: readonly Config[]): Config[] {
  return defineConfig(
    { ignores: [...ignores, "coverage", "reports", ".stryker-tmp", "playwright-report", "test-results"] },
    // No inline `eslint-disable` comments: a rule is either respected or changed here, for everyone.
    { linterOptions: { noInlineConfig: true, reportUnusedDisableDirectives: "error" } },
    js.configs.recommended,
    jsdoc.configs["flat/recommended-typescript-error"],
    tseslint.configs.strictTypeChecked,
    tseslint.configs.stylisticTypeChecked,
    {
      languageOptions: {
        parserOptions: { projectService: true, tsconfigRootDir: directory },
      },
      rules: RULES,
    },
    // The checked helper itself calls `fetch`: the only rule of `no-restricted-globals` is that one.
    { files: [...FETCH_HELPERS], rules: { "no-restricted-globals": "off" } },
    // A clock reader may read a clock: every other restriction of the two rules stays.
    { files: [...CLOCK_READERS], rules: restrictions(true) },
    ...extra,
  );
}
