import { expect, test } from "vitest";
import { citations, uncited, unmatched } from "../gates.ts";

// Finding K6-10 of the sixth deep review of visual block 2 (#34): both gates were whole in a test
// file, with no fixture they refuse.
test("refuses a citation in any form but a title of the rules", () => {
  const rules = "- **Tests prove presence.** Why.\n- **Never skip a test**:\n  - **Nested** \n";
  const cited = citations("Say it (CLAUDE.md: tests prove presence): why (it). (CLAUDE.md: never skip a test) (CLAUDE.md, testing policy). See CLAUDE.md");
  expect(cited).toStrictEqual(["(CLAUDE.md: tests prove presence)", "(CLAUDE.md: never skip a test)", "(CLAUDE.md, testing policy)", "CLAUDE.md"]);
  expect(uncited(cited, rules)).toStrictEqual(["(CLAUDE.md, testing policy)", "CLAUDE.md"]);
  expect(uncited(["(CLAUDE.md: nested)", "(CLAUDE.md: tests)"], rules)).toStrictEqual(["(CLAUDE.md: nested)", "(CLAUDE.md: tests)"]);
});

test("refuses a pattern no test gives toThrow as its whole argument", () => {
  const tests = [
    "expect(f).toThrow(/a/);\nawait expect(g).rejects.toThrow(\n  /^b \\(\\//u,\n);\n",
    "expect(f).toThrow(/long c/); // toThrow(/d/)\nconst text = '.toThrow(/e/)';\nexpect(f).toMatch(/f/); toThrow(/g/); expect(f).toThrow();\nexpect(f).toThrow(new Error('h'));\n",
  ];
  expect(unmatched(["a", "^b \\(\\/", "long c"], tests)).toStrictEqual([]);
  expect(unmatched(["", "c", "d", "e", "f", "g", "h", "a"], tests)).toStrictEqual(["", "c", "d", "e", "f", "g", "h"]);
  expect(unmatched(["a"], [])).toStrictEqual(["a"]);
});
