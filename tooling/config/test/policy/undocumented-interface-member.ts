/** Doc. */
export interface Store {
  readonly read: () => string;
  write(text: string): void;
}
