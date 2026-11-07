import { parseArgs } from "node:util";
import {
  addCard,
  BoardError,
  COLUMNS,
  editCard,
  isColumnId,
  moveCard,
  parseBoard,
  removeCard,
  serializeBoard,
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

  list [--column <col>]                       List cards
  show <id>                                   Show a card
  add <title> [--desc <t>] [--column <col>] [--milestone <m>]
  move <id> <col> [--position <n>]            Move (position within the column, 0 = top)
  edit <id> [--title <t>] [--desc <t>] [--milestone <m>]
  rm <id>                                     Remove a card
  validate                                    Check board.json
  help                                        Show this help

Columns: ${COLUMNS.map((c) => `${c.id} (${c.title})`).join(", ")}`;

/** Local date formatted as YYYY-MM-DD. */
export function localDate(d: Date): string {
  const pad = (n: number): string => String(n).padStart(2, "0");
  return `${String(d.getFullYear())}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

function formatCard(card: Card): string {
  const milestone = card.milestone === "" ? "" : ` [${card.milestone}]`;
  return `${card.id}  ${card.column.padEnd(5)}  ${card.title}${milestone}`;
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
      const id = onlyArg(positionals, "id");
      const card = load(io).cards.find((c) => c.id === id);
      if (card === undefined) {
        throw new BoardError(`card not found: ${id}`);
      }
      io.out(formatCard(card));
      if (card.description !== "") {
        io.out(card.description);
      }
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
      const [id, column] = positionals as [string, string];
      const target = toColumn(column);
      save(io, moveCard(load(io), id, target, toPosition(values.position), io.today()));
      io.out(`moved: ${id} → ${target}`);
      return;
    }
    case "edit": {
      const id = onlyArg(positionals, "id");
      const patch: EditInput = { title: values.title, description: values.desc, milestone: values.milestone };
      save(io, editCard(load(io), id, patch, io.today()));
      io.out(`edited: ${id}`);
      return;
    }
    case "rm": {
      const id = onlyArg(positionals, "id");
      save(io, removeCard(load(io), id));
      io.out(`removed: ${id}`);
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
