// The strict linting policy shared by every TypeScript package (see CLAUDE.md): no escape
// hatch can weaken the type system. Each package's eslint.config.js calls `strict`.
import js from "@eslint/js";
import { defineConfig } from "eslint/config";
import jsdoc from "eslint-plugin-jsdoc";
import tseslint from "typescript-eslint";

/**
 * The shared configuration, for the package in `directory`, ignoring `ignores` (build
 * outputs and reports), with `extra` configuration objects appended.
 *
 * @param {string} directory The package directory (`import.meta.dirname`).
 * @param {string[]} ignores Paths the package does not lint.
 * @param {...object} extra Package-specific configuration, appended last.
 */
export function strict(directory, ignores, ...extra) {
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
        "@typescript-eslint/no-explicit-any": ["error", { fixToUnknown: false, ignoreRestArgs: false }],
        "@typescript-eslint/ban-ts-comment": [
          "error",
          { "ts-expect-error": true, "ts-ignore": true, "ts-nocheck": true, "ts-check": false },
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
        // Every exported declaration is documented.
        "jsdoc/require-jsdoc": [
          "error",
          {
            publicOnly: true,
            require: { FunctionDeclaration: true, ClassDeclaration: true, ArrowFunctionExpression: true },
            contexts: ["TSInterfaceDeclaration", "TSTypeAliasDeclaration", "ExportNamedDeclaration > VariableDeclaration"],
          },
        ],
        "jsdoc/require-description": "error",
        "jsdoc/require-param": "off", // parameters are typed; describe them in the description when useful
        "jsdoc/require-returns": "off", // return types are typed
        eqeqeq: "error",
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
