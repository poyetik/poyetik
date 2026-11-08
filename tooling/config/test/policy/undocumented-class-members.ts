/** Doc. */
export class Counter {
  count = 0;
  get double(): number {
    return this.count * 2;
  }
  set double(value: number) {
    this.count = value / 2;
  }
  increment(): void {
    this.count += 1;
  }
}
