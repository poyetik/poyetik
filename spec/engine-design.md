# Engine design decisions

The decisions the engine is built on, recorded before any code that depends on them (roadmap ticket #12, step 1). Each one gives the decision, why, the alternatives rejected, how it meets or exceeds the state of the art of professional audio workstations and visual patching environments, its consequences (engine, patch format, render hash, tests) and the ticket step that implements it. Each decision gives its place in the [implementation order](#implementation-order) at the end, which says what comes first.

This document states what the engine **will** do. What it does today is specified by [`patch-format.md`](patch-format.md) and [`render-hash.md`](render-hash.md); each of those changes, in the step that implements a decision, with a new format or hash version where the bytes change.

## How the choices were made

1. Maximum quality, proof and flexibility.
2. Where two good options conflict, both are offered as a setting; the default is what the most demanding professional tools offer.
3. Where only one can exist, the one that matches or exceeds the state of the art.

The engine must power both a full audio workstation (tracks, sample-accurate automation, latency compensation, transport) and a visual patching environment with fine control of block sizes, inputs and outputs. Nothing below may weaken the core promise: **the same samples, bit for bit, on every target**.

## Words

Words this document introduces, to enter `GLOSSARY.md` with the step that gives them a name in the format or the code:

| Word | Meaning |
|---|---|
| **host** | What runs the engine and chooses its settings: the browser's AudioWorklet, a native audio backend, a desktop shell, a plugin wrapper, an offline renderer |
| **engine block** | The number of frames the engine processes per internal pass, chosen by the host; a performance setting only |
| **subpatch** | A patch placed as a box inside another, with its own inlets and outlets, and optionally its own block size, resampling and overlap |
| **event** | Something that happens at one exact frame: a message, a parameter change, a MIDI packet, a transport change |
| **message** | A value sent along a cable at one frame (a number, a list, a symbol), as opposed to a signal |
| **signal** | A value at every frame, carried by a cable as one or more channels |
| **voice** | One instance of a polyphonic subpatch |
| **bus** | A named, multichannel signal path that several boxes send to and others read from |

---

## 1. Host independence

**Decision.** The engine is a library. Its host chooses the sample rate, the host buffer (the device buffer of an audio backend is one), the engine block and the input and output channel counts. The browser's AudioWorklet is one host among others: a native audio backend, a desktop shell embedding the native engine, the engine packaged as a VST3, CLAP or AU plugin, the offline renderer. A host states what it asks for; the engine answers with what it applies. **A setting a host cannot honor is reported, never faked**: the AudioWorklet cannot choose its render quantum (128 frames) nor guarantee the sample rate asked of its context, so it reports both as imposed.

**Why.** One engine for every product; a fixed browser assumption would make the native and plugin hosts second-class.

**Rejected.** An engine shaped around the AudioWorklet (fixed 128 frames): it would forbid small buffers natively and large ones offline. Silently substituting a setting: a user tuning latency must see what is really applied.

**State of the art.** Professional tools let the user choose the device buffer and the internal processing size separately; this does the same on every host, and adds an explicit report of what a host could not honor.

**Consequences.** Engine: a `HostSettings` request and an `AppliedSettings` answer, with one reason per refused setting. Patch format: unchanged (host settings are not part of a patch). Hash: unchanged (the rate and channel count already are). Tests: every setting across the test matrix, and a law that the applied settings equal the request whenever the host can honor it.

**Step.** #12 step 13. **Order:** 5.

## 2. Three levels of block size

**Decision.** Three independent levels:

1. the **host buffer**: whatever the host delivers (any length, varying from call to call);
2. the **engine block**: chosen by the host, from 1 frame to a documented maximum, any integer, not only powers of two;
3. the **subpatch block**: set in the patch, per subpatch, with an integer up- or downsampling factor and an overlap factor (for spectral processing); a block of 1 frame is allowed.

The first two are **performance settings only: a render never depends on them.** The third is part of the patch's meaning (a spectral subpatch with a 4096-frame window and four-fold overlap sounds like that wherever it runs), so it is stored in the patch.

Sub-decisions:

- **Renders are independent of the engine block.** Every box processes sample-accurately; a box that saves work by updating derived values (coefficients) less than every frame does it on a grid of **absolute frame numbers**, never on engine block boundaries. A law checks the bits of every conformance case across the engine block sizes of the test matrix, 1 included.
- **A subpatch without a declared block has none**: it is processed in whatever spans the engine chooses, and its result is the same.
- **A subpatch with a declared block `B`, overlap `O` and factor `R`** collects `B` frames of its inputs at its own rate (`R` times the outer rate, or `1/R` of it), runs every `B/O` frames, and overlap-adds its outputs. It introduces a latency the engine derives from `B`, `O` and the resampling filters, declares and compensates (decision 13). The resampling filters are linear-phase, specified by their coefficient tables (identical on every target), and measured (passband ripple, stopband rejection, aliasing) like any box.
- Internal rates of a resampled subpatch may exceed the host's supported range; the type of an internal rate is wider than `SampleRate`, and the limits of the factor are named constants, tested at the limit and one past it.

**Why.** The engine block trades CPU (fewer calls, better vectorization) against latency and granularity; making it semantic would make the same patch sound different on two hosts. The subpatch block is the opposite: a musical choice, which must travel with the patch.

**Rejected.** A fixed engine block (today's 64 frames): it would fix the latency floor and the cost of every host. A single engine-wide block chosen by the patch: the host could no longer pick its latency. Power-of-two engine blocks only: an odd host buffer (a plugin host asking 441 frames) is then rebuffered for nothing.

**State of the art.** Patching environments offer a per-subpatch block size with overlap and resampling, and a global vector size that changes results; workstations offer a buffer size. This offers all three levels, a block of 1 frame anywhere, and adds what neither guarantees: the engine block never changes a single bit.

**Consequences.** Engine: `BLOCK_SIZE` stops being a constant of the core; `Block` becomes a span of a host-chosen length; the plan cuts spans at events and at the grid of absolute frames. Patch format: subpatches, with optional `block`, `overlap` and `resample` (format version 2 or later, ticket #3). Hash: unchanged. Tests: the engine-block law over every case; subpatch blocks across the matrix (1, a prime, a power of two, the maximum); resampling filters measured; a spectral round trip (analysis, synthesis) exact to a derived bound.

**Step.** Engine block: #12 step 13. Subpatch blocks: #12 step 17. **Order:** 5 for the engine block, 10 for subpatch blocks.

## 3. Feedback

**Decision.** A cycle is allowed only through an **explicit delay box**, whose delay is at least one frame. Two ways to get single-sample feedback, both offered:

- **automatic**: in a region without a declared block, the planner processes a cycle (its strongly connected component) in spans no longer than the shortest delay in it; a delay of 1 frame runs that component frame by frame, and nothing else in the patch pays for it;
- **explicit**: a subpatch with a block of 1, for a user who wants the boundary visible.

Inside a subpatch with a declared block `B`, a cycle's delay must be at least `B` (the planner rejects a shorter one, naming the delay box and suggesting a block of 1).

**Why.** A delay-free cycle has no defined value; an implicit one-block delay (the usual fallback) makes a filter's tuning depend on the block size, which decision 2 forbids.

**Rejected.** An implicit block of delay in every cycle: the result depends on the block size. Running the whole patch frame by frame when any cycle exists: correct but needlessly costly. Implicit solving of delay-free loops (zero-delay feedback across boxes): a numerical solver in the planner, with convergence that cannot be proven bit for bit; zero-delay feedback belongs inside a box (filter topologies), where it is exact.

**State of the art.** Patching environments require a delay of at least one block in a cycle, or a dedicated single-sample subpatch. This gives single-sample feedback anywhere, automatically, at the cost of only the cycle.

**Consequences.** Engine: strongly connected components in the planner, per-component span limits. Patch format: today's rule "a cycle without delay is rejected" keeps its meaning; a cycle with a delay box becomes valid. Hash: unchanged. Tests: a one-frame feedback comb filter checked against an exact reference across engine blocks; a cycle shorter than its subpatch block rejected; the existing cycle rejection kept.

**Step.** #12 step 17. **Order:** 10.

## 4. Events and messages

**Decision.** Everything that is not a signal is an **event stamped with an absolute frame**: a message, a parameter change, a MIDI packet, a transport change, a plan swap. MIDI input arrives as events: **MIDI 2.0 Universal MIDI Packets** inside the engine (MIDI 1.0 bytes converted at the host boundary), so 32-bit controller resolution and per-note controllers are carried without loss.

Sub-decisions:

- **Timestamps are absolute frame numbers** (64-bit, counted from the start of the render), so an event's effect never depends on how time is split. A host gives timestamps relative to its buffer; the engine converts them.
- **Late events** (a live event whose frame has passed) are applied at the first frame still to be processed and **counted as late** in the engine's report. Offline renders have no late events and stay deterministic.
- **Queues are bounded and preallocated**; an overflow is reported, never allocated around. Their capacities are named constants.
- **Message cables are edges of the graph** like signal cables: a message always flows forward in execution order, and a cycle of messages needs an explicit delay (at least one frame), like any cycle. A box that iterates (a counter loop) does it inside itself. So a message cascade is finite by construction.
- **Order of messages within one frame** (a total order, independent of thread count and of the document's order):
  1. events from the host, in the order the host gave them;
  2. then messages produced by boxes, **depth first**: when a box sends a message, it reaches every destination, and everything those destinations send in turn, before the box's next message;
  3. one outlet feeding several inlets delivers in the order of the destinations' box ids, then inlet indices: the order cables into one inlet are already summed in;
  4. a message arriving at frame `t` takes effect before the receiving box computes its sample at `t`.

**Why.** Sample-accurate events are what makes automation, sequencing and MIDI exact; absolute frames make them independent of the host; a forward-only message graph makes ordering deterministic and parallel-safe. Depth first is the order a user can predict from the cables.

**Rejected.** Events quantized to the block (a block's worth of jitter, and results that depend on block size). A separate, lower-priority message scheduler outside the audio thread with its own clock: messages would drift against audio. Breadth-first delivery: the order of side effects stops following the cables. Unbounded message loops within a frame: they can hang the audio thread. Ordering fan-out by creation order or position on screen: a hidden dependency on the editor's state.

**State of the art.** Workstations offer sample-accurate automation and MIDI; patching environments offer deterministic depth-first message ordering, but usually at block resolution unless a scheduler option is set. Here every message is sample-accurate, always, with an order that is total and documented, and MIDI is carried at MIDI 2.0 resolution.

**Consequences.** Engine: an event type, per-box event spans, a preallocated queue per plan. Patch format: message cables (distinguished from signal cables by the ports they join). Hash: unchanged; a conformance case may carry an event list. Tests: ordering laws (same frame, fan-out, depth first) over random graphs; a render with events identical across host buffers, engine blocks and thread counts; late and overflowing events reported.

**Step.** #12 steps 4 and 14. **Order:** 6.

## 5. Rates: every parameter takes messages or signals

**Decision.** Every parameter is also an inlet. It accepts **messages** (a value, or a value with a ramp time) and **signals** (one value per frame). How a connected signal combines with the value set by messages is a **setting per parameter inlet**: `add` (the signal modulates around the value, the default) or `replace` (the signal is the value).

Sub-decisions:

- **Input domain.** Every box accepts any value at every inlet and parameter (0, -0, subnormals, NaN, infinities, huge values) without panicking, and with the same bits on every target. At the parameter boundary, after combination: a non-finite value holds the previous valid value; a finite value outside the range is clamped to it; a subnormal is zero (decision 11). Signal inlets take any value; a box whose recursive state becomes non-finite resets that state to zero and reports it, so that one bad sample cannot silence a box for ever.
- **Smoothing.** Each parameter declares a **kind** (continuous, stepped, trigger) and continuous ones a **smoothing curve**: linear in the parameter's own unit by default, exponential for frequency-like and gain-like kinds (constant ratio per frame, so a sweep is even in pitch and in decibels). A message change ramps over a duration declared per parameter kind and overridable per box and per message; the ramp is a whole number of frames and lands exactly on the target value at its last frame. A signal is never smoothed (it is already exact). Stepped and trigger parameters change at their frame, without a ramp.

**Why.** Any parameter driven at audio rate is what modular synthesis needs; a smoothed message change is what automation needs. The input domain is how a patch stays safe when a user wires anything to anything.

**Rejected.** Separate "control" and "audio" variants of each parameter: twice the boxes, and a user must know which. One-pole smoothing: it never reaches the target exactly and its end depends on rounding. A single global smoothing time: too slow for a gate, too fast for a filter cutoff. Rejecting runtime values out of range: impossible for a signal, which cannot be refused.

**State of the art.** Workstations smooth automation; modular environments take signals on most parameters. This offers both on every parameter, with a choice of combination, ramps that end exactly, and a documented input domain checked by laws on every box.

**Consequences.** Engine: `ParamSpec` gains a kind, a smoothing curve and a default ramp; parameters become inlets. Patch format: the combination setting per parameter inlet; message cables into parameters. Hash: unchanged. Tests: box laws over the special values of the matrix at every inlet and parameter; zipper-noise measurements per parameter kind (step 2's measurements); a ramp's last frame exactly equal to its target.

**Step.** #12 steps 4 and 14. **Order:** 6.

## 6. Multichannel

**Decision.** **A cable carries N channels**, N fixed when the plan is built (from 0 to a named maximum). **Automatic expansion**: a box written for one channel that receives N is instantiated N times, one state per channel. Mismatched counts: a 1-channel signal is broadcast to all; any other mismatch is rejected when the plan is built, naming the cable. Several cables into one inlet are summed channel by channel, in the existing fixed order. **Buses**: named, multichannel paths with sends (with a gain) and receives, and groups (a bus feeding a bus); a bus sums its senders in the order of their box ids, so a mix is deterministic; a send and a receive are graph edges, so a bus cycle needs a delay like any cycle.

**Why.** Surround, ambisonics, per-voice processing and track routing all need it; per-channel expansion lets one mono box serve every width.

**Rejected.** One cable per channel only: patches become unreadable at 16 channels. Wrapping mismatched counts (2 into 3 cycling): a silent surprise. Unordered bus summing: the bits would depend on scheduling.

**State of the art.** Recent patching environments offer multichannel cables with expansion; workstations offer buses, sends and groups. This offers both in one engine, with summing orders that are part of the result and so identical everywhere.

**Consequences.** Engine: a channel count per outlet, instances per channel, buses in the planner. Patch format: bus names, sends and receives; box arguments that set channel counts (ticket #3, step 3). Hash: unchanged. Tests: expansion against N separate boxes, bit for bit; a mismatched cable rejected; bus summing order; channel counts 0, 1 and the maximum, and one past it.

**Step.** #12 step 16. **Order:** 9.

## 7. Polyphony

**Decision.** Two complementary forms:

- **Voices per subpatch**: a polyphonic subpatch is instantiated V times when the plan is built (V declared, so a note never allocates). An **allocation policy** (round robin, lowest free voice, last used) and a **stealing policy** (oldest, quietest, lowest pitch, highest pitch, never) are settings, ties broken by voice index; a voice tells the engine when it is done (its envelope ended), and idle voices are not processed. Voice outputs are summed in voice index order. **MPE** zones and MIDI 2.0 per-note controllers route each note's pitch bend, pressure and timbre to its voice.
- **Polyphony through multichannel cables**: a voice allocator box outputs one channel per voice (pitch, gate, expression), so a polyphonic patch is any patch processing N-channel cables. Both forms share the same allocator, so they allocate and steal identically.

**Why.** Instrument designers need voices that hold whole subpatches; modular users want voices as channels they can process freely.

**Rejected.** Allocating a voice on note-on (allocation on the audio thread). A single, fixed stealing policy. Unordered voice summing.

**State of the art.** Matches the voice management of professional samplers and synthesizers (polyphony limits, stealing modes, MPE) and the per-subpatch polyphony and multichannel polyphony of patching environments, deterministically.

**Consequences.** Engine: the voice allocator, voice activity, per-note routing. Patch format: a polyphonic subpatch with V and its policies. Hash: unchanged. Tests: allocation and stealing laws over random note streams; voice summing order; MPE routing; no allocation at note-on, by the counting allocator.

**Step.** #12 step 21. **Order:** 13.

## 8. Nyquist policy

**Decision.** Oscillators are **band-limited by default**, each with an exact **raw** variant (the naive waveform, aliasing included) selectable per box. **Every frequency-like parameter declares its Nyquist policy**: `band-limit` (partials at or above Nyquist removed: a sine above Nyquist is silence), `fold` (the raw behavior, aliasing), `clamp` (the frequency is held just below Nyquist). The band-limiting method of each oscillator is chosen per waveform by measurement (aliasing level, cost), and its aliasing threshold is documented with the box.

**Why.** Aliasing is the most audible defect of digital oscillators; the raw variant is needed for modulation sources, test signals and deliberate effects.

**Rejected.** Raw by default (aliasing for every user who does not know). Band-limited only (no exact naive shapes for control).

**State of the art.** Professional synthesizers are band-limited; patching environments often default to raw shapes. This defaults to the professional choice, keeps the raw one, and publishes each box's measured aliasing.

**Consequences.** Engine: the policy in `ParamSpec`; `osc~`'s documented aliasing above Nyquist becomes a declared policy. Patch format: a per-box variant (`raw`), as a box argument. Hash: unchanged for a sine below Nyquist; re-blessed where the policy changes output. Tests: aliasing measurements across the rate matrix; each policy at, below and above Nyquist.

**Step.** #12 steps 4 and 15. **Order:** 7.

## 9. Phase

**Decision.** Oscillator phase is an **exact 64-bit unsigned integer accumulator**: the phase is a fraction of a cycle in units of 2^-64, wrapping by integer overflow. The increment is `round(frequency · 2^64 / rate)`, computed exactly in integers once per frequency change; the phase is converted to a float only to evaluate the waveform.

**Why.** Integer addition is exact: the only error is the increment's rounding, at most 2^-65 cycle per frame, with no accumulated rounding of the phase and no wrap test. Long renders never drift beyond that bound, and a reference is exact rational arithmetic.

**Rejected.** A floating-point phase (today's `f64`): each addition rounds, the error grows with time and depends on the regime. A 32-bit integer phase: frequency resolution too coarse at high rates.

**State of the art.** Exceeds the floating-point phase of most environments; matches the best hardware-style accumulators, with frequency resolution of `rate / 2^64`.

**Consequences.** Engine: `osc~` rewritten on the accumulator. Hash: `osc~` references re-blessed (they change in the last bits). Tests: the long-run drift check against the exact phase, skipping ahead in state over hours (step 10); the increment's rounding bound tested at the extremes.

**Step.** #12 steps 15 and 10. **Order:** 7.

## 10. Precision

**Decision.** **64-bit floating point processing by default on every target**, 32-bit available as a host setting for performance. The engine is generic over its sample type; each precision has **its own reference hashes**, and both are checked natively and in WebAssembly.

Sub-decisions:

- The precision setting governs signal buffers and per-frame arithmetic. **Parameter values and values derived from them** (coefficients, increments) are always computed in 64 bits; phase and musical-time accumulators are integers in both precisions.
- **Parameter values become IEEE 754 binary64** in the patch format (version 2): the nearest binary64 to the JSON number, which is what every JSON reader and JavaScript already produce. Today's binary32 rule stays for version 1 documents, read as their binary32 values.
- **Delivery to a 32-bit host** (the AudioWorklet, most device backends): the 64-bit output is rounded to binary32, correctly and identically on every target, after the output stage.
- **The render hash records the precision**: a new version, `render-hash/2`, adds the sample format of the hashed samples (binary32 or binary64, 4 or 8 bytes each) and the processing precision that produced them. A case therefore has up to three references: 64-bit, 64-bit delivered as binary32 (what a browser captures), and 32-bit.

**Cost.** A 64-bit sample takes twice the memory, cache and memory bandwidth of a 32-bit one. Vector units hold half as many lanes (2 instead of 4 in a 128-bit register, the WebAssembly SIMD width; 4 instead of 8 in 256-bit ones), so vectorized loops do up to half the work per instruction. Scalar 64-bit arithmetic costs about the same as 32-bit on current processors, in browsers too, but 64-bit transcendental functions need more terms. The benchmarks (step 6) measure the real ratio per box and on each browser engine; the 32-bit setting exists for hosts where it matters.

**Conformance consequence.** Every reference hash changes: the existing `render-hash/1` references are **re-blessed** under `render-hash/2`, for both precisions, through `cargo xtask bless` in a commit that says why. This is a migration step of its own, done before more references accumulate.

**Why.** 64-bit processing removes rounding noise from recursive filters, long mixes and slow modulations, and keeps every box far inside its thresholds; it is what the most demanding professional tools offer.

**Rejected.** 32-bit only (rounding noise in low-frequency recursive filters). 64-bit only (no way to trade precision for CPU on a phone or a heavy browser patch). A precision per subpatch: conversions at every boundary and a hash that no longer names one precision.

**State of the art.** Professional workstations offer 64-bit processing as an option; this makes it the default on every target, the browser included, and proves both precisions bit for bit across targets.

**Consequences.** Engine: a `Sample` trait for `f32` and `f64`; every box generic over it; the hash crates and the oracle read both formats. Patch format: version 2, binary64 parameters, with a migration from version 1. Hash: `render-hash/2`. Tests: every conformance case and every law in both precisions, native and WebAssembly; the measurements of step 2 in both.

**Step.** #12 step 11. **Order:** 3.

## 11. Denormal numbers

**Decision.** **Flushed to zero deterministically on every target**, in software, at defined points: every box's outputs and every recursive state, each frame; subnormal parameters are zero (decision 5). The native shells **set the floating-point environment to the IEEE default** (no hardware flush-to-zero or denormals-are-zero) for the duration of every engine call, and restore the host's mode after it, so that a plugin host that turned those modes on cannot change a bit; the shells also detect the mode in safe code (a subnormal computed and observed) and report a host whose mode could not be set.

**Why.** WebAssembly has no hardware flush; a native plugin host may set one. Results equal on both require the same arithmetic on both, and software flushing keeps subnormals from slowing down recursive tails.

**Rejected.** Relying on the hardware flags (absent in WebAssembly, host-dependent natively). Proving every box insensitive to the host's mode instead of setting it: not provable for intermediate results. Adding noise or offsets to avoid subnormals: changes the signal.

**State of the art.** Professional tools set the hardware flags on their own threads; this gets the same speed and also the same bits as WebAssembly.

**Consequences.** Engine: a flush function used by box laws. Native shells: setting the floating-point control register needs platform code that safe Rust does not offer: one more minimal, audited file lifting `unsafe` (x86_64 and aarch64), added to the list `cargo xtask check` enforces, and tested by forcing the host's modes on. Hash: unchanged (subnormals are already hashed as they are). Tests: a subnormal conformance case; a law that no box outputs or keeps a subnormal; a native test with flush-to-zero forced on before the call.

**Step.** #12 steps 3 and 9. **Order:** 3, before step 11.

## 12. Output stage

**Decision.** Two modes, a host setting: **raw** (the signal untouched) and **protected** (a safety limiter). In **both** modes, non-finite samples are always replaced by zero and reported (count, first frame, channel). The playground defaults to protected; the native renderer, conformance cases and plugins default to raw.

Sub-decisions:

- The protected limiter is a **true-peak limiter** (ITU-R BS.1770 oversampled peak estimation) with a ceiling setting, short lookahead and a declared latency, compensated like any box's.
- The stage runs **inside the engine**, before delivery and before hashing, so it is deterministic and covered by conformance; a render hash is taken after it.
- The internal signal has no headroom limit (floating point); full scale is ±1 at delivery only.

**Why.** A browser page must never blast a listener's ears; a mastering engine must never touch a signal. Non-finite samples are always a fault, and a NaN reaching a device can damage speakers or poison a host.

**Rejected.** A single mode. Clipping instead of limiting (distortion as protection). Leaving non-finite samples to the host (today's behavior, which makes "never NaN in the output" a host duty).

**State of the art.** Workstations offer a master limiter as a plugin and some protect against non-finite output; this builds both into the engine, proven by conformance.

**Consequences.** Engine: an output stage after `Plan::process`; its report. Hash: references re-blessed where a case output non-finite samples (the special-value vectors of the hash itself are unchanged). Tests: the limiter's measurements (ceiling never exceeded in true peak, distortion below a threshold); non-finite replacement in both modes; the "never NaN" property becomes a law.

**Step.** #12 step 12. **Order:** 4.

## 13. Latency

**Decision.** Every box **declares its latency** in frames (it may depend on its parameters and the rate). The planner computes each path's latency and **compensates automatically**: shorter paths are delayed where paths meet (an inlet with several cables, a bus, the output), so every path arrives aligned. **Events are aligned too**: an event for a box is delivered at its frame plus that box's compensation, so automation stays on the beat of the compensated audio. The patch's total latency is reported to the host (a plugin reports it to its own host). A latency change while playing goes through a hot swap (decision 15). Latency inside a cycle cannot be compensated (it is part of the loop) and is reported. Compensation can be turned off per patch, for live monitoring.

**Why.** Look-ahead limiters, linear-phase filters, spectral subpatches and external processors all add latency; without compensation parallel paths comb-filter.

**Rejected.** Manual compensation by the user. Compensating audio but not events (automation would lead the audio).

**State of the art.** Matches workstation-grade automatic delay compensation, including automation alignment, which patching environments do not offer.

**Consequences.** Engine: latency in the box contract; compensation delays in the planner (allocated when the plan is built). Patch format: the per-patch compensation setting. Hash: unchanged. Tests: parallel paths with different latencies summing to an exact cancellation; events aligned; reported latency equals the measured one.

**Step.** #12 step 18. **Order:** 11.

## 14. State

**Decision.** **Every box's state is serializable**: a box type declares a state version and a maximum state size, and writes its state in a canonical binary encoding (little-endian, floats by their bits). A snapshot is copied on the audio thread at a frame boundary into a preallocated buffer, then encoded off it. Presets (parameter values, optionally state), project saves and resumed renders all use it.

**Why.** Saving a project mid-performance, presets, hot swap (decision 15) and long tests (skipping hours of signal) need it.

**Rejected.** Saving parameters only (a delay line or a sequencer's position is lost). A text encoding of state (floats would not round-trip by bits).

**State of the art.** Matches the full plugin-state save of workstations, and adds a law they do not prove: **a render saved at frame t and restored continues with exactly the same bits**.

**Consequences.** Engine: `save` and `restore` in the box contract. Patch format: an optional state per box, versioned (ticket #3). Hash: unchanged. Tests: the resume law over every box and random patches; old state versions migrate; malformed state is refused, never trusted.

**Step.** #12 step 19. **Order:** 12.

## 15. Hot swap

**Decision.** A patch edit while playing **never drops out**. The new plan is built off the audio thread; the swap is an **event at a frame**, so an offline render with edits is deterministic. State moves **by box identity**: a box with the same id and type keeps its state (migrated across state versions); a changed parameter ramps to its new value; a new box starts fresh; a removed box is dropped off the audio thread. An **optional crossfade** runs both plans for a set duration (linear, since the two are correlated when state is kept; equal-power as a setting for unrelated plans).

Building off the audio thread, per host:

- **native**: built on any thread, swapped by exchanging a pointer, the old plan freed on a non-audio thread;
- **browser**: built inside the engine's WebAssembly instance in **slices of bounded work per render quantum**, while the old plan keeps playing, from memory reserved in advance so that the module's memory never grows on the audio thread; when cross-origin isolation and WebAssembly threads are available, built by a worker in shared memory instead. An edit too large for the reserved memory creates a second engine processor and crossfades the two in the browser's audio graph.
- The construction of the browser's engine processor itself (compiling, instantiating, the first plan) stays on the audio thread, once, before it renders: bounded and measured on the three browser engines (step 6).

**Why.** Live coding and live patching need edits that are heard immediately and never click.

**Rejected.** Rebuilding everything with a silence gap. Transferring state by position in the document (an edit that reorders boxes would scramble state). Building the browser plan in one go on the audio thread (an unbounded stall).

**State of the art.** Matches the glitch-free editing of the best live environments, with state kept per box and a sample-accurate, reproducible swap.

**Consequences.** Engine: plans as swappable units; incremental, budgeted plan building; the swap event. Hash: unchanged. Tests: a render with swaps equals the expected splice; no allocation or memory growth on the audio thread while swapping; the slice budget measured.

**Step.** #12 step 20 (with step 5 for the browser's audio thread). **Order:** 12, after state.

## 16. Parallelism

**Decision.** Natively, the plan is scheduled on **several cores**, with **results identical whatever the thread count**. The core stays free of threads (`no_std`): it exposes the plan as tasks with fixed dependencies and a pure "run this task" operation; the host shell owns the threads and the synchronization. Every sum whose terms come from different tasks is computed after all of them, in its fixed order, so completion order never matters. The browser host runs single-threaded for now; the same tasks run in order there.

**Why.** Large patches and workstation sessions need every core; determinism is the core promise.

**Rejected.** Atomic accumulation into shared buses (the order of additions, so the bits, would depend on timing). Threads inside the core (it would no longer be portable).

**State of the art.** Matches the multi-core graph scheduling of workstations, and adds bit-identical results at any thread count.

**Consequences.** Engine: a task graph in the plan. Hash: unchanged. Tests: every case rendered with 1 to many threads, identical; Miri and the counting allocator on the shell's scheduler.

**Step.** #12 step 22. **Order:** 14.

## 17. Hooks for what the engine does not own

The engine provides the hook; the rest is outside it, on later cards. **Order:** 15, after everything above: musical time, then inputs, then external processors.

- **Musical time.** Boxes receive, at every frame, the transport position, tempo and time signature. The host sends transport events (play, stop, locate, loop, tempo, signature) at their frames; the engine advances the position between them with an exact integer accumulator (as the phase, decision 9), so a tempo change is sample-accurate and the position never drifts; the host's own position is authoritative at each transport event. The transport interface, the timeline and the arrangement are outside the engine (card "Transport and timeline"). Step 23.
- **Inputs.** Audio input boxes read the host's input channels (a plugin's input and side-chain buses, a device's inputs, a file offline). Device access is outside the engine (card "Audio and MIDI devices"). A conformance case with inputs records them, so it stays deterministic. Step 24.
- **External processors.** A box kind for third-party plugins: it declares its latency (compensated), imposes its block size (the engine rebuffers for it, adding declared latency), saves an opaque state blob, and is **marked non-deterministic**: every output that depends on it is marked, and a render so marked is excluded from hash guarantees and says so. Loading, sandboxing and plugin interfaces are outside the engine, native only (card "Native plugin hosting (VST3, CLAP, AU)"). On a host without plugin support, the box loads, outputs silence or passes its input through (a per-box setting) and is reported as unavailable; the patch stays intact. Step 25.

**State of the art.** Matches workstation transport sync, side-chain inputs and plugin hosting with compensated latency; adds an explicit, propagated mark of what cannot be proven.

## 18. Remaining points of ticket #12

- **Patches without audio output: time advances.** A patch with no output channel still runs every box (it may record, analyze, send messages or MIDI); a render of it has zero samples per frame but its frames are processed. The native renderer, which today runs no box, changes to agree with the WebAssembly engine, which runs them all. Its conformance is proven through what it observes (probe outputs), since its hash only covers rate and duration. Step 8, order 5.
- **Without SharedArrayBuffer** (a page not cross-origin isolated): the oscilloscope uses an AnalyserNode (no shared memory needed); capture and metering post a bounded summary (peaks, a decimated window) at a fixed low rate, in buffers from a pool transferred back and forth, instead of a message per render quantum; full-rate capture is reported as unavailable, never approximated. Events to the engine go through the processor's message port, batched, with frame timestamps, so they stay sample-accurate when scheduled ahead and are counted late otherwise. The allocations of a message are bounded per second, measured, and listed among the known deviations. Step 5, order 8.
- **Off-audio-thread construction**: decision 15.
- **The real-time cost estimate** (step 7) uses the task graph of decision 16 and the per-box benchmarks of step 6.

## Implementation order

Each step lands with its tests, its format or hash version, and its conformance re-bless when bits change. The order follows dependencies; what changes every reference comes before more references are added.

1. **Step 2**, measurements: every later quality claim needs them.
2. **Step 6**, benchmarks: the cost of the decisions below is measured, not guessed.
3. **Steps 3 and 9**, denormals and subnormal parameters, then **step 11**, precision and `render-hash/2`: one re-bless of every reference, in both precisions.
4. **Step 12**, output stage: fixes what a render ends with.
5. **Steps 13 and 8**, host settings and the engine-block law, then outputs without channels.
6. **Steps 4 and 14**, parameters, events, messages and rates: everything after relies on events.
7. **Steps 15 and 10**, oscillators: integer phase, Nyquist policy, long-run drift.
8. **Steps 5 and 7**, the browser's audio thread, then the cost estimate.
9. **Step 16**, multichannel cables and buses.
10. **Step 17**, subpatches with their blocks, and feedback (needs subpatches in the format, ticket #3).
11. **Step 18**, latency compensation (subpatch blocks and the limiter introduce latency).
12. **Step 19**, state, then **step 20**, hot swap (needs state).
13. **Step 21**, polyphony (needs subpatches, events, multichannel and state).
14. **Step 22**, parallel scheduling, once the graph's shape is stable.
15. **Steps 23, 24, 25**, the three hooks; external processors last (they need latency, state and rebuffering).
