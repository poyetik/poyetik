import { parseArgs } from "node:util";
import {
  ArgumentError,
  addCard,
  addStep,
  completeStep,
  BoardError,
  COLUMNS,
  editCard,
  formatId,
  isColumnId,
  moveCard,
  parseBoard,
  parseId,
  parseStepNumber,
  removeCard,
  removeStep,
  renameStep,
  reopenStep,
  serializeBoard,
  setTitle,
  withStatus,
  findCard,
  type Board,
  type Card,
  type ColumnId,
  type EditInput,
} from "../lib/board.ts";

/** Injected I/O: the CLI never touches the disk or the clock directly. */
export interface Io {
  /** The board file, as named in messages. */
  readonly file: string;
  readonly read: () => string;
  readonly write: (text: string) => void;
  readonly out: (line: string) => void;
  readonly err: (line: string) => void;
  readonly today: () => string;
}

/** Thrown when the command line is not a valid command: exit code 2. */
export class UsageError extends Error {
  /**
   * Describes what is wrong with the command line.
   * @param message What is wrong.
   */
  constructor(message: string) {
    super(message);
    this.name = "UsageError";
  }
}

/** Help text printed by `roadmap help`. */
export const HELP = `Usage: roadmap <command> [options]

  list [--column <col>]                       List cards (ids: 12 or #12)
  show <id>                                   Show a card
  add <title> [--desc <t>] [--column <col>] [--milestone <m>]
  move <id> <col> [--position <n>]            Move (position within the column, 0 = top)
  edit <id> [--title <t>] [--desc <t>] [--milestone <m>]
  rm <id>                                     Remove a card
  title <text>                                Set the board title (the page heading)
  step add <id> <title>                       Add a step at the end
  step done <id> [n]                          Mark step n done (default: the step in progress)
  step undo <id> <n>                          Mark step n not done
  step edit <id> <n> <title>                  Rename step n
  step rm <id> <n>                            Remove step n
  validate                                    Check board.json
  help                                        Show this help

Columns: ${COLUMNS.map((column) => `${column.id} (${column.title})`).join(", ")}
Steps: in a card of the doing column, the first step that is not done is in progress.`;

