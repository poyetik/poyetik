/**
 * Roadmap model: data + pure commands.
 * Every change goes through a command that returns a new board (never mutates).
 */

export const COLUMNS = [
  { id: "later", title: "Later" },
  { id: "next", title: "Next" },
  { id: "doing", title: "In progress" },
  { id: "done", title: "Done" },
] as const;

export type ColumnId = (typeof COLUMNS)[number]["id"];

export interface Card {
  readonly id: string;
  readonly title: string;
  readonly description: string;
  readonly column: ColumnId;
  readonly milestone: string;
  readonly createdAt: string;
  readonly updatedAt: string;
}

export interface Board {
  readonly version: 1;
  readonly title: string;
  readonly nextId: number;
  readonly cards: readonly Card[];
}

export class BoardError extends Error {
  override readonly name = "BoardError";
}

const DATE_RE = /^\d{4}-\d{2}-\d{2}$/;
const ID_RE = /^R-\d+$/;
const CARD_KEYS = ["id", "title", "description", "column", "milestone", "createdAt", "updatedAt"];
const BOARD_KEYS = ["version", "title", "nextId", "cards"];

export function isColumnId(value: unknown): value is ColumnId {
  return COLUMNS.some((c) => c.id === value);
}

export function emptyBoard(title: string): Board {
  return { version: 1, title: requireText(title, "title"), nextId: 1, cards: [] };
}

// ---------- Validation ----------

function requireText(value: unknown, field: string): string {
  if (typeof value !== "string" || value.trim() === "") {
    throw new BoardError(`${field} must be a non-empty string`);
  }
  return value.trim();
}

function requireString(value: unknown, field: string): string {
  if (typeof value !== "string") {
    throw new BoardError(`${field} must be a string`);
  }
  return value.trim();
}

function requireDate(value: unknown, field: string): string {
  if (typeof value !== "string" || !DATE_RE.test(value)) {
    throw new BoardError(`${field} must be a YYYY-MM-DD date`);
  }
  return value;
}

function requireColumn(value: unknown): ColumnId {
  if (!isColumnId(value)) {
    throw new BoardError(`unknown column: ${String(value)} (expected: ${COLUMNS.map((c) => c.id).join(", ")})`);
  }
  return value;
}

function requireExactKeys(value: object, keys: readonly string[], what: string): void {
  const actual = Object.keys(value).sort();
  const expected = [...keys].sort();
  if (actual.join(",") !== expected.join(",")) {
    throw new BoardError(`${what}: expected fields ${expected.join(", ")}, got ${actual.join(", ")}`);
  }
}

