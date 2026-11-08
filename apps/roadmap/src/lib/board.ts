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
export interface Step {
  readonly title: string;
  readonly done: boolean;
}

/** Display status of a step: done, in progress (the first step not done, only in the `doing` column) or still to do. */
export type StepStatus = "done" | "doing" | "todo";

/** A roadmap card (ticket) with its ordered steps. */
export interface Card {
  readonly id: number;
  readonly title: string;
  readonly description: string;
  readonly column: ColumnId;
  readonly milestone: string;
  readonly createdAt: string;
  readonly updatedAt: string;
  readonly steps: readonly Step[];
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
  /** Error name shown in messages and stack traces; typed `string` so that subclasses can set their own. */
  override readonly name: string = "BoardError";
}

/** Thrown when a value typed by a user (an id, a step number) is not valid: a usage error. */
export class ArgumentError extends BoardError {
  /** Error name, telling a usage error apart from other `BoardError`s in output. */
  override readonly name = "ArgumentError";
}

/** A card id as typed by a user: a positive integer without leading zero, with an optional `#`. */
const ID_TEXT_RE = /^#?([1-9]\d*)$/;
/**
 * The exact fields of a stored card: an unknown or missing field is an error, so typos in board.json never pass silently.
 * The file keeps the key `tasks` for a card's steps (see GLOSSARY.md): `parseCard` reads it into `Card.steps`, and
 * `serializeBoard` writes `Card.steps` back under it, so the file format is unchanged.
 */
const CARD_KEYS = ["id", "title", "description", "column", "milestone", "createdAt", "updatedAt", "tasks"];
/** The exact fields of a stored step. */
const STEP_KEYS = ["title", "done"];
/** A 1-based step number as typed by a user: a positive integer without leading zero. */
const STEP_NUMBER_RE = /^[1-9]\d*$/;
/** The exact top-level fields of board.json. */
const BOARD_KEYS = ["version", "title", "nextId", "cards"];

/** Whether a value is a known column identifier. */
export function isColumnId(value: unknown): value is ColumnId {
  return COLUMNS.some((column) => column.id === value);
}

/** Formats a card id for display: `#12`. */
export function formatId(id: number): string {
  return `#${String(id)}`;
}

/** Parses a card id typed by a user: `12` or `#12`, at most `Number.MAX_SAFE_INTEGER` (a larger one would read back as another number). */
export function parseId(text: string): number {
  const id = Number(ID_TEXT_RE.exec(text)?.[1]);
  if (!isPositiveInteger(id)) {
    throw new ArgumentError(`invalid id: ${text}`);
  }
  return id;
}

/** Creates an empty board with the given title. */
export function emptyBoard(title: string): Board {
  return { version: 1, title: requireText(title, "title"), nextId: 1, cards: [] };
}

// ---------- Validation ----------

/** Returns `value` trimmed; throws a `BoardError` naming `field` unless it is a string with at least one non-blank character. */
function requireText(value: unknown, field: string): string {
  if (typeof value !== "string" || value.trim() === "") {
    throw new BoardError(`${field} must be a non-empty string`);
  }
  return value.trim();
}

/** Returns `value` trimmed, possibly empty (optional texts such as a description); throws a `BoardError` naming `field` if it is not a string. */
function requireString(value: unknown, field: string): string {
  if (typeof value !== "string") {
    throw new BoardError(`${field} must be a string`);
  }
  return value.trim();
}

/**
 * Whether `value` is a day of the calendar written YYYY-MM-DD: its midnight UTC must
 * write itself back as exactly `value`, which refuses other forms ("2026-1-1"), truncated
 * ones ("2026", "2026-11", which `Date.parse` accepts), impossible days (2026-02-31, which
 * would roll over) and months (2026-13-01, not a time at all). The final comparison is
 * strict, so it also proves that `value` is a string: anything else (`["2026-11-07"]`,
 * whose text is a valid day) is refused.
 */
function isCalendarDay(value: unknown): value is string {
  // An impossible month is not a time at all (NaN); an impossible day rolls over.
  const time = Date.parse(`${String(value)}T00:00:00Z`);
  return !Number.isNaN(time) && new Date(time).toISOString().slice(0, 10) === value;
}

/** Returns `value` unchanged if it is a valid YYYY-MM-DD calendar day; throws a `BoardError` naming `field` otherwise. */
function requireDate(value: unknown, field: string): string {
  if (!isCalendarDay(value)) {
    throw new BoardError(`${field} must be a YYYY-MM-DD date`);
  }
  return value;
}

/** Returns `value` as a column id; throws a `BoardError` listing the valid ids otherwise. */
function requireColumn(value: unknown): ColumnId {
  if (!isColumnId(value)) {
    throw new BoardError(`unknown column: ${String(value)} (expected: ${COLUMNS.map((column) => column.id).join(", ")})`);
  }
  return value;
}

