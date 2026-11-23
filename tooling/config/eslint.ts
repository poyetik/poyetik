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

/**
 * Why formatting without a locale is refused: it follows the machine's locale, which a test
 * run does not vary (CLAUDE.md: never test only at the default), so a text that must not
 * change with it would pass every test and change for a user.
 */
const LOCALE_MESSAGE = "Name the locale (CLAUDE.md: never test only at the default): a formatter given none follows the machine's.";

/**
 * The selector of a call given no locale at argument `index`: none, `undefined` or the empty
 * list (`[]`, which falls back to the machine's locale as no locale does), one source for
 * every formatter of {@link LOCALE_SYNTAX}.
 */
function noLocale(index: number): string {
  return `:matches([arguments.length<${String(index + 1)}], [arguments.${String(index)}.type='Identifier'][arguments.${String(index)}.name='undefined'], [arguments.${String(index)}.type='ArrayExpression'][arguments.${String(index)}.elements.length=0])`;
}

/**
 * The formatters that follow the machine's locale when given none, `undefined` or the empty
 * list: the `toLocale…` methods and `localeCompare` of strings, numbers and dates, and the
 * formatters of `Intl`, called or constructed. Not a read of the locale itself
 * (`resolvedOptions()`), which a test emulating a locale asserts first. A locale given
 * through a variable that holds `undefined` is beyond a syntax rule, and so is one read from
 * the environment and passed on (`navigator.language`): a choice of the user's locale the
 * reader sees, whose text is meant to vary with it.
 */
const LOCALE_SYNTAX: readonly { readonly selector: string; readonly message: string }[] = [
  { selector: `CallExpression[callee.property.name=/^toLocale(String|DateString|TimeString|UpperCase|LowerCase)$/]${noLocale(0)}`, message: LOCALE_MESSAGE },
  { selector: `CallExpression[callee.property.name='localeCompare']${noLocale(1)}`, message: LOCALE_MESSAGE },
  {
    selector: `:matches(CallExpression, NewExpression)[callee.object.name='Intl'][callee.property.name=/^(NumberFormat|DateTimeFormat|Collator|PluralRules|RelativeTimeFormat|ListFormat|Segmenter|DisplayNames|DurationFormat)$/]${noLocale(0)}:not(MemberExpression[property.name='resolvedOptions'] > *)`,
    message: LOCALE_MESSAGE,
  },
];

/**
 * The matchers of text, Vitest's and Playwright's, whose argument may be a regular
 * expression: an alternation in it is refused ({@link ALTERNATION_MESSAGE}).
 */
const TEXT_MATCHERS: readonly string[] = ["toMatch", "toHaveText", "toContainText", "toHaveAccessibleName", "toHaveAccessibleDescription", "toHaveValue", "toHaveTitle", "toHaveURL", "toHaveAttribute", "toThrow", "toThrowError"];

/** Why a regular expression with an alternation is refused as a matcher's text. */
const ALTERNATION_MESSAGE = "Assert one text, not either of several (CLAUDE.md: tests prove presence): a matcher given a regular expression with an alternation passes on any branch; key what each case gives in a table and assert it exactly.";

/** Why a sleep is refused. */
const SLEEP_MESSAGE = "Wait for what the test needs (a poll, an event, the next recorded repaint), never for a time (CLAUDE.md: tests are deterministic).";

/** The names of classes, as an attribute of a selector: capitalized, with a lower-case letter. */
const CLASS_NAME = "/^[A-Z][A-Za-z0-9]*[a-z]/";

/** Why a part of an error's text is refused as what a call throws. */
const PARTIAL_MESSAGE =
  "Assert the error itself, `new X(message)`, not a part of its text (CLAUDE.md: an error is asserted whole): a text another program decides is a pattern of RUNTIME_MESSAGES, with why.";

/** The words, as a pattern, of a sine the oracle cannot tell from DC over a case's frames. */
const CONDITIONING = "sine\\.frequency cannot be measured over 1000 frames \\(the components are too close to be told apart \\(conditioning \\d+\\.\\d+, at most 0\\.5\\)\\): lengthen the case$";

/**
 * The patterns of errors whose whole text another program decides, each with why: the only
 * parts of an error's text a test may match.
 */
