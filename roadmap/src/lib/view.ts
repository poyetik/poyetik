import { COLUMNS, type Board, type Card, type ColumnId } from "./board.ts";

export interface ColumnView {
  readonly id: ColumnId;
  readonly title: string;
  readonly cards: readonly Card[];
}

export interface BoardView {
  readonly title: string;
  readonly columns: readonly ColumnView[];
  readonly total: number;
  readonly done: number;
  /** Date of the most recent card update, or null if the board is empty. */
  readonly lastUpdated: string | null;
}

/** Prepares the display: cards grouped by column, in board order. */
export function toView(board: Board): BoardView {
  const columns = COLUMNS.map((col) => ({
    id: col.id,
    title: col.title,
    cards: board.cards.filter((c) => c.column === col.id),
  }));
  const dates = board.cards.map((c) => c.updatedAt).sort();
  return {
    title: board.title,
    columns,
    total: board.cards.length,
    done: board.cards.filter((c) => c.column === "done").length,
    lastUpdated: dates.at(-1) ?? null,
  };
}
