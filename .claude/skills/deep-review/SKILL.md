---
name: deep-review
description: Exhaustive multi-agent review of a change, a ticket or the whole repository, along every quality axis of the project (correctness, determinism, performance, security, tests, global coherence, consistency, UX/UI, accessibility, documentation, portability, simplicity, error handling, public API, audio DSP quality, scientific rigor and edge cases, dependencies and CI). Every finding is verified adversarially before it is reported. Use at the end of a review block (before its pull request), before closing a roadmap ticket or a release, or when the maintainer asks for a deep or full review.
argument-hint: "[scope: empty = changes since the last review point | a git range | a ticket like #1 | a path | all]"
disable-model-invocation: true
---

# Deep review

The project wants **maximum quality**: nothing may slip through. This review is deliberately heavy: many independent reviewers, each focused on one axis, then an adversarial verification of every finding. It reports; it never changes code until the maintainer has read the report and asked for fixes.

Answer the maintainer in the language they use. Everything written into the repository stays in English (`CLAUDE.md`).

## 1. Scope

Resolve the scope from the argument, then **state it in one line before starting**:

| Argument | Scope |
|---|---|
| empty | the commits since the last `review/*` git tag if any, else since the first commit of the current roadmap ticket in progress, plus uncommitted changes |
| a git range (`a..b`) | those commits |
| a ticket (`#1`) | every commit whose message or roadmap change belongs to that ticket (`git log`, `apps/roadmap/board.json` history), plus uncommitted changes |
| a path | every file under it, as it is now |
| `all` | the whole repository as it is now |

Build the **review packet** once, and give it to every reviewer:
- the list of files in scope, and the diff when the scope is a change;
- `CLAUDE.md` (the rules every finding is judged against);
- `spec/`, the documents `spec/README.md` lists: the engine's decisions and the step implementing each, the measurement definitions and how thresholds are derived, the benchmarks, and the formats;
- the local design notes listed in `CLAUDE.md` (section "Documentation"), when present, that are relevant to the scope; they are unversioned: never quote their strategy content in a versioned file;
- the result of the last `cargo xtask check --full` or green CI run if available (do not rerun it just for the review; a default `cargo xtask check` proves only the mutants of its changes).

## 2. Reviewers: one agent per axis, all in parallel

Launch every axis below as its own subagent, **all in a single message** so they run concurrently. Each one gets the review packet, its axis brief, and these common instructions:

> You review one axis only, with the project's maximum quality bar (`CLAUDE.md`). Read the code itself; do not trust comments, names or commit messages. Do not edit any file. You may run read-only commands (`git`, `rg`, `cargo test`, `pnpm test`, building in a temporary directory) to prove a point. For every finding give: file and line, severity (critical, major, minor, nit), the concrete failure scenario (inputs or actions → wrong result), the evidence (code quoted, command output), and a suggested fix. Report only what you can defend; say explicitly "nothing found" for an area you checked and found clean, and list what you checked, so that silence is never ambiguous. Findings about other axes are welcome at the end, marked as such.

Axis briefs:

1. **Correctness.** Logic errors, off-by-one, wrong conditions, unhandled cases, races, state machines with unreachable or missing transitions, wrong units, integer overflow, float pitfalls, resource leaks (audio contexts, workers, listeners, timers), promises never settled or rejected unhandled.
2. **Determinism and bit-exactness.** The core promise: the same samples, bit for bit, on every target. Anything that can make a render depend on the platform, the browser, the host buffer size, timing, iteration order, uninitialized memory, NaN payloads, `f32`/`f64` conversions, fused multiply-add, library math, the clock or randomness. Lookahead or rendering that depends on how the host splits time.
3. **Performance and real-time safety.** Allocation, locks, logging or unbounded work on the audio thread; per-block or per-frame garbage in JavaScript; wasted copies; algorithmic complexity; React re-renders on hot paths; canvas work per frame; WASM size and start-up time; build and test time regressions.
4. **Security.** Untrusted input (patch files, imported files, WAV files, worklet messages, URLs, future network and bridge messages) parsed anywhere but at a validated boundary; `unsafe` and FFI soundness (pointer validity across calls, memory growth, aliasing); injection (HTML, URLs); CSP; secrets; supply chain (new dependencies, lockfile changes, install scripts, licenses).
5. **Test quality and useless-test detection.** Tests that cannot fail, assert nothing meaningful, duplicate another test, test the mock instead of the code, depend on order, time or the network, or pass for the wrong reason. Missing cases: edge values, error paths, property tests the policy requires. Fakes or casts that hide a real bug. Assertions weaker than they could be. Coverage obtained without real verification. Mutants killed only by accident. Reviewed unviable mutants (`mutants-unviable.txt`) whose reason is wrong.
6. **Global coherence.** Is the change consistent with the whole system, not only with itself? Contracts between parts (patch format, engine, bindings, apps, conformance, tooling) respected and documented on both sides; a decision made here contradicting one made elsewhere; duplicated concepts or logic across parts; layering violations (a part using another's internals); the local architecture notes, `CLAUDE.md`, READMEs, roadmap and changelog still describing what the code does; anything the change should have updated elsewhere and did not.
7. **Consistency and conventions.** Naming, vocabulary and error message style across Rust and TypeScript; file and module organization; English everywhere; the rules of `CLAUDE.md` (no type assertions, no lint suppressions without reason, no inline `eslint-disable`, documented items, Conventional Commits, changelog entries, one version everywhere, no reference to the local design notes outside `CLAUDE.md`, no reference to other patching software outside the import feature).
8. **UX and UI.** Clarity of the interface, states (loading, error, empty, busy), feedback for every action, wording, visual consistency, responsive layout, behavior on mobile, sound safety (volume, sudden starts), first-use experience. Check the live page with Claude in Chrome when a web interface is in scope, reading text and measurements.
9. **Accessibility.** Roles, names, keyboard access to everything, focus order and visibility, contrast, reduced motion, screen-reader announcements, the accessibility layer the canvas requires (interface test strategy, in the local design notes listed in `CLAUDE.md`), axe coverage of every state.
10. **Documentation.** Rustdoc and JSDoc that are wrong, stale, vague or missing the why; `# Errors` sections; READMEs and commands that no longer work (run them); changelog entries that do not match the change.
11. **Portability and cross-environment behavior.** Code that only works on one OS, one browser, one sample rate, one CPU architecture or one locale; assumptions about the audio device; Node versus browser differences; CI (Linux) versus local (macOS) differences; paths and line endings.
12. **Simplicity and maintainability.** Over-engineering, dead code, needless abstraction, unclear names, functions doing too much, comments that should be code, code that should be data, anything a newcomer would misread.
13. **Error handling and observability.** Errors swallowed, turned into silence, or reported without enough context; failures in the audio thread that nobody sees; error messages that do not say what to do; exit codes; logs.
14. **Public API and developer experience.** Exported interfaces of the crates and packages (engine, bindings, future SDKs): naming, stability, misuse resistance (impossible states unrepresentable), semantic versioning impact of the change, documentation of every public item.
15. **Audio DSP quality.** Judged against the state of the art, with numbers whenever possible: aliasing (band-limited oscillators, oversampled non-linearities), parameter changes (smoothing, no zipper noise, sample-accurate events), filter topologies stable under modulation (TPT / zero-delay feedback), numerical stability and precision (phase accumulators, coefficient computation, `f32` versus `f64`, catastrophic cancellation), subnormal numbers (a deterministic strategy: no hardware flush-to-zero in WebAssembly), DC offset, gain staging and headroom, latency, interpolation and resampling quality, cost per sample (approximations measured against references). Does every box have objective measurements (spectrum, THD, SNR, aliasing level, frequency response) in its tests, and are they good enough?
16. **Scientific rigor and edge cases.** Every claim treated as a theorem to prove or refute. Every documented invariant (phase within one cycle, no NaN, bounded output, error below a tolerance) checked at the extremes of its domain and their combinations: the minimum and maximum of every parameter crossed with the minimum and maximum sample rate, zero, negative values, Nyquist and above, subnormal numbers, the largest sizes accepted (frames, boxes, cables, channels), empty inputs. Long runs: drift and precision loss after hours of rendering, accumulator and counter overflow. Numerical method: precision of `f32` versus `f64`, double rounding, catastrophic cancellation, error accumulation, and whether every tolerance and threshold has a derivation (an error bound worked out, or a measured distribution) rather than a guess. Validity of every measurement method (frequency estimation, spectra, error metrics): its own bias and failure modes. Worst-case cost: pathological inputs that make an algorithm quadratic or unbounded. Derive bounds on paper, then confirm them with experiments; report numbers.
17. **Dependencies, build and CI.** Every new dependency justified in the dependency register (`dependencies.txt`, its strategic reasons in the local design notes listed in `CLAUDE.md`), bit-defining ones pinned exactly, minimal and mainstream; versions pinned; toolchains pinned; the full check (`cargo xtask check --full`) run by CI, and what the pre-push hook's default check leaves to it (mutants outside the changes, TypeScript results Stryker reuses); reproducibility of builds; anything in `cargo xtask check` that could pass while something is broken (a step that cannot fail, a gate reading the wrong file).

If the scope plainly cannot touch an axis (for example UX for a Rust-only change), still run it with the instruction to confirm that nothing in scope reaches it: coherence across parts is exactly what single-axis thinking misses.

### Standing checks

Defect classes earlier reviews found, which only a review sees. Give each reviewer the checks of its axes (in brackets), in addition to its brief; each check is reported done, with what it found, like any other area.

1. **Inherited environment** (1, 11, 17). For each process spawned by tests, hooks and tools, list the environment it inherits from each caller: an interactive shell, a git hook run from the main checkout and from a linked worktree, CI. For each workflow `CLAUDE.md` prescribes (worktrees, releases, hooks, tags), find the test that runs it end to end, or report that none does.
2. **Absolute claims** (6, 10, 17). For every absolute claim ("checked", "enforced", "never", "always", "exactly", "must", "already validated", "now does") in `CLAUDE.md`, the READMEs, doc comments and the changelog, open the check that enforces it and compare its conditions with the claim's wording. A `[Unreleased]` changelog entry must hold at the reviewed commit, not only at the commit that added it.
3. **Waits that never end** (1, 13). For each `await` on an external event (a browser callback, a message, a fetch, a child process), say what settles it if the event never comes. For each memoised promise, say who clears it when it fails.
4. **Numbers recomputed** (16). Recompute every documented bound, threshold and numeric claim exactly, at the extremes of its inputs, and check there is one test per regime of the formula.
5. **Gates attacked** (5, 17). For each gate, try the forms an honest change can bring that it may not expect: odd whitespace or flow form, a comment, an unexpected type or shape, what a tool, an editor or a machine's configuration writes, and a file it should read but does not. List its early successful returns and its allowlists, and justify each. For each rule backed by a tool, probe one case the tool exempts and compare with the written rule. A form only deliberate evasion would write is a finding only when it can change what the engine computes at run time, or when a doc or claim says the gate covers it (the finding is then that claim); otherwise check that the gate's doc states it out of scope (`CLAUDE.md`, "A gate guards against honest mistakes"). Tag each tooling finding honest or evasion, with the reason in one phrase.
6. **Renames** (6, 7). After a rename, search the new name in every case and form, and the old name in its encoded forms too (hexadecimal, ASCII codes, identities, test values), in local branches and refs, and in the local design notes listed in `CLAUDE.md`.
7. **Environment dimensions** (5, 11). For each test loop over a dimension of the environment, check that the value reaches the code under test and that the assertion would differ at another value. For each `is_err()`, `toBe(true)`, error-kind or "throws" assertion, ask whether the exact value is known and should be asserted. Check third-party default configurations for reads of the environment.
8. **Glossary against code** (6, 7, 10). Read every glossary row against the code it names: the definition, not only the name. Check every roadmap ticket number cited in a versioned file with `pnpm roadmap show <id>`.
9. **Markers and magic values** (1, 2). For every marker, sentinel or magic value, ask which wrong state also produces it.
10. **Fixes proven before endorsed** (5, and the analysts). A proposed fix is run against the normal path of the existing tests before it is recommended, and states how it keeps 100% coverage and no surviving mutant. A proof names the layer it observes. A length or count claimed about data comes with the script that measured it.
11. **Accessibility rules** (9). List every CSS rule or attribute that matters to accessibility with the test that fails without it.
12. **Copies and loops** (6, 12, 3). Search each pinned value and each list entry and count where it is typed: one source, or a gate tying the copies. Search for test helpers written more than once. Give every animation-frame or polling loop a stop condition. Read every card of the roadmap's `doing` column as a visitor would.
13. **Performance and bits** (3, 2). A performance finding comes with the measurement to take, and is never applied unmeasured. A fix that changes rendered bits only in cases the corpus does not reach yet is scheduled while it is still hash-neutral, before the change (a new box, a new case) that would make it move the reference hashes.
14. **Hardening options** (4, 17). At each review, compare the configuration with the current hardening options of the forge (GitHub Actions), the package managers (pnpm, Cargo) and the supply-chain tools (`cargo-deny`).
15. **Spec and measurements** (6, 15, 16). Every engine change matches the decision of `spec/engine-design.md` it implements, at the step that decision names. Every box and every engine component that touches the signal (resampler, smoothing, limiter, compensation, crossfades, event timing) lands its measurements with derived thresholds (`spec/measurements.md`), and every threshold has its derivation next to it: recompute it, and check one test per regime.
16. **Previous fixes closed** (5, 6, 10). For each finding the previous review of the same scope accepted, find its commit and the test or search that shows every place it named changed. A fix that landed in part, or a test it promised that was never written, is a finding of this review.

## 3. Verification: every finding is challenged

1. Merge the reports, and deduplicate findings that describe the same defect (keep the clearest scenario, note every axis that found it).
2. For each remaining finding (batch small ones by file), launch a **verifier** subagent, all in a single message, with the finding and the code, and this brief:

> Try to prove this finding wrong. Read the code and its tests, and run what you need (read-only). Answer CONFIRMED with the evidence that it is real, PLAUSIBLE when it is real only under conditions you can state, or REFUTED with the evidence that it is not. Do not soften a real problem; do not keep a wrong one.

3. Drop refuted findings, but list them briefly at the end with the reason, so that the maintainer can disagree.

## 4. Analysis: is each finding admissible, and at what cost?

A confirmed defect is not automatically worth fixing as suggested. For every CONFIRMED or PLAUSIBLE finding, launch an **analyst** subagent (all in a single message) with the finding, its verification, the code and the review packet. Findings that share a cause or a fix form a cluster analyzed by one analyst, so that their links and trade-offs are seen together; unrelated findings are never batched, and every finding of a cluster still gets its own complete analysis. The brief:

> Decide whether this finding is admissible, and how it should be addressed, with a thorough analysis:
> - **Admissibility**: is it a real problem for this project, given its goals and rules (`CLAUDE.md`)? Or a matter of taste, a deliberate documented decision, or something the project explicitly accepts? Quote the rule or decision that settles it.
> - **Impact**: who or what is affected (the sound, the user, a contributor, CI, security), how likely, how severe, and what happens if it is left as is.
> - **Options**: at least two ways to address it (including the suggested fix, and doing nothing or deferring), each with what it changes, what it costs (code, tests, complexity, performance, dependencies) and what it risks breaking elsewhere in the system.
> - **Trade-offs**: what each option gives up. Name the tension explicitly (for example proof versus simplicity, performance versus readability, scope of the ticket versus the cleanest fix).
> - **Proof**: for the recommended option, the failing test that will prove the problem and then its fix, and the checks that will show nothing else broke.
> - **Recommendation**: apply as suggested, apply differently (say how), defer (say to which ticket or condition), or reject (say why), with a confidence level.

## 5. Lessons: turn classes of defects into rules

A fixed defect is gone; a rule keeps its whole class from coming back. For the confirmed findings taken together, find what they reveal about the project's methods: a gap in the testing strategy (edges never tested, a kind of input never generated, a claim never measured), a check that passes vacuously, a promise of `CLAUDE.md` that no tool enforces, a recurring kind of mistake. For each lesson, propose in this order of preference (the guiding principle of `CLAUDE.md`):
1. **a tool** that makes the mistake impossible or detects it: a step or gate in `cargo xtask check`, a law in `audio-testkit`, a lint, a type;
2. **a rule in `CLAUDE.md`**, worded so that it applies beyond the case that revealed it;
3. **a check in this skill**, when only a review can see it.

Each lesson names the findings that revealed it.

## 6. Report

Present, in this order:
1. **Verdict**: one paragraph. Is the scope at the maximum quality bar? What blocks it?
2. **Findings**, most severe first, each with: severity, axes, `file:line`, the failure scenario, the evidence, CONFIRMED or PLAUSIBLE, then its **analysis** (admissibility, impact, options, trade-offs, recommendation and confidence).
3. **Coverage of the review**: for each axis, what was checked and found clean, so that every "nothing found" is explicit.
4. **Refuted findings**, one line each with the reason.
5. **Lessons**: the rules, tools and review checks proposed in section 5.
6. **Proposed plan**: the recommended fixes grouped into commits, each with the test that will prove it; the deferred findings with where they go; the rejected ones with why.

Then **stop**. Nothing is applied without the maintainer's decision, finding by finding.

## 7. Applying a fix: analyze again first

Never apply a fix on the strength of the review alone. For each finding the maintainer accepts, at the moment of applying it:
1. **Re-analyze it** against the code as it is now (other fixes may have changed it): is it still real, is the chosen option still the best, did a trade-off move? If the answer changed, stop and tell the maintainer before doing anything.
2. Write the failing test first (`CLAUDE.md`: every fix starts with a test that reproduces the problem).
3. Apply the fix, then run the checks it calls for: for a review block's review, targeted checks (`CLAUDE.md`, Working in blocks), the block's full check being its pull request's CI run; otherwise `cargo xtask check --full` (or a green CI run) until it is green.
4. Commit, one logical fix per commit, the message saying which finding it closes; list in the review's report, for each finding closed, its commit and the test or search proving every place it named changed, so that the next review can check it (standing check 16).

Accepted lessons are applied the same way: the tool or rule first, with a test showing it catches the defects that revealed it.

When every accepted fix is in, propose to mark the review point with a git tag `review/<YYYY-MM-DD>`, the default start of the next review.

## Cost

This review launches many agents: seventeen reviewers, a verifier per finding (or per file), an analyst per confirmed finding, and a new analysis for each fix applied. It is meant for the end of a ticket, before a release, or on request; not for every small change.
