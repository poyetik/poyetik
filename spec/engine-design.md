# Engine design decisions

The decisions the engine is built on, recorded before any code that depends on them (roadmap #12 step 1). Each one gives the decision, why, the alternatives rejected, its consequences (engine, patch format, render hash, tests) and the ticket step that implements it. Each decision gives its place in the [implementation order](#implementation-order) at the end, which says what comes first.

**The engine's own signal-touching parts are measured like boxes.** Every engine component that touches the signal has objective measurements with derived thresholds, as `spec/measurements.md` defines them and with its "How to add measurements" steps, landing with the step that builds it:

| Component | Measurements | Decision | Step |
|---|---|---|---|
| Subpatch resampler, overlap-add | Imaging and aliasing rejection, passband ripple, SNR; a spectral round trip exact to a derived bound | 2 | #12 step 26 |
| Parameter smoothing | No zipper noise: the spectral spur level during a ramp; the ramp's exactness (its last frame equal to its target) | 5 | #12 steps 14 and 15 |
| Output-stage limiter | Transparency below its threshold (bit-identical); a bound on true-peak overshoot; THD while limiting | 12 | #12 step 9 |
| Latency compensation | Paths aligned to the sample: a null test of two equal paths of different latency | 13 | #12 step 27 |
| Hot-swap crossfade | No click: a bounded discontinuity across the swap | 15 | #12 step 30 |
| Sample-accurate event timing | An event lands on its exact frame, proven at engine block and host buffer boundaries | 4 | #12 steps 14 and 15 |
| Resource resampler | Passband ripple, stopband rejection, aliasing | 26 | #12 step 31 |
| FFT | Error against a direct transform in higher precision, within a derived bound; inverse after forward within a derived bound | 27 | #12 step 25 |

This document states what the engine **will** do. What it does today is specified by [`patch-format.md`](patch-format.md) and [`render-hash.md`](render-hash.md); each of those changes, in the step that implements a decision, with a new format or hash version where the bytes change.

## How the choices were made

1. Maximum quality, proof and flexibility.
2. Where two good options conflict, both are offered as a setting; the default is what the most demanding professional tools offer.
3. Where only one can exist, the one with the highest quality and the strongest proof.

The engine must power both a full audio workstation (tracks, sample-accurate automation, latency compensation, transport) and a visual patching environment with fine control of block sizes, inputs and outputs. Nothing below may weaken the core promise: **the same samples, bit for bit, on every target**.

## Words

Words this document introduces, to enter `GLOSSARY.md` with the step that gives them a name in the format or the code:

| Word | Meaning |
|---|---|
| **engine block** | The number of frames the engine processes per internal pass, chosen by the host; a performance setting only |
| **subpatch** | A patch placed as a box inside another, with its own inlets and outlets, and optionally its own block size, resampling and overlap |
| **event** | Something that happens at one exact frame: a message, a parameter change, a MIDI packet, a transport change |
| **message** | A value sent along a cable at one frame (a number, a list, a symbol), as opposed to a signal |
| **signal** | A value at every frame, carried by a cable as one or more channels |
| **voice** | One instance of a polyphonic subpatch |
| **bus** | A named, multichannel signal path that several boxes send to and others read from |
| **channel layout** | The ordered meaning of an outlet's channels (speaker roles, positions, ambisonic components, binaural ears, or discrete), checked when the plan is built |
| **coordinating layer** | What runs above the engine, outside it: asynchronous work (interface, files, network, an assistant), heavy logic ahead of time, and the conversion of times between the audio library and sibling libraries |
| **return path** | What the engine sends out: streams of timestamped messages, latest-value slots, engine reports, and the trace stream when enabled |
| **event log** | The record of what a live engine applied (events at their applied frames, plan swaps, seed, resources, underruns); replayed, it gives the same bits |
| **execution mode** | Live, offline or build; a box type is **live-capable** or **offline-only** |
| **payload** | The typed, bounded content of a message: trigger, number, symbol, list, MIDI, note, resource |
| **symbol** | A bounded piece of text in a message, interned when the plan is built |
| **seed**, **stream** | The patch's 64-bit random seed; a box's random stream, derived from its path of box ids |
| **seek** | Jumping a render to a frame with every state restored for it |
| **external** | A box type written by a third party as a WebAssembly module implementing the box contract (not an external processor, decision 17) |
| **resource** | Immutable data a box reads (samples, tables, impulse responses), identified by the SHA-256 of its source bytes |
| **event quantum** | The grid, in frames, on which a patch or a subpatch applies its events and messages: 1 frame by default (every event at its exact frame) |
| **instantaneous response** | What a box declares of how its output at a frame depends on its input at the same frame (none, linear, or non-linear with its derivative), so that the engine can resolve a delay-free loop through it |
| **discontinuity event** | A step or a kink of a signal at a sub-sample position, sent by a box beside its samples, from which a receiving box corrects its output (hard sync with band-limited steps) |
| **render-ahead** | Rendering a part of the plan that depends on no live source before its frames are heard |

---

## 1. Host independence

**Decision.** The engine is a library. Its host chooses the sample rate, the host buffer (the device buffer of an audio backend is one), the engine block and the input and output channel counts. The browser's AudioWorklet is one host among others: a native audio backend, a desktop shell embedding the native engine, the engine packaged as a VST3, CLAP or AU plugin, the offline renderer. A host states what it asks for; the engine answers with what it applies. **A setting a host cannot honor is reported, never faked**: the AudioWorklet cannot choose its render quantum (128 frames) nor guarantee the sample rate asked of its context, so it reports both as imposed.

**The engine is an audio engine, standalone.** It counts time in frames, processes signals and the messages that drive them, and is usable as it is by any host. Other domains (symbolic scores, video, heavy patch logic) are **sibling libraries** when they come: separate libraries that share the engine's conventions (a graph of boxes, the box contract of decision 25, determinism, serializable state), each with its own time unit. A **coordinating layer** above the libraries, outside them, carries what crosses from one to another and converts times at that boundary by one written rule (decision 19).

**Why.** One engine for every product; a fixed browser assumption would make the native and plugin hosts second-class. An audio engine kept to audio stays small, fast and provable; generality comes from shared conventions, not from a core that abstracts every domain.

**Rejected.** An engine shaped around the AudioWorklet (fixed 128 frames): it would forbid small buffers natively and large ones offline. Silently substituting a setting: a user tuning latency must see what is really applied. A domain-agnostic core in which audio is one plugged-in domain among others: every frame would pay for indirections audio does not need, and the core's types (time, payloads, rates) would be shaped by the needs of every domain at once.

**Consequences.** Engine: a `HostSettings` request and an `AppliedSettings` answer, with one reason per refused setting. Patch format: unchanged (host settings are not part of a patch). Hash: unchanged (the rate and channel count already are). Tests: every setting across the test matrix, and a law that the applied settings equal the request whenever the host can honor it.

**Step.** #12 step 10. **Order:** 5.

## 2. Three levels of block size

**Decision.** Three independent levels:

1. the **host buffer**: whatever the host delivers (any length, varying from call to call);
2. the **engine block**: chosen by the host, from 1 frame to a documented maximum, any integer, not only powers of two;
3. the **subpatch block**: set in the patch, per subpatch, with an integer up- or downsampling factor and an overlap factor (for spectral processing); a block of 1 frame is allowed.

The first two are **performance settings only: a render never depends on them.** The third is part of the patch's meaning (a spectral subpatch with a 4096-frame window and four-fold overlap sounds like that wherever it runs), so it is stored in the patch.

Sub-decisions:

- **Renders are independent of the engine block.** Every box processes sample-accurately; a box that saves work by updating derived values (filter coefficients, the segments of an envelope) less than every frame does it every `N` frames on a grid of **absolute frame numbers** (the multiples of `N` counted from frame 0, at the rate the box runs at), never on engine block or host buffer boundaries. `N` is part of the box's algorithm, so changing it takes a new algorithm version (decision 29). A law checks the bits of every conformance case across the engine block sizes of the test matrix, 1 included.
- **A subpatch without a declared block has none**: it is processed in whatever spans the engine chooses, and its result is the same.
- **A subpatch with a declared block `B`, overlap `O` and factor `R`** collects `B` frames of its inputs at its own rate (`R` times the outer rate, or `1/R` of it), runs every `B/O` frames, and overlap-adds its outputs. It introduces a latency the engine derives from `B`, `O` and the resampling filters, declares and compensates (decision 13). The resampling filters are linear-phase, specified by their coefficient tables (identical on every target), and measured (passband ripple, stopband rejection, aliasing) like any box.
- Internal rates of a resampled subpatch may exceed the host's supported range; the type of an internal rate is wider than `SampleRate`, and the limits of the factor are named constants, tested at the limit and one past it.

**Why.** The engine block trades CPU (fewer calls, better vectorization) against latency and granularity; making it semantic would make the same patch sound different on two hosts. The subpatch block is the opposite: a musical choice, which must travel with the patch.

**Rejected.** A fixed engine block (today's 64 frames): it would fix the latency floor and the cost of every host. A single engine-wide block chosen by the patch: the host could no longer pick its latency. Power-of-two engine blocks only: an odd host buffer (a plugin host asking 441 frames) is then rebuffered for nothing. Updating coefficients once per engine block: the result would depend on the block size.

**Consequences.** Engine: `BLOCK_SIZE` stops being a constant of the core; `Block` becomes a span of a host-chosen length; the plan cuts spans at events and at the grid of absolute frames. Patch format: subpatches, with optional `block`, `overlap` and `resample` (format version 2 or later, ticket #3). Hash: unchanged. Tests: the engine-block law over every case; subpatch blocks across the matrix (1, a prime, a power of two, the maximum); the resampler measured with derived thresholds (imaging and aliasing rejection, passband ripple, SNR); a spectral round trip (analysis, synthesis) exact to a derived bound.

**Step.** Engine block: #12 step 10. Subpatch blocks: #12 step 26. **Order:** 5 for the engine block, 10 for subpatch blocks.

## 3. Feedback

**Decision.** A cycle is allowed through an **explicit delay box**, whose delay is at least one frame, or, without delay, when **every box in the loop declares its instantaneous response** (below). Two ways to get single-sample feedback through a delay, both offered:

- **automatic**: in a region without a declared block, the planner processes a cycle (its strongly connected component) in spans no longer than the shortest delay in it; a delay of 1 frame runs that component frame by frame, and nothing else in the patch pays for it;
- **explicit**: a subpatch with a block of 1, for a user who wants the boundary visible.

Inside a subpatch with a declared block `B`, a cycle's delay must be at least `B` (the planner rejects a shorter one, naming the delay box and suggesting a block of 1).

**Delay-free loops**, for analog modelling across boxes (zero-delay-feedback and topology-preserving filters patched from parts, ladders, feedback around a saturator):

- **Each box of the loop declares its instantaneous response** through the box contract (decision 25): **linear**, `y = a·x + b` over the current frame's input (`a` and `b` computed by the box from its state and its other inputs, as a TPT integrator does), or **non-linear**, a function of the current input with its derivative, or a bounded table with its interpolation. A box that declares none keeps the rule above: a loop through it needs a delay of at least one frame, and the planner names it.
- **The engine resolves the loop at every frame**, in the loop's span of one frame (as the automatic single-sample feedback above):
  - a **linear** loop by solving its small linear system exactly, in a fixed order of operations (the boxes by id, the elimination order fixed when the plan is built), through the math layer, so the same bits everywhere;
  - a **non-linear** loop by one fixed, documented method: the K-method (the loop's linear part solved as above, its non-linearity tabulated or evaluated in a fixed order), or Newton's method with a **fixed number of iterations and a fixed starting point** (the previous frame's solution), never a convergence test, whose number of iterations could differ between targets. The method and its iteration count belong to the loop's algorithm and are recorded like a box's (decision 29).
- Each loop's cost is bounded (the system's size and the iteration count are fixed when the plan is built) and measured; the real-time cost estimate (step 22) counts it.

**Why.** A delay-free cycle has no defined value unless its boxes say how their output follows their input at the same frame; with that, a loop has exactly one solution per frame, which a fixed computation finds with the same bits on every target. An implicit one-block delay (the usual fallback) makes a filter's tuning depend on the block size, which decision 2 forbids.

**Rejected.** An implicit block of delay in every cycle: the result depends on the block size. Running the whole patch frame by frame when any cycle exists: correct but needlessly costly. Iterating a non-linear loop until it converges: the number of iterations, so the bits, could differ between targets. Unrestricted implicit loops among arbitrary boxes: a box that does not declare its instantaneous response gives the solver nothing exact to work with. Keeping zero-delay feedback inside single boxes only: analog topologies could not be patched from parts.

**Consequences.** Engine: strongly connected components in the planner, per-component span limits; the instantaneous response in the box contract; a per-frame loop solver (linear, and the fixed non-linear method). Patch format: today's rule "a cycle without delay is rejected" keeps its meaning for boxes that declare no instantaneous response; a cycle with a delay box, or through declaring boxes only, becomes valid. Hash: unchanged. Tests: a one-frame feedback comb filter checked against an exact reference across engine blocks; a cycle shorter than its subpatch block rejected; the existing cycle rejection kept for undeclared boxes; a zero-delay-feedback one-pole and a ladder topology patched from parts, against closed-form and reference implementations, bit-identical natively and in WebAssembly, measured (frequency response, resonance) with derived thresholds (`spec/measurements.md`); a non-linear loop with its fixed iteration count against a reference computed in higher precision with the same method; the cost of each loop measured.

**Step.** #12 step 26. **Order:** 10.

## 4. Events and messages

**Decision.** Everything that is not a signal is an **event stamped with an absolute frame**: a message, a parameter change, a MIDI packet, a transport change, a plan swap. MIDI input arrives as events: **MIDI 2.0 Universal MIDI Packets** inside the engine (MIDI 1.0 bytes converted at the host boundary), so 32-bit controller resolution and per-note controllers are carried without loss.

Sub-decisions:

- **Timestamps are absolute frame numbers** (64-bit, counted from the start of the render), so an event's effect never depends on how time is split. A host gives timestamps relative to its buffer; the engine converts them.
- **Late events** (a live event whose frame has passed) are applied at the first frame still to be processed and **counted as late** in the engine's report. Offline renders have no late events and stay deterministic.
- **Event quantum**: a setting per patch and per subpatch, `N` frames, **1 by default**: every event and message acts at its exact frame, sample-accurate. With `N > 1`, events and messages act on a grid of **absolute frame numbers**, the multiples of `N` counted from frame 0 at the rate the (sub)patch runs at, never on engine block or host buffer boundaries: an event stamped at frame `t` acts at the first grid frame at or after it, `N · ⌈t / N⌉`, in exact integer arithmetic, in the order of decision 4 among the events acting at that frame. Its effect is delayed, not late: it is counted as late only when that grid frame has already been processed (a live event, as above). Boxes then cut their processing only at the grid, which lowers the cost of event-heavy patches, and renders stay independent of engine blocks and host buffers (the engine-block law holds). `N` is part of the patch's meaning (stored in the patch, ticket #3); the real-time cost estimate (step 22) may suggest raising it; an importer may use it to reproduce the block-quantized timing of other environments.
- **Queues are bounded and preallocated**; an overflow is reported, never allocated around. Their capacities are named constants, and they and their overflow behavior are the same in every execution mode (decisions 19 and 20).
- **Message cables are edges of the graph** like signal cables: a message always flows forward in execution order, and a cycle of messages needs an explicit delay (at least one frame), like any cycle. A box that iterates (a counter loop) does it inside itself. So a message cascade is finite by construction.
- **Order of messages within one frame** (a total order, independent of thread count and of the order of boxes in the document):
  1. events from the host, in the order the host gave them;
  2. then messages produced by boxes, **depth first**: when a box sends a message, it reaches every destination, and everything those destinations send in turn, before the box's next message;
  3. one outlet feeding several inlets with messages delivers them in an **explicit order stored in the patch**: the order of that outlet's cables in the patch's cable list, resolved when the plan is built, at no cost while processing. The engine never infers an order from creation history, box ids or position on screen. (Signal cables into one inlet keep their rule: they are summed in a fixed order.);
  4. a message arriving at frame `t` takes effect before the receiving box computes its sample at `t`.

**Why.** Sample-accurate events are what makes automation, sequencing and MIDI exact; absolute frames make them independent of the host; a forward-only message graph makes ordering deterministic and parallel-safe. Depth first is the order a user can predict from the cables.

**Rejected.** Events quantized to the block or the host buffer (a block's worth of jitter, and results that depend on block size), the event quantum's grid of absolute frames being the deterministic form of quantizing. A separate, lower-priority message scheduler outside the audio thread with its own clock: messages would drift against audio. Breadth-first delivery: the order of side effects stops following the cables. Unbounded message loops within a frame: they can hang the audio thread. Ordering fan-out by creation order: hidden, and deleting and recreating a cable changes behavior invisibly. By position on screen, right to left: moving a box changes behavior. By box id: deterministic, but invisible to the user. A universal time base inside the engine, rational or a tick shared with video and scores (such as 705,600,000 ticks per second): no such tick divides every supported rate (that one does not divide 384 kHz, so frames would no longer be whole ticks), and rational arithmetic on every event costs where 64-bit frame counts are exact and cheap; conversions belong to the coordinating layer (decision 19). Superdense time (several ordered microsteps within one frame) inside the engine: a message cycle keeps its delay of at least one frame, so a cascade stays finite by construction; logic that must iterate without advancing audio time runs in the coordinating layer or inside one box.

**Consequences.** Engine: an event type, per-box event spans, a preallocated queue per plan. Patch format: message cables (distinguished from signal cables by the ports they join); the cables of one outlet are kept in a meaningful order, which the canonical serialization preserves (ticket #3). Editor: when an outlet has more than one message cable, their order is visible; a new cable goes last by default and can be reordered (a "right to left" reorder command may exist); moving a box never changes the order. Hash: unchanged; a conformance case may carry an event list. Tests: ordering laws (same frame, fan-out, depth first) over random graphs; the fan-out order law: permuting the cables of an outlet permutes the delivery, and nothing else does (moving boxes, renumbering box ids, reordering boxes in the document); a render with events identical across host buffers, engine blocks and thread counts; every event landing on its exact frame, at and around engine block and host buffer boundaries; late and overflowing events reported; the event quantum: with `N = 1` renders equal the sample-accurate ones, with `N > 1` renders identical across engine blocks and host buffers, every event acting exactly at `N · ⌈t / N⌉`, at the grid frame when stamped on it.

**Step.** #12 steps 14 and 15. **Order:** 6.

## 5. Rates: every parameter takes messages or signals

**Decision.** Every parameter is also an inlet. It accepts **messages** (a value, or a value with a ramp time) and **signals** (one value per frame). How a connected signal combines with the value set by messages is a **setting per parameter inlet**: `add` (the signal modulates around the value, the default) or `replace` (the signal is the value).

Sub-decisions:

- **Input domain.** Every box accepts any value at every inlet and parameter (0, -0, subnormals, NaN, infinities, huge values) without panicking, and with the same bits on every target. At the parameter boundary, after combination: a non-finite value holds the previous valid value; a finite value outside the range is clamped to it; a subnormal is zero (decision 11). Signal inlets take any value; a box whose recursive state becomes non-finite resets that state to zero and reports it, so that one bad sample cannot silence a box for ever.
- **Smoothing.** Each parameter declares a **kind** (continuous, stepped, trigger) and continuous ones a **smoothing curve**: linear in the parameter's own unit by default, exponential for frequency-like and gain-like kinds (constant ratio per frame, so a sweep is even in pitch and in decibels). A message change ramps over a duration declared per parameter kind and overridable per box and per message; the ramp is a whole number of frames and lands exactly on the target value at its last frame. A signal is never smoothed (it is already exact). Stepped and trigger parameters change at their frame, without a ramp.

**Why.** Any parameter driven at audio rate is what modular synthesis needs; a smoothed message change is what automation needs. The input domain is how a patch stays safe when a user wires anything to anything.

**Rejected.** Separate "control" and "audio" variants of each parameter: twice the boxes, and a user must know which. One-pole smoothing: it never reaches the target exactly and its end depends on rounding. A single global smoothing time: too slow for a gate, too fast for a filter cutoff. Rejecting runtime values out of range: impossible for a signal, which cannot be refused.

**Consequences.** Engine: `ParamSpec` gains a kind, a smoothing curve and a default ramp; parameters become inlets. Patch format: the combination setting per parameter inlet; message cables into parameters. Hash: unchanged. Tests: box laws over the special values of the matrix at every inlet and parameter; zipper-noise measurements per parameter kind (`spec/measurements.md`: the spectral spur level during a ramp, against a derived threshold); a ramp's last frame exactly equal to its target.

**Step.** #12 steps 14 and 15. **Order:** 6.

## 6. Multichannel

**Decision.** **A cable carries N channels**, N fixed when the plan is built (from 0 to a named maximum). **Automatic expansion**: a box written for one channel that receives N is instantiated N times, one state per channel. Mismatched counts: a 1-channel signal is broadcast to all; any other mismatch is rejected when the plan is built, naming the cable. Several cables into one inlet are summed channel by channel, in the existing fixed order. **Buses**: named, multichannel paths with sends (with a gain) and receives, and groups (a bus feeding a bus); a bus sums its senders in the order of their box ids, so a mix is deterministic; a send and a receive are graph edges, so a bus cycle needs a delay like any cycle.

**Channel layouts.** Every signal outlet has a **channel layout**, fixed when the plan is built: an ordered list of channel descriptions, one per channel. A description is one of a closed, versioned set of kinds:

- a **speaker role**, from the speaker labels of ITU-R BS.2051 (left, right, centre, LFE, the surrounds, the heights…);
- a **speaker position**: azimuth and elevation in degrees (azimuth 0 straight ahead, positive to the listener's left; elevation positive upwards), and an optional distance in metres;
- an **ambisonic component**: its order and degree, with its normalization named (SN3D, N3D, or the legacy FuMa weights);
- a **binaural ear**: left or right;
- **discrete**: no meaning, numbered.

Sub-decisions:

- **Presets and custom layouts.** The standard layouts are named presets built from those kinds: mono, stereo, the ITU-R BS.775 and BS.2051 surround and height layouts (5.1, 7.1, 7.1.4 and the others), ambisonics of order N in each channel ordering and normalization (ACN with SN3D or N3D, FuMa), binaural. A custom layout is any list of descriptions, with a name, stored in the patch.
- **Checked when the plan is built.** A cable connects equal layouts. An inlet declared `discrete` accepts any layout of its channel count; an inlet of an expanded box also accepts the 1-channel broadcast above. Any other mismatch is refused, naming the cable and both layouts. A box declares the layout of each outlet: fixed, or the same as one of its inlets'; an expanded mono box keeps the layout of its input.
- **Conversions are boxes.** Ambisonics to binaural, downmixes, upmixes and re-ordering between ambisonic conventions are explicit boxes, never implicit.
- **Metadata only.** Layouts live in the plan, not in the samples: they cost nothing while processing and do not enter the render hash; a render's report names its output layout, and a host reports its device's layout when it knows it (decision 1).

**Why.** Surround, ambisonics, per-voice processing and track routing all need it; per-channel expansion lets one mono box serve every width. A channel count alone cannot tell third-order ambisonics from 7.1.4 plus four: layouts make such a cable a refusal when the plan is built instead of a silent mis-wiring.

**Rejected.** One cable per channel only: patches become unreadable at 16 channels. Wrapping mismatched counts (2 into 3 cycling): a silent surprise. Unordered bus summing: the bits would depend on scheduling. Channel counts without meaning: ambisonics wired into a 7.1 input would play, wrongly, without a word. Implicit conversions between layouts: a hidden, lossy process. A fixed list of layouts without custom ones: arrays of speakers and research setups could not be described.

**Consequences.** Engine: a channel count and a layout per outlet, instances per channel, buses and layout checks in the planner; the layout in the render's report and in the applied host settings. Patch format: bus names, sends and receives; box arguments that set channel counts (roadmap #3 step 3); layouts, as preset names or custom lists of descriptions, with their names (ticket #3). Hash: unchanged. Tests: expansion against N separate boxes, bit for bit; a mismatched cable rejected; bus summing order; channel counts 0, 1 and the maximum, and one past it; every preset and a custom layout through the patch format and back; a mismatch refused for every pair of kinds; a `discrete` inlet accepting every layout of its count and refusing other counts; an expanded mono box keeping its input's layout.

**Step.** #12 step 24. **Order:** 9.

## 7. Polyphony

**Decision.** Two complementary forms:

- **Voices per subpatch**: a polyphonic subpatch is instantiated V times when the plan is built (V declared, so a note never allocates). An **allocation policy** (round robin, lowest free voice, last used) and a **stealing policy** (oldest, quietest, lowest pitch, highest pitch, never) are settings, ties broken by voice index; a voice tells the engine when it is done (its envelope ended), and idle voices are not processed. Voice outputs are summed in voice index order. **MPE** zones and MIDI 2.0 per-note controllers route each note's pitch bend, pressure and timbre to its voice.
- **Polyphony through multichannel cables**: a voice allocator box outputs one channel per voice (pitch, gate, expression), so a polyphonic patch is any patch processing N-channel cables. Both forms share the same allocator, so they allocate and steal identically.

**Why.** Instrument designers need voices that hold whole subpatches; modular users want voices as channels they can process freely.

**Rejected.** Allocating a voice on note-on (allocation on the audio thread). A single, fixed stealing policy. Unordered voice summing.

**Consequences.** Engine: the voice allocator, voice activity, per-note routing. Patch format: a polyphonic subpatch with V and its policies. Hash: unchanged. Tests: allocation and stealing laws over random note streams; voice summing order; MPE routing; no allocation at note-on, by the counting allocator.

**Step.** #12 step 32. **Order:** 13.

## 8. Nyquist policy

**Decision.** Oscillators are **band-limited by default**, each with an exact **raw** variant (the naive waveform, aliasing included) selectable per box. **Every frequency-like parameter declares its Nyquist policy**: `band-limit` (partials at or above Nyquist removed: a sine above Nyquist is silence), `fold` (the raw behavior, aliasing), `clamp` (the frequency is held just below Nyquist). The band-limiting method of each oscillator is chosen per waveform by measurement (aliasing level, cost), and its aliasing threshold is documented with the box.

**Why.** Aliasing is the most audible defect of digital oscillators; the raw variant is needed for modulation sources, test signals and deliberate effects.

**Rejected.** Raw by default (aliasing for every user who does not know). Band-limited only (no exact naive shapes for control).

**Consequences.** Engine: the policy in `ParamSpec`; `osc~`'s documented aliasing above Nyquist becomes a declared policy. Patch format: a per-box variant (`raw`), as a box argument. Hash: unchanged for a sine below Nyquist; `osc~` takes a new algorithm version where the policy changes its output (decision 29), the same one as decision 9's if both land in one merge. Tests: aliasing measurements across the rate matrix; each policy at, below and above Nyquist.

**Step.** #12 steps 14 and 19. **Order:** 7.

## 9. Phase

**Decision.** Oscillator phase is an **exact 64-bit unsigned integer accumulator**: the phase is a fraction of a cycle in units of 2^-64, wrapping by integer overflow. The increment is `round(frequency · 2^64 / rate)`, computed exactly in integers once per frequency change; the phase is converted to a float only to evaluate the waveform.

**Why.** Integer addition is exact: the only error is the increment's rounding, at most 2^-65 cycle per frame, with no accumulated rounding of the phase and no wrap test. Long renders never drift beyond that bound, and a reference is exact rational arithmetic.

**Rejected.** A floating-point phase (today's `f64`): each addition rounds, the error grows with time and depends on the regime. A 32-bit integer phase: frequency resolution too coarse at high rates.

**Consequences.** Engine: `osc~` rewritten on the accumulator. Hash: `osc~` takes a new algorithm version (its references change in the last bits; decision 29): if format version 2 is on `main` by then, version 1 stays while it is supported, with its cases, and version 2 gets its own. Tests: the long-run drift check against the exact phase, skipping ahead in state over hours (step 20); the increment's rounding bound tested at the extremes.

**Step.** #12 steps 19 and 20. **Order:** 7.

## 10. Precision

**Decision.** **Binary64 floating-point processing by default on every target**, binary32 available as a host setting for performance. The engine is generic over its sample type; each precision has **its own reference hashes**, and both are checked natively and in WebAssembly.

Sub-decisions:

- **The engine's math layer.** All box arithmetic goes through the engine's math layer: the sample type with its operators, and the math functions (sine, exponential and the others, today the pure-Rust `libm`, pinned exactly in the workspace manifest, its updates following decision 29). Precision is a run-time setting, so boxes are written once, generic over the sample type. A future arithmetic option (below) is a **build-time** choice of the math layer's implementation, selected when compiling or exporting, never a test per operation at run time, so boxes need no other generic parameter and stay unchanged. Rust has no global switch for its floating-point semantics, which is why this layer is the hook. None of those options is built now, and no instantiation that nothing uses is added.
- **The sample types are the engine's own.** A signal sample is a `Binary64` or a `Binary32`, engine-owned newtypes over the IEEE 754 formats whose operators (`+`, `-`, `*`, `/`, negation, `+=`, `-=`), comparisons and conversions are the math layer's, as is every parameter value and what boxes derive from it (`Binary64`). A box never touches a raw float: the newtypes give no access to their float but through the engine's `host` module, for hosts (the renderer's delivery of samples, hashing, tests), and a parameter's bounds and default, binary32 values of its box type's description, are private to the engine, read by hosts through `host` too. So an arithmetic option changes the math layer's operators and functions, never a box. The newtypes add no instruction: the instruction counts of every box were unchanged when they replaced the plain floats, and the benchmarks hold them within their tolerance (`spec/benchmarks.md`).
- **A product then a sum is one operation of the layer**: every `a × b + c` a box writes is `mul_then_add`; in the strict mode it is exactly the product rounded then the sum rounded, the same bits as the two operators, so a fused multiply-add option will change this function alone.
- **Enforced by tooling**: a box crate is `no_std` (floats have no math method there) and may not depend on `libm` (tests apart); outside its tests, the box crate's root forbids Clippy's `float_arithmetic` (`#![cfg_attr(not(test), forbid(clippy::float_arithmetic))]`, a line xtask's math layer gate requires), so box code computes no float, not even on a float whose type is inferred, and the compiler refuses any lifting of the lint (E0453); xtask's math layer gate refuses `libm` in any crate but the math layer's and the measurements', a build script in the box crate or the engine core, and in box code (the sources of `engine-objects`, but its `*_tests.rs` files, each declared `#[cfg(test)] #[path = "<name>_tests.rs"] mod <module>;` beside it), outside comments, strings and char literals, the names of xtask's `BOX_FORBIDDEN_NAMES` (the raw float types, `libm`, and `host` with its delivery of samples), float literals suffixed `f32` or `f64`, `include!`, and `#[path]` but in that declaration, so that every line of box code is one the gate reads. **Review only**: float arithmetic the compiler evaluates in a `const` item (`const THIRD: Binary64 = Binary64::new(1.0 / 3.0)`), deterministic but outside the layer; a cast to an inferred type (`as _`) given to `Binary64::new`, which the `as_conversions` lint refuses unless an exception with its reason is reviewed; a raw float taken from an engine interface no box has a reason to call (an error's value, a render's output); equality between samples (`==` and `!=` on `Binary64`, `Binary32` or a generic `S: Sample`), exact IEEE equality, allowed by design (the same on every target) and unseen by Clippy's `float_cmp`, which only sees raw floats; and using `mul_then_add` for every product then sum rather than the two operators.
- **The hook is proven real**: a test build of the engine core and the boxes with the fused multiply-add option (`poyetik_math = "fused"`, a build-time choice given to that build only) compiles every box unchanged and passes its tests, and shows `mul_then_add` fused, in a render too; xtask runs it and checks the option was in force. The option is built nowhere else, and no engine ships it.
- The precision setting governs signal buffers and per-frame arithmetic. **Parameter values and values derived from them** (coefficients, increments) are always computed in 64 bits; phase and musical-time accumulators are integers in both precisions.
- **Parameter values become IEEE 754 binary64** in the patch format (version 2): the nearest binary64 to the JSON number, which is what every JSON reader and JavaScript already produce. Today's binary32 rule stays for version 1 documents, read as their binary32 values.
- **Delivery to a host that takes binary32** (the AudioWorklet, most device backends): the binary64 output is rounded to binary32, correctly and identically on every target, after the output stage.
- **The render hash records the precision**: a new version, `render-hash/2`, adds the sample format of the hashed samples (binary32 or binary64, 4 or 8 bytes each) and the processing precision that produced them. A case therefore has up to three references: 64-bit, 64-bit delivered as binary32 (what a browser captures), and 32-bit.
- **The render hash also records the arithmetic mode**, so that a later option never forces another re-bless of every reference. `render-hash/2` reserves it now: its header carries the **arithmetic mode** as a versioned set of **arithmetic options**, one flag each; a reader refuses a flag it does not know. With every flag off, the mode is the strict default, the only one built in ticket #12: IEEE 754 operations each correctly rounded, no fused multiply-add, transcendental functions from the pinned pure-Rust `libm`. The flags defined now, each chosen separately when building or exporting, each a build-time choice inside the math layer, never a test at run time:
  1. **fused multiply-add**: deterministic and provable. A fused multiply-add is correctly rounded once (IEEE 754), so it gives the same bits on every target; WebAssembly has no such instruction (only relaxed SIMD's `relaxed_madd`, which may or may not fuse, so it is non-deterministic and banned), so it is emulated there in software, slowly. Its native gain is real but moderate, largest on FIR filters, convolution and matrix kernels, measured with the benchmarks of step 3 before it is offered;
  2. **reassociable operations** (algebraic rewriting of sums and products, as compilers' fast floating-point modes do): non-deterministic;
  3. **platform math functions** instead of the pinned `libm` (the system math library natively): faster, but different between platforms, so non-deterministic; where a target has no faster function of its own (WebAssembly, whose host math would mean a call into JavaScript), this flag keeps `libm`.

  Room is left for later flags (faster approximations, fixed-point arithmetic). The rules, fixed now:
  1. any combination is allowed; it is a setting of the render, made by the build of the math layer it runs on, never chosen automatically per target (a web preview and a native export of the same patch would differ);
  2. a combination of provable flags only stays provable, with its own references on every target;
  3. any non-deterministic flag makes the render non-deterministic: off by default, enabled only by an explicit opt-in at export, with a warning that states what is lost (bit identity across targets and builds, so no conformance hash; output may differ slightly per target) and what is kept (audio quality measured against the derived thresholds on each target, the protections against NaN and infinities, never assumed away, and the real-time rules); its renders and exported artifacts carry the non-deterministic mark, as external processors do (decision 17);
  4. a host or the editor shows the mode, and a preview rendered in another mode than its export is marked as such;
  5. only the combinations actually built and shipped are tested: the strict default always, any other from when it is first offered; none is offered in ticket #12, and the ban of `mul_add` in `clippy.toml` stays until the maintainer decides otherwise; a box may later offer a fused variant only as an explicit patch setting, under the same rules.

**Cost.** A 64-bit sample takes twice the memory, cache and memory bandwidth of a 32-bit one. Vector units hold half as many lanes (2 instead of 4 in a 128-bit register, the WebAssembly SIMD width; 4 instead of 8 in 256-bit ones), so vectorized loops do up to half the work per instruction. Scalar 64-bit arithmetic costs about the same as 32-bit on current processors, in browsers too, but 64-bit transcendental functions need more terms. The per-box wall-time benchmarks that step 6 adds to those of step 3 (`spec/benchmarks.md`) measure the real ratio per box and on each browser engine; the 32-bit setting exists for hosts where it matters.

**Conformance consequence.** Every reference hash changes: the existing `render-hash/1` references are **re-blessed** under `render-hash/2`, for both precisions, through `cargo xtask bless` in a commit that says why. This is a migration step of its own, done before more references accumulate.

**Why.** Binary64 processing removes rounding noise from recursive filters, long mixes and slow modulations, and keeps every box far inside its thresholds; it is what the most demanding professional tools offer.

**Rejected.** Binary32 only (rounding noise in low-frequency recursive filters). Binary64 only (no way to trade precision for CPU on a phone or a heavy browser patch). A precision per subpatch: conversions at every boundary and a hash that no longer names one precision. Fused multiply-add natively and not in WebAssembly for the same render: the bits would differ between targets. Letting a compiler contract `a * b + c` into a fused operation: Rust never does it, and the lint configuration bans `mul_add`. A hash that records the precision only: a later arithmetic mode would force another re-bless of every reference.

**Consequences.** Engine: a `Sample` trait for the engine-owned `Binary64` and `Binary32`; every box generic over it, its arithmetic the math layer's; the hash crates and the oracle read both formats. Patch format: version 2, binary64 parameters, with a migration from version 1 (#3 step 7). Hash: `render-hash/2`, with the precision and the arithmetic mode in its header. Tests: every conformance case and every law in both precisions, native and WebAssembly; the measurements of step 2 in both; an unknown arithmetic mode value refused by every reader of the hash; the mode changes the hash (two headers differing only by mode hash differently, in the known-answer vectors).

**Step.** #12 step 6; format version 2: #3 step 7. **Order:** 3.

## 11. Subnormal numbers

**Decision.** **Flushed to zero deterministically on every target**, in software, at defined points: every box's outputs and every recursive state, each frame; subnormal parameters are zero (decision 5). The native shells **set the floating-point environment to the IEEE default** (no hardware flush-to-zero or denormals-are-zero) for the duration of every engine call, and restore the host's mode after it, so that a plugin host that turned those modes on cannot change a bit; the shells also detect the mode in safe code (a subnormal computed and observed) and report a host whose mode could not be set.

**Why.** WebAssembly has no hardware flush; a native plugin host may set one. Results equal on both require the same arithmetic on both, and software flushing keeps subnormals from slowing down recursive tails.

**Rejected.** Relying on the hardware flags (absent in WebAssembly, host-dependent natively). Proving every box insensitive to the host's mode instead of setting it: not provable for intermediate results. Adding noise or offsets to avoid subnormals: changes the signal.

**Consequences.** Engine: a flush function used by box laws. The defined points are each box's outputs and recursive state: the plan's sums and delivery are not flush points. Cables summed into an inlet may give a box a subnormal input, which the box laws feed every box (it flushes its own outputs and state, so no recursion runs on subnormals); a channel summing output boxes, and a binary64 render given as binary32 (rounded to the nearest binary32, `host::Delivered`), may deliver a subnormal, exact and hashed as it is (a host under flush-to-zero reads it as zero). The raw output stage keeps them; whether the protected one flushes them is decided with it (decision 12, #12 step 9). Native shells: setting the floating-point control register needs platform code that safe Rust does not offer: one more minimal, audited file lifting `unsafe` (x86_64 and aarch64), added to the list `cargo xtask check` enforces, and tested by forcing the host's modes on. Hash: unchanged (subnormals are already hashed as they are). Tests: a subnormal conformance case; a law that no box outputs or keeps a subnormal; a native test with flush-to-zero forced on before the call.

**Step.** #12 steps 4 and 5. **Order:** 3, before step 6.

## 12. Output stage

**Decision.** Two modes, a host setting: **raw** (the signal untouched) and **protected** (a safety limiter). In **both** modes, non-finite samples are always replaced by zero and reported (count, first frame, channel). The playground defaults to protected; the native renderer, conformance cases and plugins default to raw.

Sub-decisions:

- The protected limiter is a **true-peak limiter** (ITU-R BS.1770 oversampled peak estimation) with a ceiling setting, short lookahead and a declared latency, compensated like any box's.
- The stage runs **inside the engine**, before delivery and before hashing, so it is deterministic and covered by conformance; a render hash is taken after it.
- The internal signal has no headroom limit (floating point); full scale is ±1 at delivery only.

**Why.** A browser page must never blast a listener's ears; a mastering engine must never touch a signal. Non-finite samples are always a fault, and a NaN reaching a device can damage speakers or poison a host.

**Rejected.** A single mode. Clipping instead of limiting (distortion as protection). Leaving non-finite samples to the host (today's behavior, which makes "never NaN in the output" a host duty).

**Consequences.** Engine: an output stage after `Plan::process`; its report. Hash: a new render rules version (decision 29), references re-blessed where a case outputs non-finite samples (the special-value vectors of the hash itself are unchanged). Tests: the limiter's measurements with derived thresholds (bit-identical below its threshold; true-peak overshoot of the ceiling bounded; THD while limiting); non-finite replacement in both modes; the "never NaN" property becomes a law.

**Step.** #12 step 9. **Order:** 4.

## 13. Latency

**Decision.** Every box **declares its latency** in frames (it may depend on its parameters and the rate). The planner computes each path's latency and **compensates automatically**: shorter paths are delayed where paths meet (an inlet with several cables, a bus, the output), so every path arrives aligned. **Events are aligned too**: an event for a box is delivered at its frame plus that box's compensation, so automation stays on the beat of the compensated audio. The patch's total latency is reported to the host (a plugin reports it to its own host). A latency change while playing goes through a hot swap (decision 15). Latency inside a cycle cannot be compensated (it is part of the loop) and is reported. Compensation can be turned off per patch, for live monitoring.

**Why.** Look-ahead limiters, linear-phase filters, spectral subpatches and external processors all add latency; without compensation parallel paths comb-filter.

**Rejected.** Manual compensation by the user. Compensating audio but not events (automation would lead the audio).

**Consequences.** Engine: latency in the box contract; compensation delays in the planner (allocated when the plan is built). Patch format: the per-patch compensation setting. Hash: unchanged. Tests: a null test, two equal paths of different latency summing to an exact cancellation (aligned to the sample); events aligned; reported latency equals the measured one.

**Step.** #12 step 27. **Order:** 11.

## 14. State

**Decision.** **Every box's state is serializable**: a box type declares a state version and a maximum state size, and writes its state in a canonical binary encoding (little-endian, floats by their bits). A snapshot is copied on the audio thread at a frame boundary into a preallocated buffer, then encoded off it. Presets (parameter values, optionally state), project saves and resumed renders all use it.

**Why.** Saving a project mid-performance, presets, hot swap (decision 15) and long tests (skipping hours of signal) need it.

**Rejected.** Saving parameters only (a delay line or a sequencer's position is lost). A text encoding of state (floats would not round-trip by bits).

The seek law (decision 24) generalizes this law to any frame.

**Consequences.** Engine: `save` and `restore` in the box contract. Patch format: an optional state per box, versioned (ticket #3). Hash: unchanged. Tests: the resume law over every box and random patches; old state versions migrate; malformed state is refused, never trusted.

**Step.** #12 step 28. **Order:** 12.

## 15. Hot swap

**Decision.** A patch edit while playing **never drops out**. The new plan is built off the audio thread; the swap is an **event at a frame**, so an offline render with edits is deterministic. State moves **by box identity**: a box with the same id and type keeps its state (migrated across state versions); a changed parameter ramps to its new value; a new box starts fresh; a removed box is dropped off the audio thread. An **optional crossfade** runs both plans for a set duration (linear, since the two are correlated when state is kept; equal-power as a setting for unrelated plans).

Building off the audio thread, per host:

- **native**: built on any thread, swapped by exchanging a pointer, the old plan freed on a non-audio thread;
- **browser**: built inside the engine's WebAssembly instance in **slices of bounded work per render quantum**, while the old plan keeps playing, from memory reserved in advance so that the module's memory never grows on the audio thread; when cross-origin isolation and WebAssembly threads are available, built by a worker in shared memory instead. An edit too large for the reserved memory creates a second engine processor and crossfades the two in the browser's audio graph.
- The construction of the browser's engine processor itself (compiling, instantiating, the first plan) stays on the audio thread, once, before it renders: bounded and measured on the three browser engines (step 3).

**Why.** Live coding and live patching need edits that are heard immediately and never click.

Why the browser's plan is built in the worklet's own instance:

- a plan is data laid out in the memory of the instance that runs it; built anywhere else, it would have to be copied into that memory, and that copy runs on the audio thread, in one piece, in time proportional to the plan: no less work than building it, and unbounded per render quantum;
- without cross-origin isolation a page has no shared memory, so nothing outside the worklet can write into the instance's memory at all; building there is the only path every page has;
- a second instance in a Worker would compile and hold the module twice, and its plan would still have to be copied across;
- in slices, the work per render quantum has a budget, measured (steps 3 and 30), while the old plan keeps playing; where shared memory and WebAssembly threads exist, a worker builds into that same memory, which takes even the slices off the audio thread.

**Rejected.** Rebuilding everything with a silence gap. Transferring state by position in the document (an edit that reorders boxes would scramble state). Building the browser plan in one go on the audio thread (an unbounded stall). Building the browser plan in a Worker without shared memory and transferring it: the transfer lands on the audio thread as one unbounded copy.

**Consequences.** Engine: plans as swappable units; incremental, budgeted plan building; the swap event. Hash: unchanged. Tests: a render with swaps equals the expected splice; the crossfade measured: no click, its discontinuity bounded by a derived threshold; no allocation or memory growth on the audio thread while swapping; the slice budget measured.

**Step.** #12 step 30 (with step 21 for the browser's audio thread). **Order:** 12, after state.

## 16. Parallelism

**Decision.** Natively, the plan is scheduled on **several cores**, with **results identical whatever the thread count**. The core stays free of threads (`no_std`): it exposes the plan as tasks with fixed dependencies and a pure "run this task" operation; the host shell owns the threads and the synchronization. Every sum whose terms come from different tasks is computed after all of them, in its fixed order, so completion order never matters. The browser host runs single-threaded for now; the same tasks run in order there.

**Why.** Large patches and workstation sessions need every core; determinism is the core promise.

**Rejected.** Atomic accumulation into shared buses (the order of additions, so the bits, would depend on timing). Threads inside the core (it would no longer be portable).

**Consequences.** Engine: a task graph in the plan. Hash: unchanged. Tests: every case rendered with 1 to many threads, identical; Miri and the counting allocator on the shell's scheduler.

**Step.** #12 step 33. **Order:** 14.

## 17. Hooks for what the engine does not own

The engine provides the hook; the rest is later work, outside the engine. **Order:** 15, after everything above: musical time, then inputs, then external processors.

- **Musical time.** The engine core knows only frames (absolute frame numbers), never musical time. Musical time is a **time map**, data built and edited by the host or the coordinating layer (decision 1) and given to the engine as a timestamped event or a resource (decisions 4 and 26), so that a live tempo gesture is sample-accurate. Its evaluation at every frame is a library component of the engine's DSP library, used by a **transport box** (or a small transport service the plan instantiates once per transport) on the audio thread, exact and deterministic; only the boxes that need musical time read it, and no other box pays for it. The same time map serves every domain (audio, light, video, notation) through the coordinating layer. Step 35.
  - **A time map is a sequence of segments**: constant tempo; continuous tempo changes (accelerando, ritardando) with a stated curve, linear in tempo or exponential; explicit beat positions (a tempo map recorded from a performance, for rubato and free time); jumps (locate, loop). Within a segment, the position advances by exact steps in integer or exact rational accumulators, with no division per frame and no float accumulation; the frame of each boundary is computed in closed form, in exact rational arithmetic, then rounded to a frame by decision 19's rule. So a long accelerando never drifts.
  - **Meters beyond regular bars**: irregular and additive meters (7/8 as 2+2+3, aksak rhythms), changing meters, nested tuplets; a beat may be subdivided unevenly.
  - **Groove and micro-timing**: a swing or groove template is a deterministic map of offsets applied to positions.
  - **Several transports at once** (polytempo, polymeter): transports are named; each subpatch follows one transport, the patch's by default; a transport may be derived from another (a ratio, an offset).
  - **Unpulsed time**: a patch or a subpatch may follow no transport at all, its time in frames and seconds only.
  - **What a box receives**: its transport's position, exact, in every form it needs: beats, bar, beat and subdivision, and seconds.
  - **Rejected**: a single global constant tempo; positions accumulated in floating point (they drift); tempo changes applied per block (the result would depend on the block size); musical time inside the core (a heavier core, and every new time feature a core change); musical time only in the layer above, sent as events every frame (heavy, and late for live tempo gestures).
  - **Consequences**: the patch format (ticket #3) stores time maps, meters, grooves and which transport each subpatch follows; tests: positions exact at segment boundaries, against exact rational references; renders identical across engine blocks and host buffers; a long accelerando with no drift; the polytempo laws (a derived transport keeps its ratio and offset exactly); a box not reading musical time costs the same with and without a transport.
- **Inputs.** Audio input boxes read the host's input channels (a plugin's input and side-chain buses, a device's inputs, a file offline). Device access is outside the engine (later work: audio and MIDI devices). A conformance case with inputs records them, so it stays deterministic. Step 36.
- **External processors.** A box kind for third-party plugins: it declares its latency (compensated), imposes its block size (the engine rebuffers for it, adding declared latency), saves an opaque state blob, and is **marked non-deterministic**: every output that depends on it is marked, and a render so marked is excluded from hash guarantees and says so. Loading, sandboxing and plugin interfaces are outside the engine, native only (later work: native plugin hosting (VST3, CLAP, AU)). On a host without plugin support, the box loads, outputs silence or passes its input through (a per-box setting) and is reported as unavailable; the patch stays intact. Step 37.

## 18. Remaining points of ticket #12

- **Patches without audio output: time advances.** A patch with no output channel still runs every box (it may record, analyze, send messages or MIDI); a render of it has zero samples per frame but its frames are processed. The native renderer, which today runs no box, changes to agree with the WebAssembly engine, which runs them all. Its conformance is proven through what it observes (probe outputs), since its hash only covers rate and duration. Step 11, order 5.
- **Without SharedArrayBuffer** (a page not cross-origin isolated): the oscilloscope uses an AnalyserNode (no shared memory needed); capture and metering post a bounded summary (peaks, a decimated window) at a fixed low rate, in buffers from a pool transferred back and forth, instead of a message per render quantum; full-rate capture is reported as unavailable, never approximated. Events to the engine go through the processor's message port, batched, with frame timestamps, so they stay sample-accurate when scheduled ahead and are counted late otherwise. The allocations of a message are bounded per second, measured, and listed among the known deviations. Step 21, order 8.
- **Off-audio-thread construction**: decision 15.
- **The real-time cost estimate** (step 22) uses the task graph of decision 16 and the per-box benchmarks of step 3.
- **Later work, outside the engine's decisions**: a per-sample code box, user code compiled to the box contract (a WebAssembly module, deterministic, decision 25); circuit-level boxes, wave digital filters patching the components of an analog circuit, compared with a circuit simulator such as ngspice.
- **Optimizations left to each box and to the build**, under the rules of decisions 2, 10, 28, 29 and 30 (an absolute update grid, arithmetic options as settings of the render, the reduction order, a new algorithm version for any change of bits, skipping only when bit-identical), with no further engine decision: fast approximations and wavetables (only when measured faster and within the derived thresholds), oversampling of non-linear boxes, buffer reuse and cache locality in the plan, specialized compilation at export (the build mode, decision 20), and fixed-point arithmetic for some microcontrollers, as a possible future arithmetic option.

## 19. The control boundary

**Decision.** The engine meets the outside world at one boundary, in both directions, and never waits on it: **events in** (decision 4), **the return path out**, both bounded, preallocated, timestamped in frames, and with the same capacities and overflow behavior in every execution mode (decision 20). What runs asynchronously (the interface, files, network, an assistant, heavy patch logic) lives in the **coordinating layer** above the engine (decision 1), which only talks to the engine through this boundary.

Sub-decisions, inbound:

- **The host stamps, the engine never guesses.** An event reaches the engine with its absolute frame already set. Two stamping policies, chosen per input source by the host:
  - `immediate` (the default for interface gestures, which carry no precise time): the event takes the first frame of the host buffer during which the host's audio-thread glue drains it (in the browser, the first frame of the render quantum whose `process` call drains it); the lowest latency, with a jitter of at most one host buffer;
  - `scheduled` (the default for sources that carry a timestamp: MIDI devices, the coordinating layer, render-ahead): the event's own time converted to a frame, plus a constant control latency the host declares; no jitter. One that arrives after its frame is late (decision 4).
- **Conversion rule at the boundary.** A time from another base (seconds as a rational number, a video frame, a tick of a tempo map) becomes the **nearest frame, ties to the later frame**, computed in exact integer arithmetic (no floating point); a frame `n` becomes the exact instant `n / rate` seconds, and the other side rounds it by the same rule. The engine itself only ever sees frames.
- **Quota and horizon, enforced on the sending side.** The engine's sender (the host-facing part of the audio library, never on the audio thread) refuses at once, with its reason, an event beyond `EVENTS_PER_WINDOW` events stamped in one window of `EVENT_WINDOW` frames of the absolute frame grid, or stamped more than `EVENT_HORIZON` frames ahead of the render position it last read. The audio-thread side drains the transport into a pending queue sorted by frame whose capacity is derived from those three constants and the maximum engine block, so it **cannot** overflow: the only loss is a refusal the sender sees synchronously. The refusal depends only on the events' frames and order, so an offline render of the same event list refuses the same events.
- **Message cascades** inside a frame need no queue: depth-first delivery (decision 4) uses a stack whose depth is the longest message path, known when the plan is built and preallocated exactly. Their time is bounded by the counts of messages each box declares it may emit per message received (decision 25), which the cost estimate uses (step 22).

Sub-decisions, outbound (the **return path**):

- **Streams**: messages a box sends outward (a print, a value a display shows, MIDI out, a notice to the editor), each entry with its frame, the box id and outlet, a 64-bit sequence number and a bounded payload (decision 21). At most `OUTBOUND_PER_WINDOW` entries per window of `EVENT_WINDOW` frames; beyond, an entry is dropped and counted, by a rule that depends only on the render, so the content of a stream is identical in every mode.
- **Latest-value slots**: meters, scope summaries and watched cables, where only the newest value matters. The engine writes a fixed-size summary per period of frames into a slot (three buffers, so writing never waits for a reader); a slow reader reads the newest and loses nothing it needed. A watched cable is a probe registered when the plan is built, never a lookup while processing.
- **Engine reports** (late events, refusals, overflows, non-finite samples replaced, resource underruns, the applied frame of each late event) have their own stream with its own reserved capacity, so box messages never crowd them out.
- **Delivery.** Each stream crosses to its reader through a preallocated single-producer, single-consumer ring. When the reader is slow and the ring is full, the newest entries are dropped and the reader learns the exact range of missing sequence numbers: a delivery gap, reported like a late event, which never changes a sample. Offline the renderer drains the ring whenever it fills (it has no deadline), so it has no gap.
- A host shows an entry when it is heard: its frame plus the output latency the host reports.
- **The trace stream** is the engine's only hook for debugging tools. When the host enables it, the engine reports every delivered message (its frame, its cable, its index in its outlet's cable order, its payload) and the box values the host selected, through the same bounded, non-blocking, sequence-numbered return path. Enabling it is a choice made when the plan is built (a plan with or without tracing): disabled, it costs nothing while processing, not even a branch. Enabled or not, the rendered bits are identical. Slow motion, stepping, breakpoints, animated messages and rewinding are host and editor features built on it, on an offline host driven frame by frame, state snapshots and the seek law (decision 24), not engine features (roadmap #10): the engine never reads a clock, so a host may ask for frames at any pace.

The **live event log** is a host feature with an engine hook: the host records, as the engine applied them, every event (with its applied frame; late events take the frame the engine reports), every plan swap with the patch it installed, the seed (decision 23), the resources by hash (decision 26), resource underruns, and the host settings that change bits (rate, precision, channel counts). **The same patch and the same log give the same bits**: replaying a log offline applies every event at its logged frame, so nothing is late and nothing differs. The log has a canonical, versioned binary encoding (like state, decision 14), symbols written as text; the host decides whether to record it (the editor records by default, an exported app only if its author enables it) and keeps it local.

**Why.** A live performance is only reproducible if everything that entered the engine is known by frame; stamping on the host side and logging what the engine applied makes a live take an offline render. Overflow decided by frames and order, never by timing, is what keeps live and offline identical. The return path must never make the audio thread wait for a reader, so loss is allowed only where it is visible and exact.

**Rejected.** Letting the engine stamp events by reading a clock (the core has no clock, and its result would depend on timing). Blocking or allocating when a queue is full. Overflow decided by how full a ring happens to be (the result would differ between live and offline). One queue for box messages and engine reports (a chatty box would hide the report of a fault). Logging what was sent rather than what was applied (late events would replay at other frames). Converting times in floating point at the boundary. Slowing patch logic down by changing the sample rate: it changes the patch's meaning. Tracing that alters the order of processing or allocates on the audio thread.

**Consequences.** Engine: the sender with its quota and horizon, the sorted pending queue sized by construction, the depth-first stack sized when the plan is built, the outbound streams, slots and report stream, the trace stream chosen when the plan is built, the applied frame of late events. Patch format: none (the log is its own versioned document, specified in `spec/` with test vectors when it is implemented). Hash: unchanged; a conformance case may carry an event log. Tests: the conversion rule against an exact rational reference, at ties and at the extremes of 64-bit frames; quota and horizon at the limit and one past it, the same refusals live and offline; a simulated live host with random lateness, slow readers and full rings: its log replayed offline gives the same bits; every gap reported with its exact sequence numbers; the tracing law: every conformance case and random patches give the same hash with tracing on and off, and a traced render reports every delivered message in delivery order; no allocation and no wait on the audio-thread side, by the counting allocator.

**Step.** #12 step 17 (the browser's transport for it with step 21). **Order:** 6.

## 20. Execution modes

**Decision.** One engine, three **execution modes**, with the same semantics and the same bits:

- **live**: a deadline every host buffer; what cannot keep up is reported (late events, delivery gaps, underruns), never hidden;
- **offline**: no deadline; the renderer waits for what it needs (resources, events), so it is never late and never underruns;
- **build**: the export; the patch is frozen and may be compiled and precomputed (decision 25), and the built result must give the bits of an offline render of the same patch.

Sub-decisions:

- **Every box type declares its mode**: **live-capable** (bounded work per frame and per message, no allocation or release of memory, no lock, no I/O while processing) or **offline-only** (it may allocate while processing; it is still deterministic, with no clock, file or network access).
- **The real-time rules apply to live-capable boxes**, and are proven on each of them by the box laws under the counting allocator, as today; an offline-only box is held to every other law.
- **A live plan cannot hold an offline-only box, by construction**: the plan builder takes the mode, and building a live plan with an offline-only box is refused, naming the box. The editor shows the mode of each box; the cost estimate (step 22) warns when a live-capable patch is too heavy for live use on the machine.
- Queue capacities, quotas, horizons and their overflow rules are the same constants in every mode (decision 19); the mode is never part of the result.

**Why.** Heavy work (engraving-grade analysis, exhaustive searches, offline-quality processing) is legitimate where there is no deadline, and needs no excuse there; real-time rules where there is a deadline must be proven, not hoped. One semantics for every mode is what lets a live take, its offline replay and its export agree bit for bit.

**Rejected.** Real-time rules on every box (heavy offline processes would be impossible for nothing). Real-time rules on no box, with live failure left to luck. A mode that changes semantics (larger queues offline, block-quantized events live): live and offline would differ. Detecting at run time that a box allocated: too late, the audio thread has already paid.

**Consequences.** Engine: the mode in the box contract's metadata (decision 25) and in the plan builder; the live plan's refusal. Patch format: none (the mode belongs to the box type, not the patch). Hash: unchanged; the mode is not recorded, since it cannot change a bit. Tests: every registered box declares a mode (a gate over the registry); every live-capable box passes the allocation laws; a live plan refuses an offline-only box; every conformance case rendered offline and through a simulated live host gives the same hash.

**Step.** #12 step 12. **Order:** 5.

## 21. Message payloads

**Decision.** A message carries a **payload** of one **kind**, typed and bounded, and never allocates:

| Kind | Content |
|---|---|
| trigger | no value |
| number | an IEEE 754 binary64 value |
| symbol | an interned symbol: UTF-8 text of at most `SYMBOL_BYTES_MAX` bytes |
| list | at most `LIST_MAX` atoms, each a number or a symbol |
| MIDI | one MIDI 2.0 Universal MIDI Packet (32 to 128 bits) |
| note | a MIDI 2.0 note packet with its optional spelling (below) |
| resource | a reference to a resource (decision 26) |

Sub-decisions:

- **Open by version.** Each kind has a stable numeric tag; tags not listed are reserved. A new kind (a domain type such as a video frame handle) is added by a minor version of the box contract (decision 25), never by a box on its own.
- **Typed ports.** Every message inlet declares the kinds it accepts; a cable from an outlet whose kinds it cannot accept is refused when the plan is built, naming the cable. No kind is converted into another implicitly; conversion is a box.
- **Symbols are interned when the plan is built**: every symbol in the patch becomes an index in the plan's symbol table, built off the audio thread. A box that makes new text while processing interns it in the table's preallocated spare capacity, with a fixed hash and bounded work; a full table refuses the symbol and reports it. An index is never observable: symbols compare by equality, and any order between them is the order of their bytes; state (decision 14) and the event log (decision 19) write symbols as text, so a symbol's index can differ between plans without changing a bit.
- **No pool can run dry.** Depth-first delivery means a payload lives only until its cascade returns, so payloads live on the preallocated delivery stack (decision 19); a box that holds messages (a delay, a queue) holds them in its own state, of a declared maximum size.
- **Overflow is refused, never truncated**: a list longer than `LIST_MAX` or a symbol longer than `SYMBOL_BYTES_MAX` cannot be built (the builder returns an error); the message is dropped and reported with its frame and box. A truncated list would silently change its meaning.
- **Note spelling.** A note event keeps its symbolic information beside what playback needs: the spelling (a letter and an alteration from double flat to double sharp, quarter tones included), the voice, and the written duration as a fraction of a whole note. A symbolic field is either correct or absent, never stale: a box that changes pitch either computes the new spelling or removes it, and declares which. MIDI 1.0 input and MIDI 2.0 packets without it have no spelling.

**Why.** Allocation on the audio thread slips in through variable-sized data: bounding every payload and interning every symbol before playing closes that door by construction. Typed ports catch at build time what would otherwise be a silent mismatch while playing. Spelling kept on note events lets scores be shown and edited from what the engine played, at no cost to playback.

**Rejected.** Unbounded lists or strings (allocation, or unbounded copies, on the audio thread). Symbols as owned strings in messages (allocation per message). Symbol indices visible to boxes (results would depend on interning order). Implicit conversions between kinds (hidden behavior). Truncating what overflows. Spelling derived from the pitch afterwards (C sharp and D flat cannot be told apart).

**Consequences.** Engine: the payload type and its kind tags, the plan's symbol table with spare capacity, the bounded list builder, typed message ports. Patch format: message contents and box arguments with kinds (symbols as text, numbers as binary64), with ticket #3. Hash: unchanged; event lists in conformance cases carry every kind. Tests: every kind at its limit and one past it; a list or symbol overflow refused and reported; a law that the interning order never changes a bit (the same patch with its boxes listed in another order, symbols interned in another order); a cable of incompatible kinds refused; spelling kept or removed, never wrong, by every box that changes pitch; no allocation while delivering any payload.

**Step.** #12 step 16. **Order:** 6.

## 22. Render-ahead

**Decision.** The engine is designed so that parts of a patch can be **rendered ahead** of the live position, on other cores, without changing a bit; the hooks land now, the scheduling later:

- **Future events**: events may be given with frames ahead of the render position, up to the horizon of decision 19, and wait in the pending queue (order 6).
- **Live and ahead parts**: the plan's task graph (decision 16) is cut into **parts**; a part is **live** when it depends on something that cannot be known ahead (an audio input, an inlet receiving `immediate` events, an external processor), otherwise **ahead**. The host may render an ahead part early, into a buffer, and the live render mixes it in the fixed order of decision 16.
- **Invalidation**: when an event arrives for an ahead part at a frame already rendered ahead but not yet heard, the host discards that part's buffer from that frame, restores the part's state at that frame (decision 24) and renders it again. It is not late. An event whose frame has already been heard is late, as in decision 4.
- **The law**: a render with render-ahead, given the same applied events, equals the same render without it, bit for bit.

What is implemented when: future events and the pending queue at order 6; the cut into parts with the task graph and the law, with an in-process simulation of rendering ahead and invalidating, at order 14; the threads that render ahead (Workers in the browser, threads natively) belong to the hosts, on a later card, without changing the core.

**Why.** A browser page has one real-time thread and cannot make it wait; other cores can only help ahead of time. Retrofitting parts, future events and invalidation into a finished engine would touch everything; designing them in costs little now.

**Rejected.** Never rendering ahead (one core in the browser for ever). Rendering ahead without invalidation (an edit would be heard late, or the render would depend on timing). Treating an edit inside the rendered-ahead span as late (it would be applied at another frame than asked, though there is still time to honor it).

**Consequences.** Engine: the parts and their live or ahead mark in the plan; restoring a part's state at a frame. Patch format: none. Hash: unchanged. Tests: the law over random patches and random event arrivals, with invalidations at and around part boundaries; a part depending on a live source is always marked live.

**Step.** #12 step 34. **Order:** 14 (future events at 6).

## 23. Seeded randomness

**Decision.** Randomness is a pure function of the patch's **seed**, the box and the instant: `random(seed, stream, frame, draw)`. No generator holds state.

Sub-decisions:

- **The seed** is a 64-bit integer stored in the patch.
- **Streams from stable ids.** A box's **stream** is derived from the path of box ids from the top patch to the box (subpatches included), its channel or voice index, and a purpose index (a box with several random sources numbers them); never from creation order, call order or position in the document. The path's identity is the first 128 bits of the SHA-256 of its canonical text; the planner checks that no two streams of a plan collide and refuses the plan if they do, so two boxes never share a stream.
- **The generator** is **Threefry-4×64-20** (Salmon, Moraes, Dror and Shaw, "Parallel random numbers: as easy as 1, 2, 3", SC 2011): a counter-based generator made of 64-bit additions, rotations and exclusive ors only, so it is exact on every target. Key: the seed, the 128 bits of the path, and the channel, voice and purpose indices; counter: the absolute frame and the index of the draw within that frame (draws at one frame follow the total message order of decision 4). Each call gives 256 bits.
- **Exact conversion**: a uniform binary64 in [0, 1) is `(x >> 11) · 2^-53`, a binary32 one `(x >> 40) · 2^-24`; in (0, 1), for distributions that take a logarithm, `((x >> 11) + 1/2) · 2^-53`. Other distributions are computed from these with the engine's pure-Rust `libm`, like every transcendental function.
- **New seed live.** A live "new seed on every play" mode draws a seed from the host's entropy when playback starts, writes it to the event log (decision 19) and shows it, so the take can be replayed, kept in the patch or exported exactly.
- **Changing the generator or its derivation** changes renders, so it takes a new patch format version, with migration (a version 1 document keeps the generator it was made with).

The laws: the same seed gives the same bits; different seeds give different streams; adding, removing or renaming a box leaves every other box's random values unchanged; a box's values at a frame do not depend on what was rendered before (so seeking, rendering ahead, several threads and freezing one part need no replay of randomness).

**Why.** Randomness tied to stable ids and instants is reproducible however a patch is edited, split, seeked or scheduled; a stateful generator ties every value to everything drawn before it.

**Rejected.** A stateful generator per box or per patch (seeking and rendering ahead would need replay, and one more box would shift everyone's values). Streams from creation or call order. Platform or library randomness (it differs between targets). Hashing for each draw with a cryptographic hash (correct, but far costlier per value than a generator built for it). Converting integers to floats by division (rounding differs from the exact shift).

**Consequences.** Engine: the generator in the core, the stream key computed when the plan is built, a per-frame draw index in the state of a box that draws. Patch format: the seed (ticket #3). Hash: unchanged; a conformance case states its seed. Tests: the generator against the published known-answer vectors of Threefry-4×64-20, so the published statistical batteries apply to it bit for bit; the laws above over random patches and edits; the conversions exact at their extremes (0, the largest value below 1); white and other noise boxes measured against derived tolerances (mean, variance, spectral flatness, autocorrelation, `spec/measurements.md`); a stream collision refused.

**Step.** #12 step 18. **Order:** 6.

## 24. The seek law

**Decision.** **A render that jumps to frame `t`, with every box's state restored for `t`, equals from `t` on a render that played from frame 0**, bit for bit. The state for `t` comes either from a snapshot taken at `t` (decision 14) or from the nearest earlier snapshot (or frame 0), replayed to `t` with every event before `t` applied and its output discarded.

Sub-decisions:

- "Every box's state" includes the engine's own state: ramps in progress (decision 5), compensation delays (decision 13), crossfades (decision 15), voice allocation (decision 7), the transport position (decision 17), held messages (decision 21); the engine's state is serializable like a box's.
- A box may offer a faster way to reach `t` (an oscillator computing its phase at `t` directly, decision 9); the law checks it against replay.
- Across plan swaps, the patch in effect at `t` is the one the event list or the log (decision 19) installed before `t`.
- Randomness needs no replay (decision 23); every other state does, which is why snapshots exist.

The resume law of decision 14 is this law at the frame of the snapshot.

**Why.** Seeking a timeline, rendering ahead with invalidation (decision 22), parallel rendering, freezing, and tests that cover hours of signal all depend on it; stated as one law, it is proven once for all of them.

**Rejected.** Starting boxes fresh at `t` (a filter, a delay or a reverb tail would differ from continuous playback). A seek that is only "close" to continuous playback (no bit-exact guarantee could survive it).

**Consequences.** Engine: seeking in the renderer (snapshot, then replay); the engine's own state in snapshots. Patch format: none. Hash: unchanged. Tests: the law over every box and over random patches with events and swaps, at random `t` (0, at and around engine block and host buffer boundaries, after hours of signal by skipping); each box's fast path against replay.

**Step.** #12 step 29. **Order:** 12, with state.

## 25. The box contract

**Decision.** The **box contract** is public and versioned, and the same for built-in boxes and **externals** (box types written by third parties as WebAssembly modules): built-in boxes are externals compiled into the engine. It is defined as a **stable ABI** that a WebAssembly module can implement, and the Rust trait of built-in boxes is one binding of it.

Sub-decisions:

- **Contents**: metadata (type name, contract version, ports with their payload kinds (a signal, a message kind, or sub-sample discontinuity events, decision 31), parameters with kind, range, unit, default and smoothing, latency, state version and maximum size, execution mode (decision 20), the maximum messages emitted per message received, the resources it reads and how, decision 26, its instantaneous response for delay-free loops: none, linear or non-linear with its derivative, decision 3, and the capabilities it asks for: none by default); a life cycle (prepare at a rate, precision and maximum span, where every allocation happens; process a span with its inputs, outputs and events; reset; save and restore state).
- **The ABI**: functions with integer and float arguments only; every buffer exchanged by offset and length in the module's own memory, which the engine fills and reads; no pointer into the engine; every float that leaves a module has its NaN made canonical, since WebAssembly leaves NaN payloads open. Versioned major.minor: a module declares the version it implements, the engine supports a declared range, and a minor version only adds. Relaxed SIMD and threads are refused when a module is loaded (their results may differ between runtimes).
- **Buffer layout**: every signal buffer handed to a box starts on a 64-byte boundary and is padded to a multiple of 64 bytes (the widest vector considered, 512 bits), the padding written as zeros by the engine and never read back from a box's outputs; the contract states it, so externals may rely on it. It changes no bit.
- **Compile-friendly**: a box's processing is a function of its explicit state, its inputs and its events, and of nothing else (no global, no static mutable value), so code generated at build time (decision 20) can call and inline it, and it gives the same bits whether called by the plan or by generated code.
- **Routing is resolved when the plan is built**: sends and receives by name, buses, watched cables and named destinations all become direct edges in the plan; nothing is looked up by name while processing. Changing a route is a patch edit, played through a hot swap (decision 15).
- **Externals are referenced by content**: a patch names an external by the SHA-256 of its module, with a readable name, so the same patch always plays the same external.

**Why.** One contract used by every box means no first- and second-class boxes, and every law applies to externals as it applies to built-in boxes. A contract that only exists as a Rust trait cannot be implemented in another language or crossed by a WebAssembly boundary; an ABI can. Explicit state and routing resolved in advance are what let a frozen patch compile to direct code.

**Rejected.** A Rust-only contract (no externals, no other languages). Native libraries as externals (one build per platform, no browser, no sandbox, no determinism). Pointers across the boundary (unsafe, and meaningless between memories). Run-time lookup of routes by name (cost and hidden state while processing, and nothing to compile).

**Consequences.** Engine: the contract as an ABI with its version; the Rust trait as its binding; aligned and padded buffers; routing resolution in the planner; NaN made canonical at the boundary. Patch format: external references by hash and name (ticket #3). Hash: unchanged. Tests: every built-in box run directly and through the ABI form (hosted in a test harness by a WebAssembly runtime added at this step, with its entry in the dependency register), giving the same bits; the box laws on both; a module of an unsupported version, an invalid metadata or a forbidden feature refused with its reason; every buffer handed to a box aligned and padded, its padding zero; routes by name resolved before the first frame; the law that generated code equals the plan lands with the build's compiler, on a later card.

**Step.** #12 step 13. **Order:** 6, first: the contract is shaped with events, before boxes are written against it.

## 26. Resources

**Decision.** **Resources** (samples, tables, impulse responses) are immutable data a box reads, identified by the **SHA-256 of their source bytes**, with a readable name.

Sub-decisions:

- **Prepared outside the audio thread**: the host reads the bytes, then decodes them and resamples them to the session rate with Rust code that is the same on every target; the browser's own decoding is never used (its output is not specified bit for bit). The resampler is a rational polyphase filter specified by its coefficient tables, like the subpatch resampler of decision 2, and measured the same way. What a box reads is a function of the source hash, the decoding version, the rate and the precision.
- **A table per engine instance**, no global state. A resource is immutable once handed to the engine; replacing it means preparing the new one beside the old one and **swapping it at a frame**, as an event, between two blocks, with no allocation and no lock on the audio thread; the old one is freed off it.
- **Boxes never know where the bytes are.** A box reads through a handle, resolved when the plan is built: `read(handle, channel, first frame, count)` copies frames into the box's buffer, whether the resource is in memory or streamed. A box declares its access as **linear** (may be streamed) or **random** (must be in memory); the host chooses the memory mode, and the mode never changes which bytes are read.
- **Defined at every edge**: frames outside the resource read as zero; a missing resource reads as one of length zero and is reported; a streaming underrun (live only) reads zeros, is reported and logged (decision 19), so its replay gives the same bits. Offline the renderer waits for the data and never underruns.
- **Across the ABI** (decision 25), reading is an imported function that copies into the module's memory, bounded by the maximum span; a module never sees the resource's memory.

**Why.** Content addressing makes a patch portable and its inputs exact; decoding with one code everywhere makes the samples a box reads identical on every target; reading through a handle lets streaming arrive later without touching a box.

**Rejected.** Paths in patches (machine-specific, and a file can change under the same path). Decoding with the platform's decoder. Boxes holding a reference to resource memory (streaming would need every box to change, and the ABI could not offer it). Mutable resources (a change in the middle of a block would depend on timing).

**Consequences.** Engine: the resource table, handles, the read call, swaps as events, the underrun report. Host shells: decoding and resampling. Patch format: a resources section (hash and name) and box arguments that name a resource (ticket #3). Hash: unchanged; a conformance case carries its resources' bytes. Tests: the resampler's measurements with derived thresholds (passband ripple, stopband rejection, aliasing); reads at the edges (first and last frame, beyond, zero length); renders identical with the resource in memory and streamed; a swap applied at its exact frame; an underrun logged and replayed bit for bit; bytes whose hash does not match refused; no allocation in a read or a swap.

**Step.** #12 step 31. **Order:** 12, after hot swap (the same swap mechanism); the read call in the contract at 6.

## 27. A deterministic FFT

**Decision.** The engine has its own FFT, in the core (`no_std`, safe Rust), with **one code path on every target**: no detection of processor features at run time; no fused multiply-add; the order of every floating-point operation fixed by the source. An iterative Stockham FFT (radix 4, with a final radix-2 pass) for powers of two; any other length through Bluestein's algorithm on a power-of-two FFT. **Twiddle factors** are computed when the plan is built, with the pure-Rust `libm`, from angles reduced exactly in integers to the first octant (so `k / N` is never rounded) and unfolded by symmetry; generic over the sample type, the binary32 twiddles rounded from binary64.

**Why.** Spectral subpatches (decision 2), convolution, analysis and linear-phase filters need an FFT, and a general-purpose library that picks its code path from the processor, or takes its twiddles from the platform's trigonometry, can round differently on two machines. Owning it keeps the core promise and lets its error be derived.

**Rejected.** A general-purpose FFT library that selects SIMD code at run time. Such a library restricted to its scalar path (its twiddles may still come from the platform's math). Twiddle tables stored in the repository (a second source of the same fact, and large for long transforms). Sharing the FFT of the measurements (`engine_testkit::measure`): the instrument would then measure itself.

**Consequences.** Engine: the FFT in the core, its twiddle tables allocated when the plan is built. Patch format: none. Hash: unchanged. Tests, with the steps of `spec/measurements.md` for an engine component: its error against a direct transform computed in higher precision, within a bound derived term by term (as the FFT term of `spec/measurements.md`), for every power of two up to the maximum and lengths that are prime, odd and composite; inverse after forward within a derived bound; impulses and sines on bins exact to their bound; the same bits natively and in WebAssembly; its cost counted per length by the benchmarks of `spec/benchmarks.md`.

**Step.** #12 step 25. **Order:** 10, before spectral subpatches.

## 28. Reduction order

**Decision.** **Every sum of more than two terms** that a box or the engine computes in one place (mixing, cables into one inlet, buses, FIR filters, convolution, dot products, the sums inside an FFT where there are any) follows one specified, vectorizable order:

- the terms are numbered `k = 0 … n-1` by the order the decision concerned already fixes (cables in their fixed order, decision 6; taps by index; senders by box id; the output boxes summed onto one channel by box id, like buses, whatever the cables that set the execution order);
- term `k` goes to **lane** `k mod 16`; each of the 16 lanes sums its terms in increasing `k`, starting from -0 (the exact identity of IEEE 754 addition: `x + (-0)` is `x` for every `x`, -0 included);
- a sum of no terms is +0;
- the 16 lane sums are combined by a fixed pairwise tree: lane `i` with lane `i + 8`, then `i` with `i + 4`, then `i + 2`, then `i + 1`, for `i` from 0.

The lane count is a constant of the specification, 16, never the SIMD width of the machine: any vector width that divides 16 (1, 2, 4, 8 or 16 lanes, whatever the sample type) computes exactly this order, so vectorizing a loop later, or running it on a wider machine, never changes a bit. One shared helper in the core implements it; no box writes its own long sum.

**Why.** Floating-point addition is not associative: a sequential sum cannot be vectorized without changing its result. Fixing a vector-shaped order now costs nothing in bits today and keeps every later vectorization free.

**Rejected.** Left-to-right sequential sums (vectorizing them later changes bits). An order that follows the SIMD width of the target (bits would differ between a WebAssembly build with 128-bit vectors and a native one with 256- or 512-bit vectors). Compensated or exact summation everywhere (correct, but several times the cost of every mix; a box that needs it, such as a measurement, uses it explicitly).

**Consequences.** Engine: the reduction helper in `engine-core`'s math layer, `ordered_sum`, used by every sum of more than two terms; the plan sums the cables into an inlet and the output boxes on a channel a block at a time with the same core (one generic implementation of the rule over a sample or a block, so each frame of a block sum is the sample sum of its frames by construction), with one set of scratch lanes (15 blocks) for the whole plan, reserved when it is built and reached only by a sum of three or more terms, so that what a plan reserves for its sums does not grow with their number (a patch within every limit, with one set per sum, would make the engine reserve about a gigabyte); the plan decides when it is built how each inlet and each output channel is fed, from its sources in their order: none leaves it silent, one is a plain copy (the one-term result, bit for bit, at the cost of a copy), two are added frame by frame, the first plus the second (the two-term result: lanes 0 and 1 joined by the last step of the tree, so no lanes are needed), three or more go through the sum, a choice held in the plan's structure and tested there (`decides_how_each_block_is_fed_when_built`), never a branch taken while processing; the plan laws bound what building a plan keeps for its sums, against the same boxes fed by one cable per inlet, to one set of lanes and a word per cable, at the patch limits included (`reserves_one_set_of_lanes_for_every_sum`, counted by the counting allocator); decision 16's fixed summation order is this order applied to the terms its tasks produce. Adding -0 changes no value but a NaN's bits, so the helper starts each lane from its first term and leaves empty lanes out of the tree: the same bits as the rule for every result but a NaN, one term passed bit for bit; a NaN result stays a NaN (one term apart, returned bit for bit), whose payload and sign are not reproducible: natively the payload of one of the NaN terms (quieted) or the default NaN, in WebAssembly any quiet NaN when a NaN term is not canonical (its specification leaves it open), and either may change with the build, the compiler being free to swap the operands of an addition (roadmap #24 step 2 decides how sums treat NaN). **Enforced by tooling**: the sample types implement neither `Sum` nor `Product`, so an iterator of samples cannot be summed otherwise; xtask's math layer gate refuses, in box code and in `engine-core` outside its math layer (tests apart), the iterator reductions and their traits (xtask's `REDUCTION_NAMES`); every module of `engine-core` but its root and its math layer forbids Clippy's `arithmetic_side_effects` outside tests (`#![cfg_attr(not(test), forbid(clippy::arithmetic_side_effects))]`, a line the gate requires), so the compiler refuses any exception to it there, by its name, its former name or a group (E0453), and the gate refuses those names (xtask's `ENGINE_FORBIDDEN_NAMES`) in the root, which cannot forbid it since the math layer lifts it: the engine does no arithmetic on a generic sample type outside the math layer. **Review only**: an explicit loop or a chain of `+` summing more than two terms in box code, which text cannot tell from a recurrence advanced once per frame (a phase), and arithmetic on `Binary64` written in the engine outside the math layer (`clippy.toml` allows its operators). Patch format: none. Hash: renders with three or more terms in one sum change, a change that came before versions existed (decision 29: render rules version 1 is the rules after it); no conformance case has one whose bits depend on the order, so no reference changed. One case sums 17 cables into one inlet and 3 into the other (`sines-summed-from-17-and-3-cables`), every term but one of each a silent oscillator (+0), so that its expectation stays one sine with a derived bound: it proves the path of a sum (the first term copied, the lanes, the tree, the scratch reused across blocks) in the release build of every target, natively and in WebAssembly; the order of the roundings is proven by the unit tests against the literal reference. Accuracy: a term goes through at most `h = ⌈n/16⌉ - 1 + ⌈log2 min(n, 16)⌉` roundings (its lane's additions, then the tree): 1, 4, 7 and 260 for n = 2, 16, 64 and 4099, so a sum that does not overflow is within `h·u / (1 - h·u) · Σ|x_k|` of the exact one (`u` = 2^-53 in binary64, 2^-24 in binary32), where a sequential sum allows `n - 1` roundings. Tests: the helper against a scalar reference written literally from the rule above, and against the exact sum within that bound at every count (`sums_within_the_accuracy_bound_of_the_order`, whose counts of roundings `counts_the_roundings_of_each_regime` checks), for every `n` from 0 to past several multiples of 16, special values included (-0, infinities, NaN, subnormals), natively and in WebAssembly; each vectorized form, when one is added, against the same reference; a plan law over the cables into an inlet and the output boxes on a channel, up to 49 (a sum of three cables differs from the sequential sum on chosen inputs, and must match the rule); sums inside box code are review-only, as above.

**Step.** #12 step 7. **Order:** 3, with step 6.

## 29. Box algorithm version

**Decision.** Every box in a patch carries the **algorithm version** of its box type. Any change of a box type that can change a bit gives the box type a new algorithm version, with no exception. A patch keeps the version it was made with: the engine keeps playing every supported version of a box type, and moving a patch to a newer version is an explicit migration, shown to the user, never a side effect of an update. Approximations stay allowed only when they are measured faster and within the box's derived thresholds.

Sub-decisions:

- **What changes a bit.** A change changes a bit of a box type when one input could render different bits from its outlets after it than before it: any patch the engine accepted before, with every parameter value, input signal, rate and length that patch allows, in either precision, in every arithmetic mode the engine builds and ships (the strict mode alone today). A NaN that stays a NaN with another payload is not a bit change (decision 28 leaves payloads open). Counted as a change of the box type: its own code, the math-layer functions it calls (`libm` included), its declared latency (decision 13 compensates other paths with it), its parameter handling and defaults, and a change in one precision only. Not counted: a change proven bit-identical (an optimization whose fingerprint, below, is unchanged), behavior no existing patch can reach (a new parameter, argument or inlet whose default gives the old bits exactly), and a change of the hash format alone, which changes references but no sample.
- **Three kinds of bit change.** (1) **A box type's change** takes a new algorithm version of that box type. Successive changes of one box type within one branch share one new number; a number is spent when it reaches `main`, never decreases and is never reused. (2) **An engine-level change**, the bits of a render no box type owns (the plan's sums and their order, when events and reduced-rate updates take effect, flush points, the output stage, rounding at delivery, how parameters are read into binary64), takes a new **render rules version**: one integer of the engine (`engine_core::RENDER_RULES_VERSION`, from 1, the rules after step 7), stored in format version 2 beside the boxes' versions. The engine plays only its current render rules; a patch made under older ones is migrated explicitly, shown to the user with what changes, never silently; old engine-level code is kept only where that is cheap. Box changes never move it and migrate nothing, so it is not the rejected global engine version. (3) **A hash-format change** (`render-hash/N+1`) re-blesses every reference and moves no version.
- **Which versions stay playable.** A version is **supported** from the moment a build of `main` can write it into a document (from the merge of format version 2 on), until this specification **retires** it, an explicit decision kept for a version that breaks a guarantee (non-finite output, a real-time rule). A patch naming a retired version is refused with its reason and offered the migration, never silently remapped. Versions that only existed before format version 2 reached `main` were never supported: no document can name them, and their code goes when the version moves on. A format version 1 document names no version: its migration records the current ones, so it follows the current versions until it is saved as version 2.
- **`libm` and the math layer.** `libm` is pinned exactly in the workspace manifest, like every **bit-defining** dependency: one whose code computes output samples or hashes, or the values they are computed from, marked by a reason starting `bit-defining: ` in `dependencies.txt` (every normal or build dependency of the audio library is one, and `dependencies.txt` is where they are listed), each required as `=X.Y.Z`, which xtask's pin gate checks for the direct dependencies it reads (the derive that generates the patch reader, `serde_derive`, is one, required directly for that reason); the code those reach in turn (`sha2`'s `digest`, `block-buffer`, `crypto-common`, `hybrid-array` and `cpufeatures`) is not pinned by a manifest but fixed by `Cargo.lock`, which every check builds with (`--locked`), and its bits are proven by the render hash vectors, the conformance hashes and the fingerprints, so a lock change of one of them is read as an update; an update of any of them follows the rules of this sub-decision. An update that changes a bit of a box type's renders is a change of that box type, which takes a new version; once that box type's older version is supported, the update is taken only if the old functions stay for the old version (the math layer keeps them, vendored), else it is not taken. The same holds for `mul_then_add`, `flush` and every math-layer function in the strict mode; `ordered_sum` is fixed by decision 28 and never changes. Dependabot keeps proposing updates: its pull request changes the exact pin visibly, and conformance with the fingerprints decides.
- **Fingerprints.** Each box type has a deterministic **fingerprint** per algorithm version: the SHA-256 of its outputs over a fixed battery (both precisions, every sample rate of the test matrix, the parameter sets the box laws share, given as `engine_core::Params` as a plan gives them, the matrix's special inputs and a seeded random signal), computed by `engine_battery::fingerprint`. It sees a bit change where no conformance case reaches (a `libm` update outside the corpus's inputs), and proves a claimed bit-identical change. The battery has its own version (`engine_battery::FINGERPRINT_BATTERY`), raised with any change of the battery or of the inputs it reads (a test pins their digest beside it), which replaces every fingerprint. A box type without outlets has the battery's fingerprint alone: what it does reaches a render only through the inlets the plan reads (the output boxes), which the conformance cases check.
- **What bless refuses.** `cargo xtask bless` (`engine-cli bless`) writes nothing, and names every offending case, format or box type, when: (1) a reference of an existing case changes while its box types' versions, the render rules version and the render hash version all stayed as recorded (a new case is always accepted), or a case holds a reference but no versions in the record bless replaces; (2) a box type is at a lower version than recorded (in the record it replaces or the one at the merge base), or the render rules version decreased; (3) the fingerprint of a box type changes at a spent version: one the record at the merge base holds (`cargo xtask bless` reads it with git, the merge base of `HEAD` as the default check measures it, and hands it to `engine-cli bless --spent`), so that successive changes on a branch re-bless the one new number they share; `engine-cli bless` alone, without that record, spends every version recorded; (4) the fingerprint battery changed, unless `bless --battery-changed` says so, a bless that may then change no reference, add no case and record no new version, so that a battery change is a commit of its own; (5) either file is missing, or both are, unless `bless --from-nothing` says it is a first bless (both missing then): a file is never removed to bless past a refusal. For a case whose own files changed on purpose, `bless --case-changed <name>` (repeatable) accepts any change of that case's references and still refuses the others; the commit says why, as the conformance rules require. Every name it gives must be a case. Once format version 2 holds versions, a case whose patch pins its versions never changes reference but with the hash version, and a case naming an unsupported version is refused by `bless` and `check` alike.

**Why.** Optimizing a box must never change how an existing patch sounds behind its author's back; versioning each box's algorithm lets boxes improve while every saved patch keeps its bits, and the render identity covers it through the patch. The engine's own rules change rarely, and a render rules version keeps the same promise for them without migrating every patch at each box change.

**Rejected.** Silent algorithm changes (a patch would sound different after an update, and its reference hash would change for no visible reason). One global engine version instead of per-box versions (improving one box would migrate every patch). Never changing an algorithm (no box could ever become faster or better). Engine-level bit changes left unversioned, recorded only as decisions and changelog entries (an existing patch could change sound at an engine update). A `libm` pinned by the lockfile only (a routine update could change bits at an unchanged version).

**Consequences.** Engine: an algorithm version in every box type's metadata (decision 25; `ObjectSpec::algorithm_version`, an `AlgorithmVersion` counted from 1, which a box type cannot leave out), and a box type able to provide each version it still supports (format part); the render rules version, `RENDER_RULES_VERSION`. Conformance, engine side (#12 step 8): `cargo xtask bless` records beside the references (`conformance/algorithms.json`) the render hash version, the render rules version, the fingerprint of every box type at each version it was blessed at, and the algorithm version of every box type each case uses; the conformance check refuses a case whose box types are at other versions than its references were blessed with (a version bumped without a re-bless), names a reference mismatch under the same versions as bits changed without a new version, refuses another render hash version, another render rules version or a changed fingerprint at a recorded version, a missing or stale record, and a record not exactly as bless writes it; bless refuses as above. Patch format: the algorithm version per box and the render rules version, in format version 2, with a migration from version 1 that records the current versions (format part: #3 step 7); until then a patch names no version, every box plays the current version of its type, and the versions a case was blessed with are those of the conformance record. Hash: unchanged in form; a case pins its versions through its patch once the format holds them, through the conformance record until then. Tests: bless refuses each of its cases above on a fake corpus and writes nothing, accepts a new case, a version raised and the named changed case; the check refuses a changed fingerprint; a fingerprint is stable across runs and changes with a perturbed box; each supported version keeps its own cases; a patch naming an unsupported version is refused with its reason; migrations tested like every format migration.

**Review only.** The escape `bless --case-changed`, whose commit must say why the case's own files changed; `bless --battery-changed`, whose commit must change no box type, no math-layer function and no bit-defining dependency (the bless refuses a changed reference or a new version, but cannot see a box change no case reaches, since every fingerprint changes with the battery); `bless --from-nothing`, for a new conformance directory only; a claim that a change reaches no existing patch (a new parameter's default); a version number spent on `main` and never reused, beyond what the recorded fingerprints refuse.

**Step.** #12 step 8 (engine side, done); #3 step 7 (format version 2). **Order:** 3; the format part with #3.

## 30. Silence skipping

**Decision.** A box or a part of the plan may **skip processing** a span only when skipping is **proven bit-identical** to processing it. A box type declares, in the box contract, the condition under which its output stays exactly zero (typically: its inputs exactly zero, its state exactly zero, its tail finished; decision 11's flush makes a decaying recursive state reach exact zeros); the engine skips the box while that condition holds and writes exact zeros to its outputs (zeros of the sign processing would have produced). A box that declares nothing is always processed.

**Why.** Silent voices, idle effects and finished tails are most of a large session; skipping them is the largest single saving, and the only safe version of it is one that cannot change a bit.

**Rejected.** Sleeping below a level threshold (inaudible, but the bits change, and they change with the threshold). Letting the host decide when a box is silent (it cannot know the box's state).

**Consequences.** Engine: the silence condition in the box contract (decision 25), skipping in the plan. Patch format: none. Hash: unchanged. Tests: a law over every box and random patches: every render is identical with skipping on and off, at and around the frames where a box falls silent and wakes; the sign of each zero checked.

**Step.** #12 step 23. **Order:** 8, with the cost estimate (the condition in the contract at 6).

## 31. Sub-sample discontinuities

**Decision.** A box may send, beside its samples, **discontinuity events** on a typed port: each a step (or a kink) of its signal at a **sub-sample position**, the frame it falls in and its fraction of that frame, with the size of the step (and of the change of slope for a kink). The fraction is an unsigned 32-bit integer in units of 2^-32 of a frame, computed exactly from the box's integer phase (decision 9), so it is the same on every target. A receiving box uses them to correct its own output: an oscillator hard-synced to another resets its phase at the exact sub-sample instant and inserts a band-limited step (a BLEP, or its integrated form for a kink) for the jump it makes, so hard sync stays alias-free across boxes.

Sub-decisions:

- **Bounded and allocation-free**: a box declares the most discontinuity events it sends per frame; the port's buffer is sized when the plan is built, one span's worth; nothing is allocated while processing.
- **Deterministic**: the events of a frame are delivered in increasing position, ties in the order the sender emitted them; a receiver consumes them in that order.
- **Typed**: a discontinuity port connects only to a discontinuity inlet (decision 21's typed ports); no conversion from a signal is implied.

**Why.** Hard sync, band-limited discontinuities and other per-edge corrections need the instant of an edge finer than a frame; carrying it explicitly keeps them alias-free and exact across boxes, where reconstructing it from samples is approximate.

**Rejected.** Reconstructing the edge from the samples (approximate, and different for each receiver). Oversampling the whole patch for sync (costly, and still aliased above the new Nyquist). A floating-point sub-sample position (its rounding would depend on how it was computed).

**Consequences.** Engine: the discontinuity port type and its bounded buffers; the box contract's port kinds (decision 25). Patch format: cables between discontinuity ports (ticket #3). Hash: unchanged. Tests: hard sync of two oscillators against a band-limited reference, measured (aliasing level, THD) with derived thresholds, across the rate matrix; positions at 0, at the last unit of a frame and between; the bound per frame at its limit and one past it; the same bits natively and in WebAssembly.

**Step.** #12 step 19 (hard sync), the port kind in the contract at step 13. **Order:** 7, with the oscillators.

## 32. Standalone libraries

**Decision.** The **audio library** is a separate library, usable as it is: its crates (the crates of xtask's `RUST_CRATES` holding `CrateRole::Library`) need nothing else of the project, neither the editor nor the coordinating layer nor any sibling library (decision 1). A sibling library (the visual one when it comes) stands alone the same way. The **coordinating layer** sits above the libraries and uses each only through its public interface: time alignment between them, the choice of clock, scenes and cues that span several of them.

Sub-decisions:

- **Dependencies point down only.** Within the project, a crate of the audio library depends only on other crates of the audio library (its tests also on `engine-testkit`); never on the native shell, the tools, the apps, the coordinating layer or a sibling library. Its third-party dependencies are reviewed (`dependencies.txt`) and, where bit-defining, pinned (decision 29).
- **Time stays the library's own.** The audio library counts frames (decisions 1 and 4). **Open, not settled now**: whether the libraries share one tiny time-base crate, or each keeps its own exact time with a converting bridge in the coordinating layer (decision 19 already converts times at that boundary by one written rule). Decided when a second library needs it, never by a dependency taken on the way.
- **Kept possible, nothing built now**: publishing the library's time position to the layer above; following an external clock; timestamped output streams to the layer above (the return path, decision 19). None of them may need the library to know what is above it.

**Why.** A library that needs nothing above it can be used, tested, proven and published alone, and its guarantees (bits, real time, cost) do not depend on code it cannot see. Coordination across domains belongs to the one layer that sees them all, so no library grows types or costs for the others.

**Rejected.** One engine that owns every domain (decision 1's domain-agnostic core, rejected there). Libraries calling each other directly (every pair coupled, and the same alignment rule written in several places). A shared time base imposed now (decided before any second library exists to test it against).

**Consequences.** Engine: none at run time. Tooling: xtask's library gate refuses a path dependency of a library crate on anything outside the library (`engine-testkit` allowed in tests), and the dependency and pin gates review and pin its third-party crates. Patch format: none. Hash: none. Tests: the library gate's fixtures; the public interface of #32 is the only one the coordinating layer may use.

**Step.** #32 step 2, and the public interface of #32; the box contract of #12 step 13 is one of its public interfaces. **Order:** across every step: each step keeps the library standalone.

## 33. No cost for an architecture choice

**Decision.** **No architecture choice may cost performance.** Libraries, their public interfaces and the layer above cost nothing measurable against one monolithic engine, and that is **proven by measurement**, never assumed: today the instruction counts hold them to the direct counts within the counts' tolerance; each interface comparison, when it lands, is exact or within a per-call allowance derived and written for that interface.

Sub-decisions:

- **One program.** Libraries are crates linked into one program, and into one WebAssembly module in the browser: never processes or separate modules at run time, so a call between them is a function call the compiler can inline.
- **Interfaces designed for speed.** Calls take whole blocks, never a call per sample across a boundary; buffers are borrowed slices, not copies; queues between threads are lock-free (decision 19); generics, not dynamic dispatch, on hot paths: a live or offline plan calls each built-in box type directly (an enum over the registry, `engine_objects::BuiltIn`), and only a box type loaded at run time through its virtual table, one indirect call per box per block, never per sample; none in build mode, which specializes the whole patch (decision 20). Direct dispatch was measured against the indirect call it replaced (`spec/benchmarks.md`, "Same cost through every interface"): 3 to 10 instructions less per box per block, for plan loading 0.03% to 0.05% dearer and a module 1.3 KB larger, the price accepted for a faster real-time path, which comes first.
- **The build sees through the libraries.** Each library exposes its compiled plan, so the build mode (decision 20) can specialize the whole patch across them.
- **Same cost directly and through an interface.** A box processed through the box contract's ABI (decision 25), and the engine driven through its public interface (#32), cost what they cost directly, as they give the same bits: the instruction counts of `spec/benchmarks.md` measure both, a bulk copy counted by the words it moves, so an interface that copies its blocks pays for them in the counts; the comparison is exact, or within a per-call allowance derived and written for the interface (the ABI's canonical NaNs, decision 25, are work per sample), never within the counts' tolerance alone, which would let an interface cost up to that tolerance. The precedent: the sample newtypes `Binary64` and `Binary32` (decision 10) added no instruction measured when they replaced plain floats, and the benchmarks hold them within the counts' tolerance (an exact comparison is #24 step 86).

**Why.** Real time is a hard constraint, and a guarantee bought with a slower engine would push users back to patches that do less. An abstraction whose cost is not measured is paid for in every block of every patch.

**Rejected.** Separate processes or modules per library (copies and context switches at every block). A call per sample across an interface, and dynamic dispatch per sample (indirect calls the compiler cannot inline, on the hot path); a box type loaded at run time, which no build can know, is called once per box per block through its virtual table, the one exception above. An indirect call for every box to keep plan loading a few hundredths of a percent cheaper (measured, above). Accepting an interface's cost as the price of modularity (it would be paid by every user, forever).

**Consequences.** Engine: block-sized, borrowing interfaces; the compiled plan exposed. Benchmarks: every box counted directly and through the ABI (step 13), and the engine through its public interface (#32), compared with the direct counts exactly or within a written per-call allowance (`spec/benchmarks.md`, "Same cost through every interface"). Patch format: none. Hash: none. Tests: the benchmarks above, with their regression thresholds.

**Step.** #12 step 13 (the ABI at the same cost), #32 step 8 (the public interface at the same cost). **Order:** across every step: each interface lands with its cost measured.

## Implementation order

Each step lands with its tests, its format or hash version, and its conformance re-bless when bits change. The order follows dependencies; what changes every reference comes before more references are added.

1. **Step 2**, measurements: every later quality claim needs them.
2. **Step 3**, benchmarks: the cost of the decisions below is measured, not guessed.
3. **Steps 4 and 5**, subnormal numbers and subnormal parameters, then **step 6**, precision and `render-hash/2`, with **step 7**, the reduction order, and **step 8**, box algorithm versions: one re-bless of every reference, in both precisions.
4. **Step 9**, output stage: fixes what a render ends with.
5. **Steps 10 and 11**, host settings and the engine-block law, then outputs without channels, then **step 12**, execution modes (the mode is a host setting and the box contract's first new field).
6. **Step 13**, the box contract as a versioned ABI, first (every box is written against it); then **steps 14 and 15**, parameters, events, messages and rates, with **step 16**, message payloads, **step 17**, the control boundary (quotas, return path, the event log's hook) and **step 18**, seeded randomness (draws are addressed by frame and message order): everything after relies on events.
7. **Steps 19 and 20**, oscillators: integer phase, Nyquist policy, long-run drift.
8. **Steps 21 and 22**, the browser's audio thread (with the browser's transport of the control boundary), then the cost estimate, with **step 23**, silence skipping.
9. **Step 24**, multichannel cables and buses.
10. **Step 25**, the FFT, then **step 26**, subpatches with their blocks, and feedback (needs subpatches in the format, ticket #3).
11. **Step 27**, latency compensation (subpatch blocks and the limiter introduce latency).
12. **Step 28**, state, with **step 29**, the seek law, then **step 30**, hot swap (needs state), then **step 31**, resources (the same swap mechanism).
13. **Step 32**, polyphony (needs subpatches, events, multichannel and state).
14. **Step 33**, parallel scheduling, once the graph's shape is stable, with **step 34**, render-ahead (parts of the task graph, invalidation through the seek law).
15. **Steps 35, 36, 37**, the three hooks; external processors last (they need latency, state and rebuffering).
