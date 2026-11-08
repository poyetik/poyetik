// The strict linting policy shared by every TypeScript package (see CLAUDE.md): no escape
// hatch can weaken the type system. Each package's eslint.config.js calls `strict`.
import js from "@eslint/js";
import { defineConfig, type Config } from "eslint/config";
import jsdoc from "eslint-plugin-jsdoc";
import tseslint from "typescript-eslint";

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
      rules: {
        // No type assertions (`x as T`, `<T>x`): prove types with type guards instead. `as const` stays allowed.
        "@typescript-eslint/consistent-type-assertions": ["error", { assertionStyle: "never" }],
        "@typescript-eslint/no-unsafe-type-assertion": "error",
        "@typescript-eslint/no-non-null-assertion": "error",
        // Rest arguments included (`...args: any[]`), and never fixed silently to `unknown`: the defaults.
        "@typescript-eslint/no-explicit-any": "error",
        "@typescript-eslint/ban-ts-comment": [
          "error",
          { "ts-expect-error": true, "ts-ignore": true, "ts-nocheck": true },
        ],
        // Conditions must be real booleans: no truthiness on strings, numbers or nullable values.
        "@typescript-eslint/strict-boolean-expressions": [
          "error",
          { allowString: false, allowNumber: false, allowNullableObject: false },
        ],
        "@typescript-eslint/switch-exhaustiveness-check": [
          "error",
          { considerDefaultExhaustiveForUnions: false, requireDefaultForNonUnion: true },
        ],
        "@typescript-eslint/explicit-module-boundary-types": "error",
        "@typescript-eslint/prefer-readonly": "error",
        "@typescript-eslint/consistent-type-imports": "error",
        "@typescript-eslint/no-import-type-side-effects": "error",
        "no-restricted-syntax": [
          "error",
          { selector: "TSEnumDeclaration", message: "Use a union of string literals or an `as const` object." },
          { selector: "TSNamespaceExportDeclaration, TSModuleDeclaration[global=true]", message: "No global augmentation." },
        ],
        // Every named item is documented, exported or not, as in Rust (missing_docs and
        // missing_docs_in_private_items): functions, classes and their members, types, and
        // module-level constants. Inline callbacks are not items and need none.
        "jsdoc/require-jsdoc": [
          "error",
          {
            publicOnly: false,
            require: { FunctionDeclaration: true, ClassDeclaration: true, MethodDefinition: true },
            checkConstructors: false,
            checkGetters: true,
            checkSetters: true,
            contexts: [
              "TSInterfaceDeclaration",
              "TSTypeAliasDeclaration",
              "PropertyDefinition",
              "Program > VariableDeclaration",
              "ExportNamedDeclaration[declaration.type='VariableDeclaration']",
            ],
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
        "prefer-const": "error",
        "no-var": "error",
      },
    },
    ...extra,
  );
}
