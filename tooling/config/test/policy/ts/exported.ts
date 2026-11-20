/** A driver's boundary helper, the one place its code calls a module's export. */
export function exported(value: () => unknown): () => unknown {
  return () => Reflect.apply(value, undefined, []);
}
