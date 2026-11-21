/** Code under test may run a timer, bare or through a global object: it is no test's sleep. */
export function poll(check: () => void): readonly number[] {
  return [setTimeout(check, 10), globalThis.setInterval(check, 10), window.setTimeout(check, 1), self.setTimeout(check, 1)];
}
