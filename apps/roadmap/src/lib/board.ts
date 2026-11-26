/**
 * Roadmap model: data + pure commands.
 * Every change goes through a command that returns a new board (never mutates).
 */

import { firstUnsafe, oneLine } from "@poyetik/config/one-line";

/** The four roadmap columns, in the order of the data and the CLI; the page shows them in this order, and its stylesheet moves In progress first on a phone (see `index.astro`). */
export const COLUMNS = [
  { id: "later", title: "Later" },
  { id: "next", title: "Next" },
  { id: "doing", title: "In progress" },
  { id: "done", title: "Done" },
] as const;

/** Identifier of a column. */
export type ColumnId = (typeof COLUMNS)[number]["id"];

/** One step of a card. In a card of the `doing` column, one step not done is the one in progress (see `currentStepIndex`). */
export interface Step {
  /** What the step is, as shown on the card. */
  readonly title: string;
  /** Whether the step is finished. */
  readonly done: boolean;
  /**
   * The review block the step belongs to, a safe integer of at least 1, or absent when the step is in none.
   * Optional in the file too, so a board without blocks keeps format version 1 unchanged. Nothing else about a
   * block is stored: its steps, its end and whether it is done are derived (see `cardBlocks`).
   */
  readonly block?: number;
}

/** Display status of a step: done, in progress (see `currentStepIndex`, only in the `doing` column) or still to do. */
export type StepStatus = "done" | "doing" | "todo";

/** A roadmap card (ticket) with its ordered steps. */
export interface Card {
  /** Unique, never reused: given from the board's `nextId`, it stays the card's number for good. */
  readonly id: number;
  /** Short title, never empty, trimmed. */
  readonly title: string;
  /** Longer description; empty when there is none. */
  readonly description: string;
  /** The column the card is in. */
  readonly column: ColumnId;
  /** The milestone it belongs to ("M1 Engine"…); empty when there is none. */
  readonly milestone: string;
  /** Day the card was added, `YYYY-MM-DD`, local time. */
  readonly createdAt: string;
  /** Day the card or one of its steps last changed, `YYYY-MM-DD`, local time. */
  readonly updatedAt: string;
  /** The steps, in order (stored under the key `tasks`). */
  readonly steps: readonly Step[];
}

