// Breaks no rule: the formatters given a locale or a list of locales, and the reads of the
// locale itself (`resolvedOptions`), which a test emulating a locale asserts first, that the
// rule against formatting in the machine's locale allows (what it refuses: `locale.ts`).

/** A count to format. */
const count = 4096;

/** A date to format, of no clock. */
const day = new Date(0);

/** Formatted in a locale given, or the locale read: allowed. */
export const named: readonly unknown[] = [
  count.toLocaleString("en-US"),
  count.toLocaleString(["en-US"]),
  day.toLocaleDateString("en-US"),
  "i".toLocaleUpperCase("en-US"),
  "a".localeCompare("b", "en-US"),
  new Intl.NumberFormat("en-US").format(count),
  Intl.DateTimeFormat().resolvedOptions().locale,
  new Intl.DateTimeFormat().resolvedOptions().timeZone,
];