export const RUNTIME_MESSAGES: readonly (readonly [string, string])[] = [
  ["ENOENT", "Node's words of a file that is missing, with its path"],
  ["EADDRNOTAVAIL", "Node's words of an address this machine has not"],
  ["Command failed", "Node's words of a command that fails, with the command whole"],
  ["^not JSON: SyntaxError", "V8's words of a text that is no JSON, after the prefix"],
  ["^not JSON \\(", "V8's words of a text that is no JSON, after the prefix"],
  ["Error: 0 of 3 readings in 12 tries, the context closed: \\[\\]", "Playwright's words and stack around an error of the page"],
  ["^mixing 2 cables into one inlet at 48 kHz in binary64: the patch without it costs \\d+ instructions more than the patch holding it$", "a count of instructions the engine's build decides"],
  [`^invalid conformance input: expectations\\.json: a: channel 0: ${CONDITIONING}`, "a conditioning the measure computes, in all its digits"],
  [`^invalid conformance input: expectations\\.json: a: channel 1: ${CONDITIONING}`, "a conditioning the measure computes, in all its digits"],
];

/** The attributes that leave out the patterns of {@link RUNTIME_MESSAGES}. */
const REVIEWED = RUNTIME_MESSAGES.map(([pattern]) => `:not([regex.pattern=${JSON.stringify(pattern)}])`).join("");

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
  ...LOCALE_SYNTAX,
  // A test proves presence: a regular expression with an alternation passed to a matcher of
  // text accepts either branch, so that it proves neither (a table of what each case gives,
  // asserted exactly, says which). An escaped bar (`\|`) is a character, not an alternation.
  {
    selector: `CallExpression[callee.property.name=/^(${TEXT_MATCHERS.join("|")})$/] > Literal.arguments[regex.pattern=/(^|[^\\\\])\\|/]`,
    message: ALTERNATION_MESSAGE,
  },
  // A test proves the exact value it knows: an error class alone passes on any message,
  // another refusal of the same class included (the error itself, `new X(message)`, compares
  // its name and its message), and `every(...)` asserted true says nothing of which element
  // failed (the elements failing, asserted empty, name them).
  {
    // A class's name is capitalized with a lower-case letter; a constant's, in capitals
    // alone, may hold the error itself.
    selector: `CallExpression[callee.property.name=/^(toThrow|toThrowError)$/] > :matches(Identifier.arguments[name=${CLASS_NAME}], MemberExpression.arguments[property.name=${CLASS_NAME}])`,
    message: "Assert the error itself, `new X(message)`, not its class alone: a class passes on any message, another refusal of it included (CLAUDE.md: an error is asserted whole).",
  },
  // An error's text given in part, a substring or a regular expression, passes on any error
  // holding that part: the error itself, `new X(message)`, compares it whole. A text another
  // program decides is matched by a pattern reviewed in RUNTIME_MESSAGES.
  {
    selector: [":matches(Literal, TemplateLiteral).arguments", "ConditionalExpression.arguments > :matches(Literal, TemplateLiteral)"].map((given) => `CallExpression[callee.property.name=/^(toThrow|toThrowError)$/] > ${given}${REVIEWED}`).join(", "),
    message: PARTIAL_MESSAGE,
  },
  {
    // Anywhere in what `expect` is given (an `every` joined to another, or on an optional
    // chain), by any matcher of true.
    selector:
      "CallExpression:matches([callee.property.name=/^(toBe|toEqual|toStrictEqual)$/][arguments.0.value=true], [callee.property.name='toBeTruthy']) > MemberExpression.callee > CallExpression.object[callee.name='expect'] CallExpression[callee.property.name='every']",
    message: "Assert the elements that fail, `expect(xs.filter((x) => !ok(x))).toStrictEqual([])`, not `every` as true: its failure names none of them (CLAUDE.md: a kept state is compared by its values).",
  },
  // A rejection's reason, dropped by a callback that takes none, is lost without a word.
  {
    selector: "CallExpression[callee.property.name='catch'][arguments.0.type=/^(ArrowFunctionExpression|FunctionExpression)$/][arguments.0.params.length=0]",
    message: "Name the reason a rejection carries: say where it goes (a log, a typed result, an error's cause), or drop it by a name starting with `_` and a comment saying why.",
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

/** Why reading an audio context's clock is refused outside {@link AUDIO_CLOCK_READERS}. */
const AUDIO_CLOCK_MESSAGE =
  "Read an audio context's clock through the package's checked reader (its audio-clock.ts): a browser may report one that is no time, which every AudioParam refuses.";

/**
 * The reads of an audio context's clock (`currentTime`): a property read, plain or optional,
 * and a destructuring (a read by its name as a string is `dot-notation`'s). Not a write (a
 * test's fake sets its clock), nor a class's read of its own field (`this.currentTime`, a fake's).
 */
const AUDIO_CLOCK_SYNTAX: readonly { readonly selector: string; readonly message: string }[] = [
  {
    selector: "MemberExpression[property.name='currentTime'][object.type!='ThisExpression']:not(AssignmentExpression > MemberExpression.left)",
    message: AUDIO_CLOCK_MESSAGE,
  },
  { selector: "ObjectPattern > Property[key.name='currentTime']", message: AUDIO_CLOCK_MESSAGE },
];

/**
 * Why calling a function by `Reflect.apply` is refused in a module driver's code
 * ({@link MODULE_DRIVERS}) outside its boundary helper.
 */
const EXPORT_CALL_MESSAGE =
  "Call a module's export only through the driver's boundary helper (a function named `exported…`), which checks it and runs its call through the driver's failure handling: a call made otherwise escapes the guard the driver documents.";

/**
 * A call by `Reflect.apply` (how a driver calls a module's export it read as unknown)
 * anywhere but inside a function declaration named `exported…`: the one place each driver
 * checks an export and hands out its call, guarded.
 */
const EXPORT_CALL_SYNTAX: readonly { readonly selector: string; readonly message: string }[] = [
  {
    selector: "CallExpression[callee.object.name='Reflect'][callee.property.name='apply']:not(FunctionDeclaration[id.name=/^exported/] *)",
    message: EXPORT_CALL_MESSAGE,
  },
];

/** The restricted properties that are not a clock read: what a clock reader keeps refusing. */
const PROPERTIES: readonly { readonly object?: string; readonly property: string; readonly message: string }[] = [
  // Playwright's sleep, on a page or a frame: whatever object holds it.
  { property: "waitForTimeout", message: SLEEP_MESSAGE },
  // Sleeps no timer names: a signal that times out, a thread that blocks.
  { object: "AbortSignal", property: "timeout", message: SLEEP_MESSAGE },
  { object: "Atomics", property: "wait", message: SLEEP_MESSAGE },
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
 * The two restriction rules, with the clock reads unless `readsClock`, the reads of an
 * audio clock unless `readsAudioClock`, and the calls of a module's export outside its
 * boundary helper when `drivesModule`: the one source of the general rules, of the
 * exemptions of {@link CLOCK_READERS} and {@link AUDIO_CLOCK_READERS}, which can therefore
 * drop only their own reads, and of the drivers' rule ({@link MODULE_DRIVERS}), which adds
 * only its own, every other restriction staying.
 */
function restrictions(readsClock: boolean, readsAudioClock: boolean, drivesModule: boolean): Linter.RulesRecord {
  return {
    "no-restricted-syntax": ["error", ...SYNTAX, ...(readsClock ? [] : CLOCK_SYNTAX), ...(readsAudioClock ? [] : AUDIO_CLOCK_SYNTAX), ...(drivesModule ? EXPORT_CALL_SYNTAX : [])],
    "no-restricted-properties": ["error", ...PROPERTIES, ...(readsClock ? [] : CLOCK_PROPERTIES)],
  };
}

/**
 * The timers a sleep is made of (`await new Promise((resolve) => { setTimeout(resolve, ms); })`),
 * refused in test code ({@link TEST_FILES}) but in {@link TIMED_WAITS}: a test waits for what
 * it needs, never for a time. The code under test may run a timer (a poll with a bound).
 */
const SLEEP_GLOBALS: readonly { readonly name: string; readonly message: string }[] = ["setTimeout", "setInterval"].map((name) => ({ name, message: SLEEP_MESSAGE }));

/**
 * The modules of Node's timers, by each of their names, whose import binds a timer no rule of
 * globals sees, refused in test code ({@link TEST_FILES}) but in {@link TIMED_WAITS}. A
 * module imported by a computed or dynamic import is beyond this rule.
 */
const SLEEP_IMPORTS: readonly { readonly name: string; readonly message: string }[] = ["node:timers", "node:timers/promises", "timers", "timers/promises"].map((name) => ({ name, message: SLEEP_MESSAGE }));

/** What a file may call of the globals {@link globals} refuses. */
interface Allowed {
  /** `fetch`: a fetch helper ({@link FETCH_HELPERS}). */
  readonly fetches: boolean;
  /** The timers of a sleep: code that is no test, or a timed wait ({@link TIMED_WAITS}). */
  readonly sleeps: boolean;
}

/**
 * The rule refusing globals, bare or through the global object (`globalThis`, `window`,
 * `self`, declared as globals by {@link strict} so that the rule sees them): `fetch` unless
 * `fetches`, the timers of a sleep unless `sleeps`. The one source of every
 * entry, so that an exemption drops only its own globals.
 */
function globals({ fetches, sleeps }: Allowed): Linter.RuleEntry {
  return ["error", { globals: [...(fetches ? [] : [{ name: "fetch", message: FETCH_MESSAGE }]), ...(sleeps ? [] : SLEEP_GLOBALS)], checkGlobalObject: true }];
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
  // augmentation, a focused or skipped test, a formatter given no locale, an alternation in
  // a matcher's text, Playwright's sleep, and, in
  // code that runs in browsers, APIs newer than what the friendly capability checks
  // explain: the playground names a missing AudioWorklet, not a TypeError; they are banned
  // here one by one until a baseline is declared), the clock reads, and the reads of an
  // audio context's clock.
  ...restrictions(false, false, false),
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
  // Bare or through the global object; in test code, the timers of a sleep too (TEST_FILES).
  "no-restricted-globals": globals({ fetches: false, sleeps: true }),
  "prefer-const": "error",
  "no-var": "error",
  // Every unused parameter is reported, a leading one too (`args: "all"`), unless its name
  // says it is dropped on purpose (`_`, with a comment saying why): a rejection's reason
  // above all, which a `.catch` callback must name (the restricted syntax above).
  "@typescript-eslint/no-unused-vars": ["error", { args: "all", argsIgnorePattern: "^_" }],
};

/**
 * The files allowed to call `fetch`: each package's one helper, which checks the status
 * of every response before returning it (and is tested to). Nowhere else.
 */
export const FETCH_HELPERS: readonly string[] = ["**/fetch.ts"];

/**
 * The test code of a package, whatever its framework: its unit tests and end-to-end tests,
 * with their helpers, fixtures and pages, and the helpers they share (`testing/`), where the
 * timers of a sleep ({@link SLEEP_GLOBALS}) and their modules ({@link SLEEP_IMPORTS}) are
 * refused.
 */
export const TEST_FILES: readonly string[] = ["**/test/**", "**/testing/**", "**/e2e/**", "**/*.test.*", "**/*.spec.*"];

/**
 * The files of test code allowed a timer, each reviewed for its reason; nowhere else in
 * test code:
 * - the playground's readings of the audio clock in the end-to-end tests (`clockSamples`),
 *   spaced by the page's own timers: what it measures is how the browser reports the clock
 *   between two readings apart in time, which no event marks, and every reading is bounded
 *   (it tries a fixed number of times, then fails naming what it read).
 */
export const TIMED_WAITS: readonly string[] = ["**/e2e/audio-clock.ts"];

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
 * The files allowed to read an audio context's clock (`currentTime`): each package's one
 * checked reader, which returns no time where the browser reports a non-finite clock, so
 * that whoever schedules on it handles that absence (the playground's `audioTime`). Nowhere
 * else in the package.
 */
export const AUDIO_CLOCK_READERS: readonly string[] = ["**/audio-clock.ts"];

/**
 * The code of the packages that drive a WebAssembly module (each binding's `ts/`, what it
 * ships): a call into the module by `Reflect.apply` stays inside the driver's boundary
 * helper (`exported…`), which checks the export and runs the call through the driver's
 * guard, so that no call escapes the failure handling a driver documents (a `VisualTrap`,
 * the instance never called again). A driver of another package's own may not call around
 * it either.
 */
export const MODULE_DRIVERS: readonly string[] = ["ts/**/*.ts"];

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
        // The global objects, so that `no-restricted-globals` sees a global reached through them.
        globals: { window: "readonly", self: "readonly" },
      },
      rules: RULES,
    },
    // Test code refuses the timers of a sleep too, and their modules.
    { files: [...TEST_FILES], rules: { "no-restricted-globals": globals({ fetches: false, sleeps: false }) } },
    { files: [...TEST_FILES], ignores: [...TIMED_WAITS], rules: { "no-restricted-imports": ["error", { paths: SLEEP_IMPORTS }] } },
    // The checked helper itself calls `fetch`; one in test code still refuses a sleep (the entry after).
    { files: [...FETCH_HELPERS], rules: { "no-restricted-globals": globals({ fetches: true, sleeps: true }) } },
    // Each inner array is an intersection: a file that matches both the helper and the tests.
    { files: FETCH_HELPERS.flatMap((helper) => TEST_FILES.map((tests) => [helper, tests])), rules: { "no-restricted-globals": globals({ fetches: true, sleeps: false }) } },
    // A reviewed timed wait may run a timer: `fetch` stays refused.
    { files: [...TIMED_WAITS], rules: { "no-restricted-globals": globals({ fetches: false, sleeps: true }) } },
    // A clock reader may read a clock: every other restriction of the two rules stays.
    { files: [...CLOCK_READERS], rules: restrictions(true, false, false) },
    // An audio clock reader may read an audio clock: every other restriction stays.
    { files: [...AUDIO_CLOCK_READERS], rules: restrictions(false, true, false) },
    // A module driver's code calls a module's export only through its boundary helper.
    { files: [...MODULE_DRIVERS], rules: restrictions(false, false, true) },
    ...extra,
  );
}
