/** Doc. */
export class Counter {
  /** Set once, in the constructor, and never again: it must be `readonly`. */
  #start: number;

  constructor(start: number) {
    this.#start = start;
  }

  /** Doc. */
  get start(): number {
    return this.#start;
  }
}
