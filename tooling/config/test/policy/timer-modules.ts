/** Code under test may import a timer: it is no test's sleep. */
import { setTimeout as wait } from "node:timers/promises";

/** The timer it binds. */
export const WAIT: unknown = wait;
