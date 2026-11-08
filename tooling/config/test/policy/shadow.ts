/** Doc. */
export function run(values: readonly number[]): number {
  // Hides the constant declared below: shadowing is refused even before the declaration.
  return values.reduce((total, VALUE) => total + VALUE, 0);
}

/** Doc. */
export const VALUE = 1;
