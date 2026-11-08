import { COLUMNS, currentStepIndex, withStatus, type Board, type Card, type ColumnId, type StepWithStatus } from "./board.ts";

/** A card prepared for display. */
export interface CardView extends Card {
  /** The card's steps with their status (refining `Card.steps`, since `StepWithStatus` extends `Step`): in the `doing` column, the first one not done is in progress. */
  readonly steps: readonly StepWithStatus[];
  readonly doneSteps: number;
  /** Title of the step in progress, or null when there is none (card not in `doing`, no step left, or no step). */
  readonly currentStep: string | null;
}

/** A column and its cards, prepared for display. */
export interface ColumnView {
  readonly id: ColumnId;
  readonly title: string;
  readonly cards: readonly CardView[];
}

/** The whole board, prepared for display. */
export interface BoardView {
  readonly title: string;
  readonly columns: readonly ColumnView[];
  readonly total: number;
  readonly done: number;
  /** Date of the most recent card update, or null if the board is empty. */
  readonly lastUpdated: string | null;
}

/** Adds to a card its step statuses, its count of done steps and the title of the step in progress. */
function toCardView(card: Card): CardView {
  return {
    ...card,
    steps: withStatus(card),
    doneSteps: card.steps.filter((step) => step.done).length,
    // Index -1 (nothing in progress) reads `undefined`, hence null.
    currentStep: card.steps[currentStepIndex(card)]?.title ?? null,
  };
}

/**
 * The rank of each column on the page: what is in progress first, then what comes next,
 * later, and what is done. On a narrow screen the columns stack, and the work under way
 * must not come after the backlog.
 */
export const DISPLAY_RANK: Readonly<Record<ColumnId, number>> = { doing: 0, next: 1, later: 2, done: 3 };

/** Prepares the display: cards grouped by column, in display order, then in board order. */
export function toView(board: Board): BoardView {
  const columns = [...COLUMNS]
    .sort((left, right) => DISPLAY_RANK[left.id] - DISPLAY_RANK[right.id])
    .map((col) => ({
      id: col.id,
      title: col.title,
      cards: board.cards.filter((card) => card.column === col.id).map(toCardView),
    }));
  const dates = board.cards.map((card) => card.updatedAt).sort();
  return {
    title: board.title,
    columns,
    total: board.cards.length,
    done: board.cards.filter((card) => card.column === "done").length,
    lastUpdated: dates.at(-1) ?? null,
  };
}
