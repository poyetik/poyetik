import { existsSync } from "node:fs";
import path from "node:path";

/** The root of the repository, found by walking up: the tests also run from a copy of the package, deeper in it. */
export function repository(): string {
  for (let directory = import.meta.dirname; directory !== path.dirname(directory); directory = path.dirname(directory)) {
    if (existsSync(path.join(directory, "pnpm-workspace.yaml"))) {
      return directory;
    }
  }
  throw new Error("repository root not found");
}
