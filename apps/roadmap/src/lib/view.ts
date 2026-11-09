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
  /** The first cards of `cards`, always shown: every card, except in the Done column, which shows its {@link DONE_SHOWN} most recent. */
  readonly shown: readonly CardView[];
  /** The rest of `cards`, behind a "Show all" toggle; empty when every card is shown. */
  readonly more: readonly CardView[];
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
  /** The columns, in the order of `COLUMNS` (the page's stylesheet moves In progress first on a phone). */
  readonly columns: readonly ColumnView[];
  /** Date of the most recent card update, or null if the board is empty. */
  readonly lastUpdated: string | null;
}

/** How many cards the Done column shows before its "Show all" toggle: the most recent ones. */
export const DONE_SHOWN = 5;

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
 * The cards of one column in display order: board order, except in the Done column, where the most
 * recently finished come first (latest `updatedAt` first; on the same day, the later in the board first).
 */
function columnCards(board: Board, column: ColumnId): Card[] {
  const cards = board.cards.filter((card) => card.column === column);
  if (column !== "done") {
    return cards;
  }
  // Each date, most recent first, then that day's cards, the later in the board first. Dates are ISO
  // (YYYY-MM-DD), so the default sort (by code unit, never through a locale) orders them in time.
  const dates = [...new Set(cards.map((card) => card.updatedAt))].sort().reverse();
  return dates.flatMap((date) => cards.filter((card) => card.updatedAt === date).reverse());
}

/**
 * Prepares the display: cards grouped by column, in the classic kanban order of `COLUMNS`
 * (Later, Next, In progress, Done, from left to right), then in board order, except Done, most
 * recent first, of which only the {@link DONE_SHOWN} first are shown until "Show all". On a phone
 * the page's stylesheet moves In progress first, so that the work under way never comes after
 * the backlog.
 */
export function toView(board: Board): BoardView {
  const columns = COLUMNS.map((col) => {
    const cards = columnCards(board, col.id).map(toCardView);
    const limit = col.id === "done" ? DONE_SHOWN : cards.length;
    return { id: col.id, title: col.title, cards, shown: cards.slice(0, limit), more: cards.slice(limit) };
  });
  const dates = board.cards.map((card) => card.updatedAt).sort();
  return {
    title: board.title,
    columns,
    lastUpdated: dates.at(-1) ?? null,
  };
}