function isPositiveInteger(value: unknown): value is number {
  return Number.isInteger(value) && Number(value) >= 1;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function parseCard(raw: unknown, index: number): Card {
  const where = `card #${String(index)}`;
  if (!isRecord(raw)) {
    throw new BoardError(`${where} must be an object`);
  }
  requireExactKeys(raw, CARD_KEYS, where);
  const id = raw.id;
  if (typeof id !== "string" || !ID_RE.test(id)) {
    throw new BoardError(`${where}: invalid id`);
  }
  return {
    id,
    title: requireText(raw.title, `${where} title`),
    description: requireString(raw.description, `${where} description`),
    column: requireColumn(raw.column),
    milestone: requireString(raw.milestone, `${where} milestone`),
    createdAt: requireDate(raw.createdAt, `${where} createdAt`),
    updatedAt: requireDate(raw.updatedAt, `${where} updatedAt`),
  };
}

/** Validates an unknown board (JSON read from disk) and returns it typed. */
export function parseBoard(raw: unknown): Board {
  if (!isRecord(raw)) {
    throw new BoardError("board must be an object");
  }
  requireExactKeys(raw, BOARD_KEYS, "board");
  if (raw.version !== 1) {
    throw new BoardError(`unsupported version: ${String(raw.version)}`);
  }
  const nextId = raw.nextId;
  if (!isPositiveInteger(nextId)) {
    throw new BoardError("nextId must be an integer >= 1");
  }
  const cards = raw.cards;
  if (!Array.isArray(cards)) {
    throw new BoardError("cards must be an array");
  }
  const parsed = cards.map((c, i) => parseCard(c, i));
  const seen = new Set<string>();
  for (const card of parsed) {
    if (seen.has(card.id)) {
      throw new BoardError(`duplicate id: ${card.id}`);
    }
    seen.add(card.id);
    if (Number(card.id.slice(2)) >= nextId) {
      throw new BoardError(`${card.id} must be lower than nextId (${String(nextId)})`);
    }
  }
  return { version: 1, title: requireText(raw.title, "title"), nextId, cards: parsed };
}

/** Stable serialization (fixed key order, trailing newline) for clean git diffs. */
export function serializeBoard(board: Board): string {
  const ordered = {
    version: board.version,
    title: board.title,
    nextId: board.nextId,
    cards: board.cards.map((c) => ({
      id: c.id,
      title: c.title,
      description: c.description,
      column: c.column,
      milestone: c.milestone,
      createdAt: c.createdAt,
      updatedAt: c.updatedAt,
    })),
  };
  return `${JSON.stringify(ordered, null, 2)}\n`;
}

// ---------- Commands ----------

export interface AddInput {
  readonly title: string;
  readonly description?: string | undefined;
  readonly column?: ColumnId | undefined;
  readonly milestone?: string | undefined;
}

export interface EditInput {
  readonly title?: string | undefined;
  readonly description?: string | undefined;
  readonly milestone?: string | undefined;
}

function findCard(board: Board, id: string): Card {
  const card = board.cards.find((c) => c.id === id);
  if (card === undefined) {
    throw new BoardError(`card not found: ${id}`);
  }
  return card;
}

export function addCard(board: Board, input: AddInput, today: string): { board: Board; card: Card } {
  const date = requireDate(today, "today's date");
  const card: Card = {
    id: `R-${String(board.nextId)}`,
    title: requireText(input.title, "title"),
    description: requireString(input.description ?? "", "description"),
    column: requireColumn(input.column ?? "later"),
    milestone: requireString(input.milestone ?? "", "milestone"),
    createdAt: date,
    updatedAt: date,
  };
  return { board: { ...board, nextId: board.nextId + 1, cards: [...board.cards, card] }, card };
}

/** Moves a card to a column, at a given position among that column's cards (end by default). */
export function moveCard(board: Board, id: string, column: ColumnId, position: number | undefined, today: string): Board {
  const date = requireDate(today, "today's date");
  const target = requireColumn(column);
  const card: Card = { ...findCard(board, id), column: target, updatedAt: date };
  const others = board.cards.filter((c) => c.id !== id);
  const inTarget = others.filter((c) => c.column === target);
  const pos = position ?? inTarget.length;
  if (!Number.isInteger(pos) || pos < 0 || pos > inTarget.length) {
    throw new BoardError(`invalid position: ${String(pos)} (0 to ${String(inTarget.length)})`);
  }
  const anchor = inTarget[pos];
  const insertAt = anchor === undefined ? others.length : others.indexOf(anchor);
  return { ...board, cards: [...others.slice(0, insertAt), card, ...others.slice(insertAt)] };
}

export function editCard(board: Board, id: string, patch: EditInput, today: string): Board {
  const date = requireDate(today, "today's date");
  const current = findCard(board, id);
  if (patch.title === undefined && patch.description === undefined && patch.milestone === undefined) {
    throw new BoardError("nothing to edit");
  }
  const card: Card = {
    ...current,
    title: patch.title === undefined ? current.title : requireText(patch.title, "title"),
    description: patch.description === undefined ? current.description : requireString(patch.description, "description"),
    milestone: patch.milestone === undefined ? current.milestone : requireString(patch.milestone, "milestone"),
    updatedAt: date,
  };
  return { ...board, cards: board.cards.map((c) => (c.id === id ? card : c)) };
}

export function removeCard(board: Board, id: string): Board {
  findCard(board, id);
  return { ...board, cards: board.cards.filter((c) => c.id !== id) };
}