/** Throws a `BoardError` unless `value` has exactly the fields `keys`, in any order: no missing and no extra field. */
function requireExactKeys(value: object, keys: readonly string[], what: string): void {
  const actual = Object.keys(value).sort();
  const expected = [...keys].sort();
  if (actual.join(",") !== expected.join(",")) {
    throw new BoardError(`${what}: expected fields ${expected.join(", ")}, got ${actual.join(", ")}`);
  }
}

/**
 * Whether `value` is a safe integer of at least 1, the valid range of card ids and of `nextId`:
 * from 2^53 on, `n + 1 === n`, so a larger id could not be told apart from its neighbour.
 */
function isPositiveInteger(value: unknown): value is number {
  return Number.isSafeInteger(value) && Number(value) >= 1;
}

/** Whether `value` is a plain object (not null, not an array), so that its fields can be read. */
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Validates one stored step; `where` locates it in error messages. */
function parseStep(raw: unknown, where: string): Step {
  if (!isRecord(raw)) {
    throw new BoardError(`${where} must be an object`);
  }
  requireExactKeys(raw, STEP_KEYS, where);
  if (typeof raw["done"] !== "boolean") {
    throw new BoardError(`${where} done must be a boolean`);
  }
  return { title: requireText(raw["title"], `${where} title`), done: raw["done"] };
}

/** Validates the step list of a card (the `tasks` field of the file); `where` names the card in error messages, and each step is located by its 1-based number. */
function parseSteps(raw: unknown, where: string): Step[] {
  if (!Array.isArray(raw)) {
    throw new BoardError(`${where} tasks must be an array`);
  }
  return raw.map((rawStep, i) => parseStep(rawStep, `${where} step ${String(i + 1)}`));
}

/**
 * Validates one stored card; `index` is its 0-based position in `cards`. Errors locate the card
 * by its 1-based entry in the file until its id is proven, then by both (`card #4 (entry 3)`):
 * ids are not in file order, so the entry alone would point at the wrong ticket, and the id
 * alone may be what is invalid.
 */
