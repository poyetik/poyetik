/**
 * Roadmap model: data + pure commands.
 * Every change goes through a command that returns a new board (never mutates).
 */

/** The four roadmap columns, in display order. */
export const COLUMNS = [
  { id: "later", title: "Later" },
  { id: "next", title: "Next" },
  { id: "doing", title: "In progress" },
  { id: "done", title: "Done" },
] as const;

/** Identifier of a column. */
export type ColumnId = (typeof COLUMNS)[number]["id"];

/** One step of a card. In a card of the `doing` column, the first step that is not done is the one in progress. */
export interface Task {
  readonly title: string;
  readonly done: boolean;
}

/** Display status of a step: done, in progress (the first step not done, only in the `doing` column) or still to do. */
export type TaskStatus = "done" | "doing" | "todo";

/** A roadmap card (ticket) with its ordered steps. */
export interface Card {
  readonly id: number;
  readonly title: string;
  readonly description: string;
  readonly column: ColumnId;
  readonly milestone: string;
  readonly createdAt: string;
  readonly updatedAt: string;
  readonly tasks: readonly Task[];
}

/** The whole roadmap, as stored in `board.json`. */
export interface Board {
  readonly version: 1;
  readonly title: string;
  readonly nextId: number;
  readonly cards: readonly Card[];
}

/** An expected, user-facing error: invalid data or an invalid command. */
export class BoardError extends Error {
  override readonly name = "BoardError";
}

const DATE_RE = /^\d{4}-\d{2}-\d{2}$/;
const ID_TEXT_RE = /^#?([1-9]\d*)$/;
const CARD_KEYS = ["id", "title", "description", "column", "milestone", "createdAt", "updatedAt", "tasks"];
const TASK_KEYS = ["title", "done"];
const TASK_NUMBER_RE = /^[1-9]\d*$/;
const BOARD_KEYS = ["version", "title", "nextId", "cards"];

/** Whether a value is a known column identifier. */
export function isColumnId(value: unknown): value is ColumnId {
  return COLUMNS.some((c) => c.id === value);
}

/** Formats a card id for display: `#12`. */
export function formatId(id: number): string {
  return `#${String(id)}`;
}

/** Parses a card id typed by a user: `12` or `#12`. */
export function parseId(text: string): number {
  const match = ID_TEXT_RE.exec(text);
  if (match === null) {
    throw new BoardError(`invalid id: ${text}`);
  }
  return Number(match[1]);
}

/** Creates an empty board with the given title. */
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

function parseTask(raw: unknown, where: string): Task {
  if (!isRecord(raw)) {
    throw new BoardError(`${where} must be an object`);
  }
  requireExactKeys(raw, TASK_KEYS, where);
  if (typeof raw["done"] !== "boolean") {
    throw new BoardError(`${where} done must be a boolean`);
  }
  return { title: requireText(raw["title"], `${where} title`), done: raw["done"] };
}

function parseTasks(raw: unknown, where: string): Task[] {
  if (!Array.isArray(raw)) {
    throw new BoardError(`${where} tasks must be an array`);
  }
  return raw.map((t, i) => parseTask(t, `${where} task ${String(i + 1)}`));
}

function parseCard(raw: unknown, index: number): Card {
  const where = `card #${String(index)}`;
  if (!isRecord(raw)) {
    throw new BoardError(`${where} must be an object`);
  }
  requireExactKeys(raw, CARD_KEYS, where);
  const id = raw["id"];
  if (!isPositiveInteger(id)) {
    throw new BoardError(`${where}: invalid id`);
  }
  return {
    id,
    title: requireText(raw["title"], `${where} title`),
    description: requireString(raw["description"], `${where} description`),
    column: requireColumn(raw["column"]),
    milestone: requireString(raw["milestone"], `${where} milestone`),
    createdAt: requireDate(raw["createdAt"], `${where} createdAt`),
    updatedAt: requireDate(raw["updatedAt"], `${where} updatedAt`),
    tasks: parseTasks(raw["tasks"], where),
  };
}

