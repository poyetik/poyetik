// The `ImageData` jsdom lacks, which the setup gives every test.

/** An `ImageData` for jsdom, which has none: the bytes it was given, and its size. */
export class FakeImageData {
  /** The pixels. */
  readonly data: Uint8ClampedArray;
  /** Width, in pixels. */
  readonly width: number;
  /** Height, in pixels. */
  readonly height: number;

  /** Holds `data` as an `ImageData` of `width` × `height`. */
  constructor(data: Uint8ClampedArray, width: number, height: number) {
    this.data = data;
    this.width = width;
    this.height = height;
  }
}
