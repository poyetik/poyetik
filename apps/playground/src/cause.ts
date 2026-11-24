/**
 * Whose a failure is, by the remedy it takes, one type for every failure the page explains
 * (the square's, `SquareFailure`; the GPU's refusals, `Refusal`; the sound's, `Failure`), so
 * that a remedy is chosen by cause, never by the stage that failed, which can hold several:
 * `network`, the page's files did not reach the browser (try again, then check the
 * connection); `environment`, this browser or machine cannot do it (another browser, another
 * device, nothing the page can fix); `defect`, the page's own, the same in every browser (try
 * again, then its details say what failed).
 */
export type Cause = "network" | "environment" | "defect";