/** Local date formatted as YYYY-MM-DD. */
export function localDate(date: Date): string {
  const pad = (part: number): string => String(part).padStart(2, "0");
  return `${String(date.getFullYear())}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

/** Suffix printed after a step for each status; a step still to do gets none, so only Done and In progress stand out. */
const TAGS = { done: "  [Done]", doing: "  [In progress]", todo: "" } as const;

/** One-line summary of a card: id, column, title, milestone in brackets when set, and `done/total` steps when it has any. */
function formatCard(card: Card): string {
  const milestone = card.milestone === "" ? "" : ` [${card.milestone}]`;
  const done = card.steps.filter((step) => step.done).length;
  const progress = card.steps.length === 0 ? "" : `  ${String(done)}/${String(card.steps.length)}`;
  return `${formatId(card.id)}  ${card.column.padEnd(5)}  ${card.title}${milestone}${progress}`;
}

/** Prints a card in full: its summary line, its description when not empty, then its numbered steps (1-based, as the `step` commands expect) with their status tag. */
function printCard(io: Io, card: Card): void {
  io.out(formatCard(card));
  if (card.description !== "") {
    io.out(card.description);
  }
  withStatus(card).forEach((step, i) => {
    io.out(`  ${String(i + 1)}. ${step.title}${TAGS[step.status]}`);
  });
}

/** Usage line of each `step` action; its keys are also the list of valid actions. */
const STEP_USAGE = {
  add: "step add <id> <title>",
  done: "step done <id> [n]",
  undo: "step undo <id> <n>",
  edit: "step edit <id> <n> <title>",
  rm: "step rm <id> <n>",
} as const;

/** A valid `step` action name. */
type StepAction = keyof typeof STEP_USAGE;

/** Whether `value` names a `step` action; `Object.hasOwn` so that inherited names like `toString` are refused. */
function isStepAction(value: string): value is StepAction {
  return Object.hasOwn(STEP_USAGE, value);
}

/** Runs `roadmap step <action> ...`: checks the action and its argument count (a `UsageError` otherwise), applies the change, saves the board and prints the changed card. */
function runStep(args: readonly string[], io: Io): void {
  const [action = "", ...rest] = args;
  if (!isStepAction(action)) {
    throw new UsageError(
      `unknown step action: ${action === "" ? "(none)" : action} (expected: ${Object.keys(STEP_USAGE).join(", ")})`,
    );
  }
  const usage = new UsageError(`expected: ${STEP_USAGE[action]}`);
  const board = load(io);
  const today = io.today();
  let next: Board;
  let id: number;
  switch (action) {
    case "add": {
      if (!isTwo(rest)) throw usage;
      const [idText, title] = rest;
      id = parseId(idText);
      next = addStep(board, id, title, today);
      break;
    }
    case "done": {
      if (isOne(rest)) {
        id = parseId(rest[0]);
        next = completeStep(board, id, undefined, today);
        break;
      }
      if (!isTwo(rest)) throw usage;
      id = parseId(rest[0]);
      next = completeStep(board, id, parseStepNumber(rest[1]), today);
      break;
    }
    case "undo": {
      if (!isTwo(rest)) throw usage;
      const [idText, stepNumber] = rest;
      id = parseId(idText);
      next = reopenStep(board, id, parseStepNumber(stepNumber), today);
      break;
    }
    case "edit": {
      if (!isThree(rest)) throw usage;
      const [idText, stepNumber, title] = rest;
      id = parseId(idText);
      next = renameStep(board, id, parseStepNumber(stepNumber), title, today);
      break;
    }
    case "rm": {
      if (!isTwo(rest)) throw usage;
      const [idText, stepNumber] = rest;
      id = parseId(idText);
      next = removeStep(board, id, parseStepNumber(stepNumber), today);
      break;
    }
  }
  save(io, next);
  printCard(io, findCard(next, id));
}

/** Checks a column typed on the command line; an unknown one is a `UsageError` listing the valid ids. */
function toColumn(value: string): ColumnId {
  if (!isColumnId(value)) {
    throw new UsageError(`unknown column: ${value} (expected: ${COLUMNS.map((column) => column.id).join(", ")})`);
  }
  return value;
}

/** Parses the `--position` option: a non-negative integer (0 = top of the column), or undefined when absent. The upper bound is checked by `moveCard`. */
function toPosition(value: string | undefined): number | undefined {
  if (value === undefined) {
    return undefined;
  }
  if (!/^\d+$/.test(value)) {
    throw new UsageError(`invalid position: ${value}`);
  }
  return Number(value);
}

/** Whether exactly one positional argument was given; narrows the array to a one-element tuple. */
function isOne(args: readonly string[]): args is readonly [string] {
  return args.length === 1;
}

/** Whether exactly two positional arguments were given; narrows the array to a pair. */
function isTwo(args: readonly string[]): args is readonly [string, string] {
  return args.length === 2;
}

/** Whether exactly three positional arguments were given; narrows the array to a triple. */
function isThree(args: readonly string[]): args is readonly [string, string, string] {
  return args.length === 3;
}

/** Returns the single positional argument `name`; none or more than one is a `UsageError`. */
function onlyArg(positionals: readonly string[], name: string): string {
  const [value, ...rest] = positionals;
  if (value === undefined || rest.length > 0) {
    throw new UsageError(`expected exactly one argument: ${name}`);
  }
  return value;
}

/** Reads and validates the board file; malformed JSON or an invalid board is a `BoardError` naming the file (exit code 1). */
function load(io: Io): Board {
  const text = io.read();
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch (error: unknown) {
    throw new BoardError(`unreadable ${io.file}: ${String(error)}`);
  }
  try {
    return parseBoard(raw);
  } catch (error: unknown) {
    // `parseBoard` throws only `BoardError`s; the file is named here, so the model stays free of any file context.
    throw new BoardError(`invalid ${io.file}: ${failure(error)}`);
  }
}

/** Writes the board back in its stable serialization, so each change is a minimal git diff. */
function save(io: Io, board: Board): void {
  io.write(serializeBoard(board));
}

/** Parses the options and dispatches one command; errors are thrown and turned into an exit code by `run`. Options are strict: an unknown one throws a `TypeError` from `parseArgs`. */
function execute(args: readonly string[], io: Io): void {
  const [command, ...rest] = args;
  const { values, positionals } = parseArgs({
    args: rest,
    allowPositionals: true,
    strict: true,
    options: {
      column: { type: "string" },
      desc: { type: "string" },
      milestone: { type: "string" },
      position: { type: "string" },
      title: { type: "string" },
    },
  });

  switch (command) {
    case undefined:
    case "help": {
      io.out(HELP);
      return;
    }
    case "list": {
      const column = values.column === undefined ? undefined : toColumn(values.column);
      const cards = load(io).cards.filter((card) => column === undefined || card.column === column);
      if (cards.length === 0) {
        io.out("(no cards)");
      }
      for (const card of cards) {
        io.out(formatCard(card));
      }
      return;
    }
    case "show": {
      const card = findCard(load(io), parseId(onlyArg(positionals, "id")));
      printCard(io, card);
      io.out(`created ${card.createdAt}, updated ${card.updatedAt}`);
      return;
    }
    case "add": {
      const title = onlyArg(positionals, "title");
      const result = addCard(
        load(io),
        {
          title,
          description: values.desc,
          column: values.column === undefined ? undefined : toColumn(values.column),
          milestone: values.milestone,
        },
        io.today(),
      );
      save(io, result.board);
      io.out(`added: ${formatCard(result.card)}`);
      return;
    }
    case "move": {
      if (!isTwo(positionals)) {
        throw new UsageError("expected: move <id> <column>");
      }
      const [idText, column] = positionals;
      const id = parseId(idText);
      const target = toColumn(column);
      save(io, moveCard(load(io), id, target, toPosition(values.position), io.today()));
      io.out(`moved: ${formatId(id)} → ${target}`);
      return;
    }
    case "edit": {
      const id = parseId(onlyArg(positionals, "id"));
      const patch: EditInput = { title: values.title, description: values.desc, milestone: values.milestone };
      save(io, editCard(load(io), id, patch, io.today()));
      io.out(`edited: ${formatId(id)}`);
      return;
    }
    case "rm": {
      const id = parseId(onlyArg(positionals, "id"));
      save(io, removeCard(load(io), id));
      io.out(`removed: ${formatId(id)}`);
      return;
    }
    case "title": {
      const title = onlyArg(positionals, "title");
      const board = setTitle(load(io), title);
      save(io, board);
      io.out(`title: ${board.title}`);
      return;
    }
    case "step": {
      runStep(positionals, io);
      return;
    }
    case "validate": {
      const board = load(io);
      io.out(`valid board: ${String(board.cards.length)} card(s)`);
      return;
    }
    default: {
      throw new UsageError(`unknown command: ${command}`);
    }
  }
}

/**
 * Testable entry point: returns the exit code: 0 on success, 1 when the command failed,
 * 2 for a usage error. Every error is one `error:` line, never a stack trace.
 */
export function run(args: readonly string[], io: Io): number {
  try {
    execute(args, io);
    return 0;
  } catch (error) {
    // parseArgs reports unknown or malformed options as TypeErrors with a code.
    if (error instanceof UsageError || error instanceof ArgumentError || (error instanceof TypeError && "code" in error)) {
      io.err(`error: ${error.message} (see: roadmap help)`);
      return 2;
    }
    if (error instanceof BoardError) {
      io.err(`error: ${error.message}`);
      return 1;
    }
    throw error;
  }
}

/**
 * Describes why a file operation failed, for an `error:` line that already names the file:
 * a system error by its stable code (`ENOENT`, `EACCES`, `ERR_ENCODING_INVALID_ENCODED_DATA`),
 * since Node's sentence repeats the path and may change; anything else by its message.
 */
export function failure(error: unknown): string {
  if (error instanceof Error) {
    return "code" in error && typeof error.code === "string" ? error.code : error.message;
  }
  return String(error);
}

/** A stream as `guardStreams` needs it: one that reports its failures as `error` events. */
export interface ErrorSource {
  readonly on: (event: "error", listener: (error: Error) => void) => unknown;
}

/**
 * Applies the command-line contract to the output streams, which fail after `run` returned:
 * - a closed stdout (a reader gone, as with `| head`) is not an error: what nobody reads is
 *   dropped, and the exit code stays the outcome of the command;
 * - any other stdout failure is one `error:` line on stderr, then `failed` is called, which sets
 *   the exit code (see `exitAfterStreamFailure`);
 * - a stderr failure is ignored, since there is nowhere left to report it: the exit code still tells.
 *
 * Never a stack trace: without a listener, Node would throw the stream error.
 */
export function guardStreams(stdout: ErrorSource, stderr: ErrorSource & { readonly write: (text: string) => unknown }, failed: () => void): void {
  stderr.on("error", () => undefined);
  stdout.on("error", (error) => {
    if ("code" in error && error.code === "EPIPE") {
      return;
    }
    stderr.write(`error: cannot write to stdout: ${failure(error)}\n`);
    failed();
  });
}

/** The exit code once stdout failed: 1 after a success, unchanged after a failure (a usage error keeps its 2). */
export function exitAfterStreamFailure(code: number | string | null | undefined): number | string | null | undefined {
  return code === 0 ? 1 : code;
}
