# roadmap

The public roadmap of the project: a kanban board whose data lives in `board.json` (versioned), edited only through its command-line tool, and shown by a static Astro page with no JavaScript.

| Path | What |
|---|---|
| `board.json` | The board: cards, their columns, milestones and ordered steps |
| `src/lib/board.ts` | The board model: parsing (every field proven at the boundary) and every edit, as pure functions |
| `src/lib/view.ts` | The board prepared for display: cards grouped by column, step statuses, steps grouped by review block (the column order on screen is described there) |
| `src/cli/` | The command-line tool (`pnpm roadmap`), the only way to change `board.json` |
| `src/components/Board.astro`, `src/pages/index.astro` | The page, under a strict Content Security Policy (`src/lib/csp.ts`) |
| `e2e/` | End-to-end tests, with axe, on the browsers and devices of `playwright.config.ts` |

```sh
pnpm roadmap help   # every command: list, show, add, move, edit, rm, title, step …, validate
pnpm dev            # http://localhost:4321
pnpm check          # types, lint, unit tests at 100% coverage, build, end-to-end tests
```

In a card of the `doing` column, the first step not done of the lowest review block still open is shown "In progress", or, when every block is done, the first not done in no block (`currentStepIndex`); elsewhere no step is. A step may belong to a review block (`pnpm roadmap step block`): the page groups a card's steps by block, and a block, ended by a deep review, its decided fixes and a pull request whose full check is green, is done when all its steps are. `pnpm roadmap validate` reports the block numbers below a card's highest that no step uses, and every `pnpm roadmap step` command prints them after the card it changed, each with the command that shows that card.
