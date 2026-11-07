import { parseArgs } from "node:util";
import {
  addCard,
  addTask,
  completeTask,
  BoardError,
  COLUMNS,
  editCard,
  formatId,
  isColumnId,
  moveCard,
  parseBoard,
  parseId,
  parseTaskNumber,
  removeCard,
  removeTask,
  renameTask,
  reopenTask,
  serializeBoard,
  withStatus,
  findCard,
  type Board,
  type Card,
  type ColumnId,
  type EditInput,
} from "../lib/board.ts";

/** Injected I/O: the CLI never touches the disk or the clock directly. */
export interface Io {
  readonly read: () => string;
  readonly write: (text: string) => void;
  readonly out: (line: string) => void;
  readonly err: (line: string) => void;
  readonly today: () => string;
}

export const HELP = `Usage: roadmap <command> [options]

  list [--column <col>]                       List cards (ids: 12 or #12)
  show <id>                                   Show a card
  add <title> [--desc <t>] [--column <col>] [--milestone <m>]
  move <id> <col> [--position <n>]            Move (position within the column, 0 = top)
  edit <id> [--title <t>] [--desc <t>] [--milestone <m>]
  rm <id>                                     Remove a card
  task add <id> <title>                       Add a step at the end
  task done <id> [n]                          Mark step n done (default: the step in progress)
  task undo <id> <n>                          Mark step n not done
  task edit <id> <n> <title>                  Rename step n
  task rm <id> <n>                            Remove step n
  validate                                    Check board.json
  help                                        Show this help

Columns: ${COLUMNS.map((c) => `${c.id} (${c.title})`).join(", ")}
Steps: the first step that is not done is in progress.`;

/** Local date formatted as YYYY-MM-DD. */
export function localDate(d: Date): string {
  const pad = (n: number): string => String(n).padStart(2, "0");
  return `${String(d.getFullYear())}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

const TAGS = { done: "  [Done]", doing: "  [In progress]", todo: "" } as const;

function formatCard(card: Card): string {
  const milestone = card.milestone === "" ? "" : ` [${card.milestone}]`;
  const done = card.tasks.filter((t) => t.done).length;
  const progress = card.tasks.length === 0 ? "" : `  ${String(done)}/${String(card.tasks.length)}`;
  return `${formatId(card.id)}  ${card.column.padEnd(5)}  ${card.title}${milestone}${progress}`;
}

function printCard(io: Io, card: Card): void {
  io.out(formatCard(card));
  if (card.description !== "") {
    io.out(card.description);
  }
  withStatus(card.tasks).forEach((t, i) => {
    io.out(`  ${String(i + 1)}. ${t.title}${TAGS[t.status]}`);
  });
}

const TASK_USAGE = {
  add: "task add <id> <title>",
  done: "task done <id> [n]",
  undo: "task undo <id> <n>",
  edit: "task edit <id> <n> <title>",
  rm: "task rm <id> <n>",
} as const;

type TaskAction = keyof typeof TASK_USAGE;

function isTaskAction(value: string): value is TaskAction {
  return Object.hasOwn(TASK_USAGE, value);
}

function runTask(args: readonly string[], io: Io): void {
  const [action = "", ...rest] = args;
  if (!isTaskAction(action)) {
    throw new BoardError(
      `unknown task action: ${action || "(none)"} (expected: ${Object.keys(TASK_USAGE).join(", ")})`,
    );
  }
  const usage = new BoardError(`expected: ${TASK_USAGE[action]}`);
  const board = load(io);
  const today = io.today();
  let next: Board;
  let id: number;
  switch (action) {
    case "add": {
      if (rest.length !== 2) throw usage;
      const [idText, title] = rest as [string, string];
      id = parseId(idText);
      next = addTask(board, id, title, today);
      break;
    }
    case "done": {
      if (rest.length !== 1 && rest.length !== 2) throw usage;
      const [idText, n] = rest as [string, string | undefined];
      id = parseId(idText);
      next = completeTask(board, id, n === undefined ? undefined : parseTaskNumber(n), today);
      break;
    }
    case "undo": {
      if (rest.length !== 2) throw usage;
      const [idText, n] = rest as [string, string];
      id = parseId(idText);
      next = reopenTask(board, id, parseTaskNumber(n), today);
      break;
    }
    case "edit": {
      if (rest.length !== 3) throw usage;
      const [idText, n, title] = rest as [string, string, string];
      id = parseId(idText);
      next = renameTask(board, id, parseTaskNumber(n), title, today);
      break;
    }
    case "rm": {
      if (rest.length !== 2) throw usage;
      const [idText, n] = rest as [string, string];
      id = parseId(idText);
      next = removeTask(board, id, parseTaskNumber(n), today);
      break;
    }
  }
  save(io, next);
  printCard(io, findCard(next, id));
}

function toColumn(value: string): ColumnId {
  if (!isColumnId(value)) {
    throw new BoardError(`unknown column: ${value} (expected: ${COLUMNS.map((c) => c.id).join(", ")})`);
  }
  return value;
}

function toPosition(value: string | undefined): number | undefined {
  if (value === undefined) {
    return undefined;
  }
  if (!/^\d+$/.test(value)) {
    throw new BoardError(`invalid position: ${value}`);
  }
  return Number(value);
}

function onlyArg(positionals: readonly string[], name: string): string {
  const [value, ...rest] = positionals;
  if (value === undefined || rest.length > 0) {
    throw new BoardError(`expected exactly one argument: ${name}`);
  }
  return value;
}

function load(io: Io): Board {
  let raw: unknown;
  try {
    raw = JSON.parse(io.read());
  } catch (error) {
    throw new BoardError(`unreadable board.json: ${error instanceof Error ? error.message : String(error)}`);
  }
  return parseBoard(raw);
}

function save(io: Io, board: Board): void {
  io.write(serializeBoard(board));
}

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
      const cards = load(io).cards.filter((c) => column === undefined || c.column === column);
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
      if (positionals.length !== 2) {
        throw new BoardError("expected: move <id> <column>");
      }
      const [idText, column] = positionals as [string, string];
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
    case "task": {
      runTask(positionals, io);
      return;
    }
    case "validate": {
      const board = load(io);
      io.out(`valid board: ${String(board.cards.length)} card(s)`);
      return;
    }
    default: {
      throw new BoardError(`unknown command: ${command}`);
    }
  }
}

/** Testable entry point: returns the exit code. */
export function run(args: readonly string[], io: Io): number {
  try {
    execute(args, io);
    return 0;
  } catch (error) {
    if (error instanceof BoardError || (error instanceof TypeError && "code" in error)) {
      io.err(`error: ${error.message}`);
      return 1;
    }
    throw error;
  }
}
