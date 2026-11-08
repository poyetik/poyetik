// The worklet module, imported by the tests through an alias (see vitest.config.ts): it is
// type-checked by its own project (ts/worklet), with the worklet globals. Declared empty:
// it is only imported for its side effects, and a named import from it fails to compile
// instead of being typed `any`.
declare module "#worklet/processors" {}
