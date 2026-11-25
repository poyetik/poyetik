# The box contract across a module boundary

How a box type compiled to its own WebAssembly module meets the engine: the ABI of decision 25 of [`engine-design.md`](engine-design.md), which says why it exists and what it contains. Built with #12 step 13. This document holds what is specified so far; what the contract's contents still lack is listed at its end. **Its major version is 0 until #12 step 18**, which makes it 1.0: until then nothing is promised between two versions.

The description of section 4 is common to every box type, whatever its body: what it is, its ports with the kind of each, its parameters, its version. Section 5 is the body of a box type whose ports are all audio, a WebAssembly module; a box type of another body (a shader, whose images stay on the GPU) is outside this version, which defines audio ports alone.

Every number that crosses the boundary has a fixed width. Offsets and lengths are unsigned 32-bit integers, in bytes, in the module's own memory; a memory's length is compared in 64 bits, since the largest is 2^32 bytes.

## 1. The version

A contract version is `major.minor`, two unsigned 16-bit integers, crossing the boundary as one unsigned 32-bit integer: `major × 2^16 + minor`.

The engine declares, for each major version it supports, the last minor version it knows. A minor version only adds (decision 25), so a module is accepted when its major version is declared and its minor version is at most the one declared for it. Major version 0 is the exception: nothing being promised inside it, a module is accepted only at the one minor version declared. It is refused otherwise, with the reason: a major version the engine does not support, another minor version of major 0, or a minor version later than the last the engine knows, which the refusal names.

## 2. The buffers

A **run of buffers** is what the engine hands a box for its inlets, or for its outlets: one buffer per audio port, in port order, each holding one span of samples in the precision of the render, little-endian.

- A run starts on a multiple of 64 bytes, and each buffer starts on one: a buffer's **stride** is the bytes of its span rounded up to a multiple of 64.
- A span is 1 to 2^20 frames, the sizes an engine block may have (decision 2).
- A run of no buffer is valid (a box without inlets or outlets) and takes no byte, at any aligned offset inside the memory or at its end.
- Refused, each with its reason: an offset that is not a multiple of 64, a span outside 1 to 2^20 frames, a run whose end is beyond the module's memory.

The bytes between the end of a span and the end of its stride are the **padding**: the engine writes them as zeros in the buffers it fills, and never reads them.

## 3. The canonical NaN

WebAssembly leaves the sign and the payload of a computed NaN open, so the same module may give two different NaNs on two machines. Every sample the engine reads from a module has its NaN made **canonical**: any NaN, whatever its sign and payload, quiet or signaling, becomes the positive quiet NaN with no payload, `0x7ff8000000000000` in binary64 and `0x7fc00000` in binary32. Every other value keeps its bits (zeros of both signs, subnormal numbers, infinities). The decision reads the bits, so that no floating-point mode of a host changes it.

## 4. The metadata

A module describes its box type in one run of bytes, found through its header (section 5) and read whole before any code of the module runs. Integers are unsigned and little-endian. A **text** is its length in bytes (32 bits), then that many bytes of UTF-8. A **list** is its count (32 bits), then its items.

| Field | Form |
|---|---|
| algorithm version (decision 29) | 32 bits, at least 1 |
| execution mode (decision 20) | 8 bits: 0 live-capable, 1 offline-only |
| instantaneous response (decision 3) | 8 bits: 0 none, 1 linear, 2 non-linear with its derivative |
| name | text, not empty, without whitespace |
| description | text, not blank |
| inlets, then outlets | two lists of ports: a name and a description, then the port's kind, 8 bits, then the fields its kind defines: 0 an audio signal, 1 audio sub-sample discontinuities (decision 31), neither with a field; 2 is reserved for spectral frames (decision 27), whose fields (the window size, the hop and the window shape) and buffers the minor version of #12 step 42 defines |
| parameters | list: a name and a description, then the bits of the default, the minimum and the maximum, in binary64 (decision 10) |
| capabilities | list, each 8 bits: 0 audio output |

A port's kind is what it carries, and its code fixes the fields after it: a later minor version defines a kind (another content, another pace, a content outside the module's memory) by a new code with its fields, and its buffers and calls with it (section 5); this version refuses every other code, 2 included, naming it. The name and the description of a port or a parameter are texts, not blank. Names are unique within the inlets, within the outlets and within the parameters, and no capability is listed twice. A parameter's minimum and maximum are finite, the minimum at most the maximum, and neither is below 2^-126 in magnitude but zero, since a value there is zero (decision 5) and none would reach such a bound; its width, the maximum less the minimum in binary64, is finite, which a ramp across the range needs and every binary32 range has; its default, made zero if it is below 2^-126 in magnitude, is within them. For binary32 values this is the rule of a built-in box (`ParamSpec::new` in `audio-core`).

The whole description is at most `METADATA_BYTES_MAX` bytes (`audio-core`, the largest size of a patch document), the one bound on it: nothing inside it has a limit of its own. It bounds the memory a hostile module makes a host use while it loads, off the audio thread, and is provisional. A length or a count announced beyond the bytes left is refused, and reserves nothing: texts are read in place and lists grow with the items read.

The bytes are refused, naming the field, when there are too many, when they end before it, when a number is none of those above, when a text is not UTF-8 or breaks the rules above, when a name or a capability repeats, when a parameter's range or default is refused, and when bytes follow the last field.

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

It may export other names, which the engine never uses. It has no start function: instantiating it runs no code.

**The header** lies in the memory as instantiation leaves it: the contract version (section 1, 32 bits), the length of the metadata in bytes (32 bits), then the metadata (section 4). An exported static of a language compiled to WebAssembly is a global holding its address, so a header is a static whose metadata follow its two integers. The engine reads it first, and refuses a header or metadata that end beyond the memory, then the version, then the metadata.

**The life cycle.**

1. The engine writes the parameter values at `poyetik_params()`: one binary64 per parameter, little-endian, in the order of the metadata, each within its range and none below 2^-126 in magnitude but zero, as a built-in box receives them.
2. `poyetik_prepare(rate, precision, span)`: the sample rate in hertz, 64 or 32, and the most frames a call will process, 1 to 2^20, the rate and the span of its audio ports, the only ones this version defines. It returns 0 when ready; any other value refuses, and is reported.
3. The engine reads `poyetik_inputs()` and `poyetik_outputs()`: runs of `span` frames, one buffer per audio port of the metadata, checked as section 2 says, the two not overlapping. They stay there until the next `poyetik_prepare`.
4. `poyetik_process(frames)`, `frames` from 1 to `span`: the engine has written the first `frames` samples of each input, with zeros to the end of its stride; the box writes the first `frames` samples of each output. What the outputs hold on entry is unspecified (decision 25). The engine reads those samples, each NaN made canonical.

A module whose memory grows, or that traps, outside `poyetik_prepare` outputs silence from that call on, is not called again, and is reported: an external's fault silences its box, never the host.

Refused when a module is loaded, each with its reason: bytes that are not a valid module; an import this version does not define; an export of the table missing or of another type; a start function; a memory that is shared or 64-bit; a feature whose results may differ between runtimes (relaxed SIMD, threads).

## Not specified yet

Events and parameter changes while processing (#12 step 41), messages (steps 15 and 16), randomness (step 18), latency (step 27), the state saved and restored (step 28) and resources (step 31) are contents of the contract (decision 25) that their steps add here.
