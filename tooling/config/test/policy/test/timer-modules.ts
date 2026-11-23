/** Sleeps in test code by Node's timer modules, each by either of its names: one refused import each. */
import { setInterval as every } from "node:timers";
import { setTimeout as pause } from "node:timers/promises";
import { setImmediate } from "timers";
import { setTimeout as wait } from "timers/promises";

/** The timers they bind. */
export const TIMERS: readonly unknown[] = [every, pause, setImmediate, wait];
