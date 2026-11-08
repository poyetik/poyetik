import { experimental_AstroContainer as AstroContainer } from "astro/container";
import { readFileSync } from "node:fs";
import { expect, test } from "vitest";
import IndexPage from "../src/pages/index.astro";
import { parseBoard } from "../src/lib/board.ts";
import { CSP } from "../src/lib/csp.ts";
import { toView } from "../src/lib/view.ts";

test("renders the board of board.json, with its title, policy and icon", async () => {
  const container = await AstroContainer.create();
  const html = await container.renderToString(IndexPage);
  const view = toView(parseBoard(JSON.parse(readFileSync(new URL("../board.json", import.meta.url), "utf8"))));
  expect(html).toContain(`<title>${view.title} · Roadmap</title>`);
  expect(html).toContain(`<meta http-equiv="Content-Security-Policy" content="${CSP}">`);
  expect(html).toContain('<link rel="icon" href="/favicon.svg" type="image/svg+xml">');
  expect(html).toContain(`<h1>${view.title}</h1>`);
  for (const column of view.columns) {
    expect(html).toContain(`data-column="${column.id}"`);
  }
  expect(html).not.toContain("<script");
});
