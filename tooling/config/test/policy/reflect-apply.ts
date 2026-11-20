/** Outside a module driver's code (a test's, a tool's), a call by `Reflect.apply` is no export call to guard. */
export function apply(value: () => unknown): unknown {
  return Reflect.apply(value, undefined, []);
}
