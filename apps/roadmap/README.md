# roadmap

The public roadmap of the project: a kanban board whose data lives in `board.json` (versioned), edited only through its command-line tool, and shown by a static Astro page with no JavaScript.

| Path | What |
|---|---|
| `board.json` | The board: cards, their columns, milestones and ordered steps |
| `src/lib/board.ts` | The board model: parsing (every field proven at the boundary) and every edit, as pure functions |
| `src/lib/view.ts` | The board prepared for display: cards grouped by column, step statuses (the column order on screen is described there) |
| `src/cli/` | The command-line tool (`pnpm roadmap`), the only way to change `board.json` |
| `src/components/Board.astro`, `src/pages/index.astro` | The page, under a strict Content Security Policy (`src/lib/csp.ts`) |
| `e2e/` | End-to-end tests, with axe, on the browsers and devices of `playwright.config.ts` |

```sh
pnpm roadmap help   # every command: list, show, add, move, edit, rm, title, step …, validate
pnpm dev            # http://localhost:4321
pnpm check          # types, lint, unit tests at 100% coverage, mutation testing, build, end-to-end tests
```

In a card of the `doing` column, the first step not done is shown "In progress"; elsewhere no step is.
