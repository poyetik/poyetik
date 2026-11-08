/** Doc. */
export function f(): void {
  try {
    f();
  } catch (error) {
    error.message;
  }
}