/** Validates an unknown board (JSON read from disk) and returns it typed. */
export function parseBoard(raw: unknown): Board {
  if (!isRecord(raw)) {
    throw new BoardError("board must be an object");
  }
  requireExactKeys(raw, BOARD_KEYS, "board");
  if (raw["version"] !== 1) {
    throw new BoardError(`unsupported version: ${String(raw["version"])}`);
  }
  const nextId = raw["nextId"];
  if (!isPositiveInteger(nextId)) {
    throw new BoardError("nextId must be an integer >= 1");
  }
  const cards = raw["cards"];
  if (!Array.isArray(cards)) {
    throw new BoardError("cards must be an array");
  }
  const parsed = cards.map((c, i) => parseCard(c, i));
  const seen = new Set<number>();
  for (const card of parsed) {
    if (seen.has(card.id)) {
      throw new BoardError(`duplicate id: ${formatId(card.id)}`);
    }
    seen.add(card.id);
    if (card.id >= nextId) {
      throw new BoardError(`${formatId(card.id)} must be lower than nextId (${String(nextId)})`);
    }
  }
  return { version: 1, title: requireText(raw["title"], "title"), nextId, cards: parsed };
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
      tasks: c.tasks.map((t) => ({ title: t.title, done: t.done })),
    })),
  };
  return `${JSON.stringify(ordered, null, 2)}\n`;
}

// ---------- Commands ----------

/** Fields of a new card; only the title is required. */
export interface AddInput {
  readonly title: string;
  readonly description?: string | undefined;
  readonly column?: ColumnId | undefined;
  readonly milestone?: string | undefined;
}

/** Fields to change on a card; at least one must be given. */
export interface EditInput {
  readonly title?: string | undefined;
  readonly description?: string | undefined;
  readonly milestone?: string | undefined;
}

/** Returns the card with this id, or throws a `BoardError`. */
export function findCard(board: Board, id: number): Card {
  const card = board.cards.find((c) => c.id === id);
  if (card === undefined) {
    throw new BoardError(`card not found: ${formatId(id)}`);
  }
  return card;
}

/** Adds a card at the end of the board and returns the new board and card. */
export function addCard(board: Board, input: AddInput, today: string): { board: Board; card: Card } {
  const date = requireDate(today, "today's date");
  const card: Card = {
    id: board.nextId,
    title: requireText(input.title, "title"),
    description: (input.description ?? "").trim(),
    column: input.column ?? "later",
    milestone: (input.milestone ?? "").trim(),
    createdAt: date,
    updatedAt: date,
    tasks: [],
  };
  return { board: { ...board, nextId: board.nextId + 1, cards: [...board.cards, card] }, card };
}

/** Moves a card to a column, at a given position among that column's cards (end by default). */
export function moveCard(board: Board, id: number, column: ColumnId, position: number | undefined, today: string): Board {
  const date = requireDate(today, "today's date");
  const card: Card = { ...findCard(board, id), column, updatedAt: date };
  const others = board.cards.filter((c) => c.id !== id);
  const inTarget = others.filter((c) => c.column === column);
  const pos = position ?? inTarget.length;
  if (!Number.isInteger(pos) || pos < 0 || pos > inTarget.length) {
    throw new BoardError(`invalid position: ${String(pos)} (0 to ${String(inTarget.length)})`);
  }
  const anchor = inTarget[pos];
  const insertAt = anchor === undefined ? others.length : others.indexOf(anchor);
  return { ...board, cards: [...others.slice(0, insertAt), card, ...others.slice(insertAt)] };
}

/** Changes the given fields of a card and updates its date. */
export function editCard(board: Board, id: number, patch: EditInput, today: string): Board {
  const date = requireDate(today, "today's date");
  const current = findCard(board, id);
  if (patch.title === undefined && patch.description === undefined && patch.milestone === undefined) {
    throw new BoardError("nothing to edit");
  }
  const card: Card = {
    ...current,
    title: patch.title === undefined ? current.title : requireText(patch.title, "title"),
    description: patch.description === undefined ? current.description : patch.description.trim(),
    milestone: patch.milestone === undefined ? current.milestone : patch.milestone.trim(),
    updatedAt: date,
  };
  return { ...board, cards: board.cards.map((c) => (c.id === id ? card : c)) };
}

