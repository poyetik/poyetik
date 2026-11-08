/** The most recent samples of a signal, in a fixed-size ring: nothing allocated per render quantum. */
export class SampleHistory {
  /** The ring of samples. */
  readonly #ring: Float32Array;
  /** Where the next sample goes. */
  #next = 0;
  /** Number of samples received, capped at the ring size. */
  #filled = 0;

  /** Keeps the last `size` samples. */
  constructor(size: number) {
    this.#ring = new Float32Array(size);
  }

  /** Appends samples, dropping the oldest ones beyond the ring size. */
  push(samples: Float32Array): void {
    for (const sample of samples.subarray(Math.max(0, samples.length - this.#ring.length))) {
      this.#ring[this.#next] = sample;
      this.#next = (this.#next + 1) % this.#ring.length;
    }
    this.#filled = Math.min(this.#ring.length, this.#filled + samples.length);
  }

  /** The samples received, oldest first (at most the ring size). */
  snapshot(): Float32Array {
    const out = new Float32Array(this.#filled);
    const start = (this.#next - this.#filled + this.#ring.length) % this.#ring.length;
    // The oldest samples run from `start` to the end of the ring, then wrap to its beginning.
    const head = this.#ring.subarray(start, start + this.#filled);
    out.set(head);
    out.set(this.#ring.subarray(0, this.#filled - head.length), head.length);
    return out;
  }
}