function parseCard(raw: unknown, index: number): Card {
  const entry = `entry ${String(index + 1)}`;
  if (!isRecord(raw)) {
    throw new BoardError(`card ${entry} must be an object`);
  }
  requireExactKeys(raw, CARD_KEYS, `card ${entry}`);
  const id = raw["id"];
  if (!isPositiveInteger(id)) {
    throw new BoardError(`card ${entry}: invalid id`);
  }
  const where = `card ${formatId(id)} (${entry})`;
  return {
    id,
    title: requireText(raw["title"], `${where} title`),
    description: requireString(raw["description"], `${where} description`),
    column: requireColumn(raw["column"]),
    milestone: requireString(raw["milestone"], `${where} milestone`),
    createdAt: requireDate(raw["createdAt"], `${where} createdAt`),
    updatedAt: requireDate(raw["updatedAt"], `${where} updatedAt`),
    steps: parseSteps(raw["tasks"], where),
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
    throw new BoardError("nextId must be a safe integer >= 1");
  }
  const cards = raw["cards"];
  if (!Array.isArray(cards)) {
    throw new BoardError("cards must be an array");
  }
  const parsed = cards.map((rawCard, i) => parseCard(rawCard, i));
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

/** Stable serialization (fixed key order, trailing newline) for clean git diffs; a card's steps are written under the file's key `tasks`. */
export function serializeBoard(board: Board): string {
  const ordered = {
    version: board.version,
    title: board.title,
    nextId: board.nextId,
    cards: board.cards.map((card) => ({
      id: card.id,
      title: card.title,
      description: card.description,
      column: card.column,
      milestone: card.milestone,
      createdAt: card.createdAt,
      updatedAt: card.updatedAt,
      tasks: card.steps.map((step) => ({ title: step.title, done: step.done })),
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
  const card = board.cards.find((candidate) => candidate.id === id);
  if (card === undefined) {
    throw new BoardError(`card not found: ${formatId(id)}`);
  }
  return card;
}

/** Adds a card at the end of the board and returns the new board and card; throws a `BoardError` when no safe id is left for the next card. */
export function addCard(board: Board, input: AddInput, today: string): { board: Board; card: Card } {
  const date = requireDate(today, "today's date");
  if (board.nextId === Number.MAX_SAFE_INTEGER) {
    throw new BoardError("no card id left");
  }
  const card: Card = {
    id: board.nextId,
    title: requireText(input.title, "title"),
    description: (input.description ?? "").trim(),
    column: input.column ?? "later",
    milestone: (input.milestone ?? "").trim(),
    createdAt: date,
    updatedAt: date,
    steps: [],
  };
  return { board: { ...board, nextId: board.nextId + 1, cards: [...board.cards, card] }, card };
}

/** Moves a card to a column, at a given position among that column's cards (end by default). */
export function moveCard(board: Board, id: number, column: ColumnId, position: number | undefined, today: string): Board {
  const date = requireDate(today, "today's date");
  const card: Card = { ...findCard(board, id), column, updatedAt: date };
  const others = board.cards.filter((other) => other.id !== id);
  const inTarget = others.filter((other) => other.column === column);
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
  return { ...board, cards: board.cards.map((existing) => (existing.id === id ? card : existing)) };
}

/** Removes a card. Its id is never reused. */
export function removeCard(board: Board, id: number): Board {
  findCard(board, id);
  return { ...board, cards: board.cards.filter((card) => card.id !== id) };
}

// ---------- Steps ----------

/** A step together with its display status. */
export interface StepWithStatus extends Step {
  readonly status: StepStatus;
}

/**
 * Index of the step in progress: the first step not done, and only for a card in the `doing` column.
 * A card waiting in another column has nothing in progress. Returns -1 when there is none.
 */
export function currentStepIndex(card: Pick<Card, "column" | "steps">): number {
  return card.column === "doing" ? card.steps.findIndex((step) => !step.done) : -1;
}

/** Status of each step: done, the one in progress (see {@link currentStepIndex}), the rest to do. */
export function withStatus(card: Pick<Card, "column" | "steps">): StepWithStatus[] {
  const current = currentStepIndex(card);
  return card.steps.map((step, i) => ({ ...step, status: step.done ? "done" : i === current ? "doing" : "todo" }));
}

/** Parses a 1-based step number typed by a user. */
export function parseStepNumber(text: string): number {
  if (!STEP_NUMBER_RE.test(text)) {
    throw new ArgumentError(`invalid step number: ${text}`);
  }
  return Number(text);
}

/** Applies `change` to the card `id`, stamps it with `today` (YYYY-MM-DD) and returns the new board; throws a `BoardError` if the date is invalid or the card does not exist. */
function updateCard(board: Board, id: number, today: string, change: (card: Card) => Card): Board {
  const date = requireDate(today, "today's date");
  const card = { ...change(findCard(board, id)), updatedAt: date };
  return { ...board, cards: board.cards.map((existing) => (existing.id === id ? card : existing)) };
}

/** Converts a 1-based step number into an index of `card.steps`; throws a `BoardError` giving the valid range otherwise. */
function requireStepIndex(card: Card, stepNumber: number): number {
  if (!Number.isInteger(stepNumber) || stepNumber < 1 || stepNumber > card.steps.length) {
    throw new BoardError(`invalid step number for ${formatId(card.id)}: ${String(stepNumber)} (1 to ${String(card.steps.length)})`);
  }
  return stepNumber - 1;
}

/** Adds a step at the end of a card. */
export function addStep(board: Board, id: number, title: string, today: string): Board {
  const step: Step = { title: requireText(title, "step title"), done: false };
  return updateCard(board, id, today, (card) => ({ ...card, steps: [...card.steps, step] }));
}

/** Marks a step done, or the step in progress when no number is given. */
export function completeStep(board: Board, id: number, stepNumber: number | undefined, today: string): Board {
  return updateCard(board, id, today, (card) => {
    const index = stepNumber === undefined ? currentStepIndex(card) : requireStepIndex(card, stepNumber);
    if (index === -1) {
      throw new BoardError(`${formatId(card.id)} has no step in progress`);
    }
    return { ...card, steps: card.steps.map((step, i) => (i === index ? { ...step, done: true } : step)) };
  });
}

/** Marks step `stepNumber` (1-based) of a card as not done. */
export function reopenStep(board: Board, id: number, stepNumber: number, today: string): Board {
  return updateCard(board, id, today, (card) => {
    const index = requireStepIndex(card, stepNumber);
    return { ...card, steps: card.steps.map((step, i) => (i === index ? { ...step, done: false } : step)) };
  });
}

/** Renames step `stepNumber` (1-based) of a card. */
export function renameStep(board: Board, id: number, stepNumber: number, title: string, today: string): Board {
  return updateCard(board, id, today, (card) => {
    const index = requireStepIndex(card, stepNumber);
    const text = requireText(title, "step title");
    return { ...card, steps: card.steps.map((step, i) => (i === index ? { ...step, title: text } : step)) };
  });
}

/** Removes step `stepNumber` (1-based) of a card. */
export function removeStep(board: Board, id: number, stepNumber: number, today: string): Board {
  return updateCard(board, id, today, (card) => {
    const index = requireStepIndex(card, stepNumber);
    return { ...card, steps: card.steps.filter((_step, i) => i !== index) };
  });
}
