# The box contract across a module boundary

How a box type compiled to its own WebAssembly module meets the engine: the ABI of decision 25 of [`engine-design.md`](engine-design.md), which says why it exists and what it contains. Built with #12 step 13. This document holds what is specified so far; what the contract's contents still lack is listed at its end. **Its major version is 0 until #29 step 3**, which stabilizes the public interfaces of the engine, this contract among them, and makes it 1.0: until then nothing is promised between two versions.

The description of section 4 is what every box type says of itself, however it is built: what it is, its ports with the kind of each, its parameters, its version. A module gives it as these bytes; a built-in box type, described by `ObjectSpec` today, which has no port kind nor instantaneous response, takes its port kinds with #12 step 19 and its instantaneous response with #12 step 26. Section 5 is a box type whose ports are all audio, compiled to a WebAssembly module; a box type compiled to anything else (a shader, whose images stay on the GPU) is outside this version, which defines audio ports alone.

Every number that crosses the boundary has a fixed width and is little-endian. Offsets and lengths are unsigned 32-bit integers, in bytes, in the module's own memory; a memory's length is compared in 64 bits, since the largest is 2^32 bytes. A host that cannot hold a module's whole memory as one run of bytes (a 32-bit host, whose largest is 2^31 − 1 bytes, or a browser, where the engine and a module are two instances) reads it through a window and its offset, which #12 step 40 defines; until then the readers take the whole memory.

## 1. The version

A contract version is `major.minor`, two unsigned 16-bit integers, crossing the boundary as one unsigned 32-bit integer: `major × 2^16 + minor`.

**This version of the contract is 0.0**, the one version the engine declares (`host::CONTRACT_VERSION`, `host::SUPPORTED_VERSIONS`). The engine declares, for each major version it supports, the last minor version it knows. A minor version only adds (decision 25), so a module is accepted when its major version is declared and its minor version is at most the one declared for it. Major version 0 is the exception: nothing being promised inside it, a module is accepted only at the one minor version declared. It is refused otherwise, with the reason: a major version the engine does not support, another minor version of major 0, or a minor version later than the last the engine knows, which the refusal names.

## 2. The buffers

A **run of buffers** is what the engine hands a box for its inlets, or for its outlets: one buffer per audio port, in port order, each holding one span of samples in the precision of the render, little-endian.

- A run starts on a multiple of 64 bytes (`host::BUFFER_ALIGNMENT`), and each buffer starts on one: a buffer's **stride** is the bytes of its span rounded up to a multiple of 64.
- A span is 1 to 2^20 frames, the sizes an engine block may have (decision 2).
- A run of no buffer is valid (a box without inlets or outlets) and takes no byte, at any aligned offset inside the memory or at its end.
- Refused, each with its reason, the first that holds: an offset that is not a multiple of 64, a span outside 1 to 2^20 frames, a run whose end is beyond the module's memory, with the memory's bytes.
- A copy in or out of a buffer is of 1 frame to a span, the frames a call processes (section 5, step 4).

The bytes between the end of a span and the end of its stride are the **padding**: the engine writes them as zeros in the buffers it fills, and never reads them.

## 3. The canonical NaN

WebAssembly leaves the sign and the payload of a computed NaN open, so the same module may give two different NaNs on two machines. Every sample the engine reads from a module has its NaN made **canonical**: any NaN, whatever its sign and payload, quiet or signaling, becomes the positive quiet NaN with no payload, `0x7ff8000000000000` in binary64 and `0x7fc00000` in binary32. A subnormal number becomes a zero of its sign, the flush every box output has (`spec/engine-design.md`, decision 11), which the engine does not trust a module to do. Every other value keeps its bits (zeros of both signs, normal numbers, infinities). The decisions read the bits, so that no floating-point mode of a host changes them.

## 4. The description

