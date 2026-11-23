// What failed, in words: every value a page may catch, bounded and never throwing.
import { describe, expect, test } from "vitest";
import { MORE_CAUSES, MOST_CAUSES, UNWRITABLE, failure } from "../src/failure.ts";
import { random } from "./random.ts";

/** A proxy revoked at once: every conversion of it, `instanceof` included, throws. */
function revoked(): object {
  const { proxy, revoke } = Proxy.revocable({}, {});
  revoke();
  return proxy;
}

/** An object without a prototype that holds itself: neither JSON nor `String` can write it. */
function bare(): object {
  const value: { self?: unknown } = {};
  Reflect.setPrototypeOf(value, null);
  value.self = value;
  return value;
}

/**
 * An arbitrary value drawn from `next`, `depth` levels deep at most: every kind a page may
 * catch (primitives of every type, errors with causes, plain, cyclic, prototype-less or
 * revoked objects, arrays, symbols, functions, big integers), some holding one another,
 * some their own parents.
 */
function arbitrary(next: () => number, depth: number, parents: readonly object[] = []): unknown {
  const leaves: readonly (() => unknown)[] = [
    () => undefined,
    () => null,
    () => next() < 0.5,
    () => next() * 2 - 1,
    () => [Number.NaN, Infinity, -Infinity, -0][Math.floor(next() * 4)],
    () => BigInt(Math.floor(next() * 1000)),
    () => Symbol(String(next())),
    () => "words",
    () => () => undefined,
    revoked,
    bare,
    () => parents[Math.floor(next() * parents.length)],
  ];
  const nodes: readonly (() => unknown)[] = [
    () => {
      const error = new Error(String(next()));
      error.cause = arbitrary(next, depth - 1, [...parents, error]);
      return error;
    },
    () => {
      const holder: { inside?: unknown } = {};
      holder.inside = arbitrary(next, depth - 1, [...parents, holder]);
      return holder;
    },
    () => {
      const list: unknown[] = [];
      list.push(arbitrary(next, depth - 1, [...parents, list]));
      return list;
    },
  ];
  const choices = depth > 0 ? [...leaves, ...nodes] : leaves;
  return choices[Math.floor(next() * choices.length)]?.();
}

describe("failure", () => {
  test("words what failed with every cause it carries", () => {
    const download = new TypeError("Failed to fetch");
    const wrapped = new Error("cannot load the square", { cause: new Error("the module", { cause: download }) });
    expect(failure(wrapped)).toBe("Error: cannot load the square, caused by Error: the module, caused by TypeError: Failed to fetch");
    expect([failure("plain"), failure({ code: 1 }), failure(undefined), failure(null)]).toStrictEqual(["plain", '{"code":1}', "undefined", "null"]);
    expect(failure(new Error("no square", { cause: "offline" })), "a cause that is no error, the last").toBe("Error: no square, caused by offline");
  });

  test("words once a cause the error's own words already end with", () => {
    const compile = new Error("no magic", { cause: "offset 0" });
    expect(failure(new Error(`cannot be compiled: ${String(compile)}`, { cause: compile })), "its words say it, its own cause after").toBe("Error: cannot be compiled: Error: no magic, caused by offset 0");
    expect(failure(new Error("Error: no magic in it", { cause: new Error("no magic") })), "words that hold it, not at their end").toBe("Error: Error: no magic in it, caused by Error: no magic");
    expect(failure(new Error("first", { cause: new Error("second, caused by Error: second", { cause: new Error("second") }) })), "a cause worded after its error").toBe("Error: first, caused by Error: second, caused by Error: second");
  });

  test("words a browser's own exception, a cause or an error, as its name and message", () => {
    const refused = new DOMException("no device", "NotSupportedError");
    expect(failure(refused)).toBe("NotSupportedError: no device");
    expect(failure(new Error("no sound", { cause: refused }))).toBe("Error: no sound, caused by NotSupportedError: no device");
    Object.defineProperty(refused, "cause", { value: "unplugged" });
    expect(failure(refused), "its own cause").toBe("NotSupportedError: no device, caused by unplugged");
  });

  test("ends at a cause met again: an error its own cause, or two causing each other", () => {
    const itself = new Error("itself");
    itself.cause = itself;
    expect(failure(itself)).toBe("Error: itself");
    const first = new Error("first");
    const second = new Error("second", { cause: first });
    first.cause = second;
    expect(failure(first)).toBe("Error: first, caused by Error: second");
  });

  test("words what JSON cannot write, and what nothing can", () => {
    const cyclic: { self?: unknown } = {};
    cyclic.self = cyclic;
    expect(failure(1n), "a big integer").toBe("1");
    expect(failure(cyclic), "a cycle").toBe("[object Object]");
    expect(failure(Symbol("s")), "a symbol").toBe("Symbol(s)");
    expect(failure((): number => 0), "a function").toBe("() => 0");
    expect(failure(bare()), "no prototype").toBe("[object Object]");
    expect(failure(revoked()), "a revoked proxy").toBe(UNWRITABLE);
    expect(failure(new Error("no square", { cause: 2n })), "a big integer as a cause").toBe("Error: no square, caused by 2");
    expect(UNWRITABLE).toBe("a value that cannot be written");
  });

  test("words an error whose own words throw, or whose cause cannot be read", () => {
    const silent = new Error("hidden");
    Object.defineProperty(silent, "message", {
      get: () => {
        throw new Error("no message");
      },
    });
    // Its own words throw: JSON's, which has none of its fields to write.
    expect(failure(silent)).toBe("{}");
    const guarded = new Error("guarded");
    Object.defineProperty(guarded, "cause", {
      get: () => {
        throw new Error("no cause");
      },
    });
    expect(failure(guarded)).toBe("Error: guarded");
  });

  test("words at most 16 causes, then says there are more: a chain of causes without end", () => {
    // What revealed the bound: a cause getter that makes a new error at every read, never met
    // again, kept the words growing forever (third deep review of visual block 2).
    let made = 0;
    /** An error whose cause is a new such error at every read. */
    const endless = (): Error => {
      made += 1;
      const error = new Error(`error ${String(made)}`);
      Object.defineProperty(error, "cause", { get: endless });
      return error;
    };
    const words = failure(endless());
    expect(words).toBe([...Array.from({ length: 17 }, (_unused, at) => (at === 0 ? "Error: error 1" : `caused by Error: error ${String(at + 1)}`)), "and more causes"].join(", "));
    expect([MOST_CAUSES, MORE_CAUSES]).toStrictEqual([16, "and more causes"]);
    /** A chain of `causes` causes after the error itself. */
    const chain = (causes: number): Error => {
      let error = new Error(`cause ${String(causes)}`);
      for (let at = causes - 1; at >= 0; at -= 1) {
        error = new Error(at === 0 ? "the error" : `cause ${String(at)}`, { cause: error });
      }
      return error;
    };
    /** The words of a chain of `causes` causes, each worded. */
    const worded = (causes: number): string => ["Error: the error", ...Array.from({ length: causes }, (_unused, at) => `caused by Error: cause ${String(at + 1)}`)].join(", ");
    expect(failure(chain(16)), "16 causes, every one worded").toBe(worded(16));
    expect(failure(chain(17)), "17 causes, the 17th untold").toBe(`${worded(16)}, and more causes`);
  });

  test("words any value, bounded and never throwing (property, 2000 seeded values)", () => {
    for (let seed = 0; seed < 2000; seed += 1) {
      const value = arbitrary(random(seed), 4);
      expect(typeof failure(value), `seed ${String(seed)}`).toBe("string");
    }
  });
});