/** Removes a card. Its id is never reused. */
export function removeCard(board: Board, id: number): Board {
  findCard(board, id);
  return { ...board, cards: board.cards.filter((c) => c.id !== id) };
}

// ---------- Tasks ----------

/** A step together with its display status. */
export interface TaskWithStatus extends Task {
  readonly status: TaskStatus;
}

/**
 * Index of the step in progress: the first step not done, and only for a card in the `doing` column.
 * A card waiting in another column has nothing in progress. Returns -1 when there is none.
 */
export function currentTaskIndex(card: Pick<Card, "column" | "tasks">): number {
  return card.column === "doing" ? card.tasks.findIndex((t) => !t.done) : -1;
}

/** Status of each task: done, the one in progress (see {@link currentTaskIndex}), the rest to do. */
export function withStatus(card: Pick<Card, "column" | "tasks">): TaskWithStatus[] {
  const current = currentTaskIndex(card);
  return card.tasks.map((t, i) => ({ ...t, status: t.done ? "done" : i === current ? "doing" : "todo" }));
}

/** Parses a 1-based task number typed by a user. */
export function parseTaskNumber(text: string): number {
  if (!TASK_NUMBER_RE.test(text)) {
    throw new BoardError(`invalid task number: ${text}`);
  }
  return Number(text);
}

function updateCard(board: Board, id: number, today: string, change: (card: Card) => Card): Board {
  const date = requireDate(today, "today's date");
  const card = { ...change(findCard(board, id)), updatedAt: date };
  return { ...board, cards: board.cards.map((c) => (c.id === id ? card : c)) };
}

function requireTaskIndex(card: Card, n: number): number {
  if (!Number.isInteger(n) || n < 1 || n > card.tasks.length) {
    throw new BoardError(`invalid task number for ${formatId(card.id)}: ${String(n)} (1 to ${String(card.tasks.length)})`);
  }
  return n - 1;
}

/** Adds a step at the end of a card. */
export function addTask(board: Board, id: number, title: string, today: string): Board {
  const task: Task = { title: requireText(title, "task title"), done: false };
  return updateCard(board, id, today, (card) => ({ ...card, tasks: [...card.tasks, task] }));
}

/** Marks a task done, or the task in progress when no number is given. */
export function completeTask(board: Board, id: number, n: number | undefined, today: string): Board {
  return updateCard(board, id, today, (card) => {
    const index = n === undefined ? currentTaskIndex(card) : requireTaskIndex(card, n);
    if (index === -1) {
      throw new BoardError(`${formatId(card.id)} has no task in progress`);
    }
    return { ...card, tasks: card.tasks.map((t, i) => (i === index ? { ...t, done: true } : t)) };
  });
}

/** Marks step `n` (1-based) of a card as not done. */
export function reopenTask(board: Board, id: number, n: number, today: string): Board {
  return updateCard(board, id, today, (card) => {
    const index = requireTaskIndex(card, n);
    return { ...card, tasks: card.tasks.map((t, i) => (i === index ? { ...t, done: false } : t)) };
  });
}

/** Renames step `n` (1-based) of a card. */
export function renameTask(board: Board, id: number, n: number, title: string, today: string): Board {
  return updateCard(board, id, today, (card) => {
    const index = requireTaskIndex(card, n);
    const text = requireText(title, "task title");
    return { ...card, tasks: card.tasks.map((t, i) => (i === index ? { ...t, title: text } : t)) };
  });
}

/** Removes step `n` (1-based) of a card. */
export function removeTask(board: Board, id: number, n: number, today: string): Board {
  return updateCard(board, id, today, (card) => {
    const index = requireTaskIndex(card, n);
    return { ...card, tasks: card.tasks.filter((_, i) => i !== index) };
  });
}
