/** A driver calling a module's export around its boundary helper: refused. */
export function callAround(value: () => unknown): unknown {
  return Reflect.apply(value, undefined, []);
}
