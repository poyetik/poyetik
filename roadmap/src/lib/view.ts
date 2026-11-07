import { COLUMNS, withStatus, type Board, type Card, type ColumnId, type TaskWithStatus } from "./board.ts";

export interface CardView extends Card {
  /** Steps with their status: the first one not done is in progress. */
  readonly steps: readonly TaskWithStatus[];
  readonly doneSteps: number;
  /** Title of the step in progress, or null when every step is done or there is none. */
  readonly currentStep: string | null;
}

export interface ColumnView {
  readonly id: ColumnId;
  readonly title: string;
  readonly cards: readonly CardView[];
}

export interface BoardView {
  readonly title: string;
  readonly columns: readonly ColumnView[];
  readonly total: number;
  readonly done: number;
  /** Date of the most recent card update, or null if the board is empty. */
  readonly lastUpdated: string | null;
}

function toCardView(card: Card): CardView {
  return {
    ...card,
    steps: withStatus(card.tasks),
    doneSteps: card.tasks.filter((t) => t.done).length,
    currentStep: card.tasks.find((t) => !t.done)?.title ?? null,
  };
}

/** Prepares the display: cards grouped by column, in board order. */
export function toView(board: Board): BoardView {
  const columns = COLUMNS.map((col) => ({
    id: col.id,
    title: col.title,
    cards: board.cards.filter((c) => c.column === col.id).map(toCardView),
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