A module describes its box type in one run of bytes, found through its header (section 5) and read whole before any code of the module runs. Integers are unsigned and little-endian. A **text** is its length in bytes (32 bits), then that many bytes of UTF-8, which hold no control character (C0, DEL and C1), no bidirectional formatting character and neither the line nor the paragraph separator: the characters a message escapes (`safe-message-vectors.json`), which would drive a terminal, forge a line or reorder what is shown. A text that is a **name** (of the box type, a port or a parameter) holds no default-ignorable code point either (Unicode's `Default_Ignorable_Code_Point`, a set its stability policy keeps), which shows nothing, so that two names that read alike would differ; a description text may hold them (a joiner shapes emoji and scripts). A text is **blank** when it shows nothing: empty, or whitespace and default-ignorable code points alone. Names are compared byte for byte: two names that differ only in their Unicode normalization are two names. One function holds these rules for a module's description and a built-in box type's alike (`host::text_fault`, which the box laws apply). A **list** is its count (32 bits), then its items.

| Field | Form |
|---|---|
| algorithm version (decision 29) | 32 bits, at least 1 |
| execution mode (decision 20) | 8 bits: 0 live-capable, 1 offline-only |
| instantaneous response (decision 3) | 8 bits: 0 none, 1 linear, 2 non-linear with its derivative |
| name | text, not empty, without whitespace |
| description text | text, not blank |
| inlets, then outlets | two lists of ports: a name and a description text, then the port's kind, 8 bits, then the fields its kind defines: 0 an audio signal, without a field; 1 is reserved for audio sub-sample discontinuities (decision 31), whose fields, bound of events per frame and buffers the minor version of #12 step 19 defines, and 2 for spectral frames (decision 27), whose fields (the window size, the hop and the window shape) and buffers the minor version of #12 step 42 defines |
| parameters | list: a name and a description text, then the bits of the default, the minimum and the maximum, in binary64 (decision 10) |
| capabilities | list, each 8 bits: 0 audio output |

A port's kind is what it carries, and its code fixes the fields after it: a later minor version defines a kind (another content, another pace, a content outside the module's memory) by a new code with its fields, and its buffers and calls with it (section 5); this version refuses every other code, naming it, and says that 1 and 2 are reserved (`host::RESERVED_PORT_KINDS`). The name and the description text of a port or a parameter are texts, not blank. Names are unique within the inlets, within the outlets and within the parameters, and no capability is listed twice. A parameter's minimum and maximum are finite, the minimum at most the maximum, and neither is below 2^-126 in magnitude but zero, since a value there is zero (decision 5) and none would reach such a bound; its width, the maximum less the minimum in binary64, is finite, which a ramp across the range needs; its default, made zero if it is below 2^-126 in magnitude, is within them. This is the rule of a built-in box type too, one function for both (`ParamSpec::new` in `audio-core`).

