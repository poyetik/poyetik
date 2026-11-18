// Breaks the rule against formatting in the machine's locale: each formatter given no
// locale, `undefined` or the empty list, called and constructed; not one given a locale or
// a list of locales, nor a read of the locale itself (`resolvedOptions`), which a test
// emulating a locale asserts first.

/** A count to format. */
const count = 4096;

/** A date to format, of no clock. */
const day = new Date(0);

/** Formatted in the machine's locale: refused. */
export const unnamed: readonly unknown[] = [
  count.toLocaleString(),
  count.toLocaleString(undefined, { maximumFractionDigits: 0 }),
  day.toLocaleDateString(),
  day.toLocaleTimeString(),
  "i".toLocaleUpperCase(),
  "I".toLocaleLowerCase(),
  "a".localeCompare("b"),
  "a".localeCompare("b", undefined, { sensitivity: "base" }),
  new Intl.NumberFormat().format(count),
  Intl.NumberFormat(undefined, { maximumFractionDigits: 0 }).format(count),
  new Intl.DateTimeFormat().format(day),
  Intl.DateTimeFormat().format(day),
  new Intl.Collator().compare("a", "b"),
  new Intl.PluralRules().select(count),
  new Intl.RelativeTimeFormat().format(1, "day"),
  new Intl.ListFormat().format(["a", "b"]),
  new Intl.Segmenter().segment("a b"),
  new Intl.DisplayNames(undefined, { type: "region" }).of("FR"),
  new Intl.DurationFormat().format({ hours: 1 }),
  // The empty list of locales, which falls back to the machine's as no locale does.
  count.toLocaleString([]),
  day.toLocaleTimeString([], { hour: "2-digit" }),
  "a".localeCompare("b", []),
  new Intl.NumberFormat([]).format(count),
  Intl.DateTimeFormat([]).format(day),
];

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
