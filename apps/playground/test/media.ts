// The system preferences jsdom lacks (`matchMedia`): a media query list the tests set.

/** A media query list whose answer a test sets, telling its listeners as a browser does. */
export class FakeMediaQuery extends EventTarget {
  /** Every query asked, in order; emptied before each test. */
  static readonly asked: string[] = [];
  /** Whether the system asks for what the query says: nothing, unless a test changes it. */
  matches = false;
  /** How many listeners of its changes it has. */
  listening = 0;

  /** Adds a listener, counting it. */
  override addEventListener(type: string, listener: EventListenerOrEventListenerObject | null): void {
    this.listening += 1;
    super.addEventListener(type, listener);
  }

  /** Removes a listener of changes, counting it; refuses any other. */
  override removeEventListener(type: string, listener: EventListenerOrEventListenerObject | null): void {
    if (type !== "change") {
      throw new Error(`no listener of ${type}`);
    }
    this.listening -= 1;
    super.removeEventListener(type, listener);
  }

  /** The system changes its preference: the list matches `matches` and tells its listeners. */
  become(matches: boolean): void {
    this.matches = matches;
    this.dispatchEvent(new Event("change"));
  }
}

/** The one media query list of a test: every query answers through it. */
export const media = { list: new FakeMediaQuery() };

/** `matchMedia` for jsdom: records the query, and answers the test's list. */
export function matchMedia(query: string): FakeMediaQuery {
  FakeMediaQuery.asked.push(query);
  return media.list;
}
