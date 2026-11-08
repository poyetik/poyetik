/** A fixture every rule accepts: the policy does not reject sound code. */
export const LIMIT = 3;

/** Whether `count` is within {@link LIMIT}. */
export function within(count: number): boolean {
  return count <= LIMIT;
}

/** Draws on the canvas it receives: mutating a canvas context is allowed. */
export function paint(canvas: { fillStyle: string }): void {
  canvas.fillStyle = "red";
}

/** Writes into the element it receives: mutating a DOM element is allowed. */
export function label(element: { textContent: string }): void {
  element.textContent = "ready";
}

/** A class whose constructor needs no comment of its own: the class says what it builds. */
export class Box {
  /** The content of the box. */
  readonly content: number;

  constructor(content: number) {
    this.content = content;
  }
}
