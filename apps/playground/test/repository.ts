// Files of the repository outside the package, read by the tests.
import { existsSync } from "node:fs";
import { pathToFileURL } from "node:url";

/**
 * The file at `path` (relative to the repository root), found by walking up from the
 * package directory: under jsdom, module URLs are not file URLs, and the tests also run
 * from a copy of the package (the mutation testing sandbox), deeper in the repository.
 * @throws {Error} when no directory above holds `path`.
 */
export function findUp(path: string): URL {
  for (let directory = pathToFileURL(`${process.cwd()}/`); directory.pathname !== "/"; directory = new URL("../", directory)) {
    const file = new URL(path, directory);
    if (existsSync(file)) {
      return file;
    }
  }
  throw new Error(`${path} not found above ${process.cwd()}`);
}