/** The whole roadmap, as stored in `board.json`. */
export interface Board {
  /** Format version of `board.json`: only 1 exists, any other is refused. */
  readonly version: 1;
  /** Title of the board, shown as the page heading. */
  readonly title: string;
  /** Id of the next card added: above every id ever given, so that a removed card's id is never reused. */
  readonly nextId: number;
  /** Every card. Within a column, this is the order the page shows them in (`move` places a card in it). */
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
/** The fields every stored step has. */
const STEP_KEYS = ["title", "done"];
/** The field a stored step may have besides `STEP_KEYS`. */
const OPTIONAL_STEP_KEY = "block";
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

/**
 * Returns `value` trimmed; throws a `BoardError` naming `field` if it holds a control,
 * bidirectional formatting or line separator character (see `oneLine`): a text of the board
 * is printed by the CLI and shown on the page as it is, so it can neither drive a terminal,
 * forge a line nor reorder what is shown.
 */
function requireSafe(value: string, field: string): string {
  const unsafe = firstUnsafe(value);
  if (unsafe !== undefined) {
    throw new BoardError(`${field} must not hold the control or formatting character ${oneLine(unsafe)}`);
  }
  return value.trim();
}

/** Returns `value` trimmed; throws a `BoardError` naming `field` unless it is a string with at least one non-blank character, and no unsafe one (see `requireSafe`). */
function requireText(value: unknown, field: string): string {
  if (typeof value !== "string" || value.trim() === "") {
    throw new BoardError(`${field} must be a non-empty string`);
  }
  return requireSafe(value, field);
}

/** Returns `value` trimmed, possibly empty (optional texts such as a description); throws a `BoardError` naming `field` if it is not a string or holds an unsafe character (see `requireSafe`). */
function requireString(value: unknown, field: string): string {
  if (typeof value !== "string") {
    throw new BoardError(`${field} must be a string`);
  }
  return requireSafe(value, field);
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

/**
 * Throws a `BoardError` unless `value` has exactly the fields `keys`, in any order, and possibly the field
 * `optional`: no missing and no unknown field.
 */
function requireExactKeys(value: object, keys: readonly string[], what: string, optional?: string): void {
  const actual = Object.keys(value).sort();
  const expected = [...keys].sort();
  const required = actual.filter((key) => key !== optional);
  if (required.join(",") !== expected.join(",")) {
    const also = optional === undefined ? "" : ` (and optionally ${optional})`;
    throw new BoardError(`${what}: expected fields ${expected.join(", ")}${also}, got ${actual.join(", ")}`);
  }
}

/** Returns `value` as a block number; throws a `BoardError` naming `field` unless it is a safe integer of at least 1. */
function requireBlock(value: unknown, field: string): number {
  if (!isPositiveInteger(value)) {
    throw new BoardError(`${field} must be a safe integer >= 1`);
  }
  return value;
}

/**
 * Whether `value` is a safe integer of at least 1, the valid range of card ids and of `nextId`:
 * from 2^53 on, `n + 1 === n`, so a larger id could not be told apart from its neighbor.
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
  requireExactKeys(raw, STEP_KEYS, where, OPTIONAL_STEP_KEY);
  if (typeof raw["done"] !== "boolean") {
    throw new BoardError(`${where} done must be a boolean`);
  }
  const step = { title: requireText(raw["title"], `${where} title`), done: raw["done"] };
  return "block" in raw ? { ...step, block: requireBlock(raw["block"], `${where} block`) } : step;
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
      // A step in no block has `block` undefined, which JSON.stringify leaves out, so the file holds no key for it.
      tasks: card.steps.map((step) => ({ title: step.title, done: step.done, block: step.block })),
    })),
  };
  return `${JSON.stringify(ordered, null, 2)}\n`;
}

// ---------- Commands ----------

/** Fields of a new card; only the title is required. */
export interface AddInput {
  /** Title of the card; refused when empty once trimmed. */
  readonly title: string;
  /** Description; none by default. */
  readonly description?: string | undefined;
  /** Column it starts in; `later` by default. */
  readonly column?: ColumnId | undefined;
  /** Milestone; none by default. */
  readonly milestone?: string | undefined;
}

/** Fields to change on a card; at least one must be given. */
export interface EditInput {
  /** New title; refused when empty once trimmed. */
  readonly title?: string | undefined;
  /** New description; an empty one removes it. */
  readonly description?: string | undefined;
  /** New milestone; an empty one removes it. */
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
    description: requireString(input.description ?? "", "description"),
    column: input.column ?? "later",
    milestone: requireString(input.milestone ?? "", "milestone"),
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
    description: patch.description === undefined ? current.description : requireString(patch.description, "description"),
    milestone: patch.milestone === undefined ? current.milestone : requireString(patch.milestone, "milestone"),
    updatedAt: date,
  };
  return { ...board, cards: board.cards.map((existing) => (existing.id === id ? card : existing)) };
}

/** Sets the board's title, shown as the page heading; throws a `BoardError` if it is blank. */
export function setTitle(board: Board, title: string): Board {
  return { ...board, title: requireText(title, "title") };
}

/** Removes a card. Its id is never reused. */
export function removeCard(board: Board, id: number): Board {
  findCard(board, id);
  return { ...board, cards: board.cards.filter((card) => card.id !== id) };
}

// ---------- Steps ----------

/** A step together with its display status. */
export interface StepWithStatus extends Step {
  /** Whether the step is done, in progress or still to do (see `StepStatus`). */
  readonly status: StepStatus;
}

/**
 * Index of the step in progress, only for a card in the `doing` column: the first step not done of the lowest-numbered
 * review block that still has one, since a block is finished before the next begins; when every block is done, the
 * first step not done in no block. Within a block, card order: a block whose work goes in another order says so in its
 * steps' words, and its steps are ticked by number. A card waiting in another column has nothing in progress.
 * Returns -1 when there is none.
 */
export function currentStepIndex(card: Pick<Card, "column" | "steps">): number {
  if (card.column !== "doing") {
    return -1;
  }
  let current = -1;
  let lowest = Number.POSITIVE_INFINITY;
  card.steps.forEach((step, i) => {
    const rank = step.block ?? Number.POSITIVE_INFINITY;
    if (!step.done && (current === -1 || rank < lowest)) {
      current = i;
      lowest = rank;
    }
  });
  return current;
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

/** Puts step `stepNumber` (1-based) of a card in review block `block`, or in none when `block` is undefined. */
export function setStepBlock(board: Board, id: number, stepNumber: number, block: number | undefined, today: string): Board {
  return updateCard(board, id, today, (card) => {
    const index = requireStepIndex(card, stepNumber);
    const number = block === undefined ? undefined : requireBlock(block, "block");
    return {
      ...card,
      steps: card.steps.map((step, i) => {
        if (i !== index) {
          return step;
        }
        const kept: Step = { title: step.title, done: step.done };
        return number === undefined ? kept : { ...kept, block: number };
      }),
    };
  });
}

/** Parses a block number typed by a user: a positive integer without leading zero, or `none` for no block. */
export function parseBlock(text: string): number | undefined {
  if (text === "none") {
    return undefined;
  }
  const block = Number(STEP_NUMBER_RE.test(text) ? text : Number.NaN);
  if (!isPositiveInteger(block)) {
    throw new ArgumentError(`invalid block: ${text} (a number from 1, or none)`);
  }
  return block;
}

/** One review block of a card, derived from its steps' `block` fields. */
export interface CardBlock {
  /** The block's number. */
  readonly block: number;
  /** The 1-based numbers of its steps, in order (not always consecutive: a step added later may join an earlier block). */
  readonly steps: readonly number[];
  /** Whether every step of the block is done. */
  readonly done: boolean;
}

/** The review blocks of a card, by increasing number; empty when no step is in a block. */
export function cardBlocks(card: Pick<Card, "steps">): CardBlock[] {
  const numbers = [...new Set(card.steps.flatMap((step) => (step.block === undefined ? [] : [step.block])))].sort((left, right) => left - right);
  return numbers.map((block) => {
    const steps = card.steps.flatMap((step, i) => (step.block === block ? [{ number: i + 1, done: step.done }] : []));
    return { block, steps: steps.map((step) => step.number), done: steps.every((step) => step.done) };
  });
}

/** A run of block numbers no step uses, `from` to `to` included. */
export interface BlockGap {
  /** The first block number of the run. */
  readonly from: number;
  /** The last block number of the run, `from` or more. */
  readonly to: number;
}

/**
 * The runs of block numbers below a card's highest that no step uses (`[{ from: 4, to: 4 }]` for blocks 1, 2, 3
 * and 5), so that a block number skipped is seen. Found between the numbers used, in time bounded by the steps
 * whatever the numbers. Not refused when the board is read, since renumbering blocks one step at a time passes
 * through such states; `roadmap validate` reports them.
 */
export function blockGaps(card: Pick<Card, "steps">): BlockGap[] {
  let below = 0;
  return cardBlocks(card).flatMap(({ block }) => {
    const gap = block - below > 1 ? [{ from: below + 1, to: block - 1 }] : [];
    below = block;
    return gap;
  });
}

/** Writes a gap as a reader says it: "block 4", "blocks 2 and 3", "blocks 2 to 4". */
export function formatGap(gap: BlockGap): string {
  if (gap.from === gap.to) {
    return `block ${String(gap.from)}`;
  }
  return `blocks ${String(gap.from)} ${gap.to - gap.from === 1 ? "and" : "to"} ${String(gap.to)}`;
}

/**
 * Writes a list of step numbers as a reader says it, runs of three or more joined by "to":
 * `[4]` is "step 4", `[5, 6]` "steps 5 and 6", `[16, 17, 18, 39]` "steps 16 to 18 and 39".
 */
export function formatSteps(numbers: readonly number[]): string {
  const runs: number[][] = [];
  for (const number of numbers) {
    const last = runs.at(-1);
    if (last?.at(-1) === number - 1) {
      last.push(number);
    } else {
      runs.push([number]);
    }
  }
  const parts = runs.flatMap((run) => (run.length >= 3 ? [`${String(run[0])} to ${String(run.at(-1))}`] : run.map(String)));
  const head = parts.slice(0, -1);
  const tail = String(parts.at(-1));
  const list = head.length === 0 ? tail : `${head.join(", ")} and ${tail}`;
  return `${numbers.length === 1 ? "step" : "steps"} ${list}`;
}

/** What ends every review block, shown after its steps: the review, its fixes and the green full check of its pull request. */
export const BLOCK_END = "a deep review, its decided fixes, and a pull request whose full check is green";

/** Removes step `stepNumber` (1-based) of a card. */
export function removeStep(board: Board, id: number, stepNumber: number, today: string): Board {
  return updateCard(board, id, today, (card) => {
    const index = requireStepIndex(card, stepNumber);
    return { ...card, steps: card.steps.filter((_step, i) => i !== index) };
  });
}