The whole description is at most `DESCRIPTION_BYTES_MAX` bytes (`audio-core`, the largest size of a patch document), and each list of ports at most `PORTS_MAX` ports (65,536, the ports a patch's 16-bit port indices name, so that every port is within a cable's reach): nothing else inside it has a limit of its own. The first bounds the memory a hostile module makes a host use while it loads, off the audio thread, and is provisional; how many times its size the reader asks of the heap is not established yet (#12 step 40). The second bounds the buffers a plan gives one box: at most twice `PORTS_MAX` buffers, each a span of samples, whose bytes the module's memory must hold too, so that a host checks a module's runs before it builds a plan for it, and allocates the plan's buffers so that a failure is refused and reported, naming the box, never an abort (#12 step 40, which measures both factors: the memory a description makes a box hold, its parameters bounded by its bytes alone, and the buffers its ports take). A length or a count announced beyond the bytes left is refused, and reserves nothing: texts are read in place and lists grow with the items read.

The bytes are refused, naming the field (an item of a list by its position from 0 and its part: its name, its description text, a port's kind, a parameter's values) and the values refused, when there are too many, when a list holds more ports than that, when they end before it, when a number is none of those above, when a text is not UTF-8, holds a character the rules above refuse, naming its code point, or breaks another of them, when a name or a capability repeats, when a parameter's range or default is refused, and when bytes follow the last field.

## 5. The module

A module is a WebAssembly module of one box type. **One instance of the module holds one instance of the box** (provisional: to be measured against one instance per box type before 1.0): its state is its memory and its globals, no other box shares them, and a module that traps takes no other box with it.

It exports, with exactly these types, what the table below lists. **In this version it imports nothing**: a later minor version defines the one import decision 26 gives a box, a function that copies a resource's frames into the module's memory.

| Export | Type | Meaning |
|---|---|---|
| `memory` | one memory, 32-bit, not shared | Where everything below lies |
| `poyetik_box` | immutable global, i32 | The first byte of its header |
| `poyetik_params` | () → i32 | The first byte of its parameter values |
| `poyetik_prepare` | (rate: i32, precision: i32, span: i32) → i32 | Makes the instance ready, where it may grow its memory |
| `poyetik_inputs` | () → i32 | The first byte of its run of input buffers (section 2) |
| `poyetik_outputs` | () → i32 | The first byte of its run of output buffers |
| `poyetik_process` | (frames: i32) → () | Processes one span |
| `poyetik_reset` | () → () | Returns to the state `poyetik_prepare` left |

It may export other names, which the engine never uses, but none starting with `poyetik_`: the prefix is the contract's, for the exports of its later minor versions. It has no start function: instantiating it runs no code.

**The header** lies in the memory as instantiation leaves it: the contract version (section 1, 32 bits), the length of the description in bytes (32 bits), both little-endian, then the description (section 4). An exported static of a language compiled to WebAssembly is a global holding its address, so a header is a static whose description follows its two integers. The engine reads it first, and refuses a header or a description that ends beyond the memory, then the version, then the description.

**The life cycle.**

1. The engine writes the parameter values at `poyetik_params()`: one binary64 per parameter, little-endian, in the order of the description, each within its range and none below 2^-126 in magnitude but zero, as a built-in box receives them (a default as the description gives it is flushed first). A region of 8 bytes per parameter that ends beyond the memory is refused, with the memory's bytes, before any value is written.
2. `poyetik_prepare(rate, precision, span)`: the sample rate in hertz, a whole number from 1 to 2^31 − 1 in this version (a box in a subpatch of a fractional rate cannot be a module of this version; a later minor version gives the rate exactly), 64 or 32, and the most frames a call will process, 1 to 2^20, the rate and the span of its audio ports, the only ones this version defines. It returns 0 when ready; any other value refuses, and is reported.
3. The engine reads `poyetik_inputs()` and `poyetik_outputs()`: runs of `span` frames, one buffer per audio port of the description, checked as section 2 says, the two not overlapping (`host::Runs`): runs that touch are accepted, and a run of no buffer overlaps nothing. They stay there until the next `poyetik_prepare`.
4. `poyetik_process(frames)`, `frames` from 1 to `span`: the engine has written the first `frames` samples of each input, with zeros to the end of its stride; the box writes the first `frames` samples of each output. What the outputs hold on entry is unspecified (decision 25). The engine reads those samples, each NaN made canonical and each subnormal number a zero of its sign (section 3).

A module whose memory grows, or that traps, outside `poyetik_prepare` outputs silence from that call on, is not called again, and is reported: an external's fault silences its box, never the host.

Refused when a module is loaded, each with its reason: bytes that are not a valid module; an import this version does not define; an export of the table missing or of another type; a start function; a memory that is shared or 64-bit; a feature whose results may differ between runtimes (relaxed SIMD, threads).

## Not specified yet

Events and parameter changes while processing, with each parameter's kind, unit and smoothing, and a module's driven body (#12 step 41), messages (steps 15 and 16), randomness (step 18), sub-sample discontinuities (step 19), silence (step 23), channel counts and layouts (step 24), how an instantaneous response gives its derivative, and a delay that lets a loop through the box, per path from an inlet to an outlet (step 26), latency (step 27), the state saved and restored (step 28) and resources (step 31) are contents of the contract (decision 25) that their steps add here.
