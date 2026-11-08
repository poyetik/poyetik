import { COLUMNS, currentStepIndex, withStatus, type Board, type Card, type ColumnId, type StepWithStatus } from "./board.ts";

/** A card prepared for display. */
export interface CardView extends Card {
  /** The card's steps with their status (refining `Card.steps`, since `StepWithStatus` extends `Step`): in the `doing` column, the first one not done is in progress. */
  readonly steps: readonly StepWithStatus[];
  /** Number of steps done, out of `steps.length`. */
  readonly doneSteps: number;
  /** Title of the step in progress, or null when there is none (card not in `doing`, no step left, or no step). */
  readonly currentStep: string | null;
}

/** A column and its cards, prepared for display. */
export interface ColumnView {
  /** Identifier of the column, as stored. */
  readonly id: ColumnId;
  /** Title of the column, as shown ("In progress" for `doing`). */
  readonly title: string;
  /** The cards of the column, in display order. */
  readonly cards: readonly CardView[];
}

/**
 * What the `Board` component receives. Declared here, in TypeScript the policy lints,
 * rather than in the component's frontmatter, which no ESLint configuration reads yet.
 */
export interface BoardProps {
  /** The board to show. */
  readonly view: BoardView;
}

/** The whole board, prepared for display. */
export interface BoardView {
  /** Title of the board, shown as the page heading. */
  readonly title: string;
  /** The columns, in display order: what is in progress first. */
  readonly columns: readonly ColumnView[];
  /** Number of cards on the board. */
  readonly total: number;
  /** Number of cards in the `done` column. */
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
 * Prepares the display: cards grouped by column, in the classic kanban order of `COLUMNS`
 * (Later, Next, In progress, Done, from left to right), then in board order. On a phone the
 * page's stylesheet moves In progress first, so that the work under way never comes after
 * the backlog.
 */
export function toView(board: Board): BoardView {
  const columns = COLUMNS.map((col) => ({
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
