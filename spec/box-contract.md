# The box contract across a module boundary

How a box type compiled to its own WebAssembly module meets the engine: the ABI of decision 25 of [`engine-design.md`](engine-design.md), which says why it exists and what it contains. Built with #12 step 13. This document holds what is specified so far: the version, the buffers and the canonical NaN. The functions of a module, its metadata and its life cycle join it in that step.

Every number that crosses the boundary has a fixed width. Offsets and lengths are unsigned 32-bit integers, in bytes, in the module's own memory; a memory's length is compared in 64 bits, since the largest is 2^32 bytes.

## 1. The version

A contract version is `major.minor`, two unsigned 16-bit integers, crossing the boundary as one unsigned 32-bit integer: `major × 2^16 + minor`.

The engine declares, for each major version it supports, the last minor version it knows. A minor version only adds (decision 25), so a module is accepted when its major version is declared and its minor version is at most the one declared for it. It is refused otherwise, with the reason: a major version the engine does not support, or a minor version later than the last it knows, which the refusal names.

## 2. The buffers

A **run of buffers** is what the engine hands a box for its inlets, or for its outlets: one buffer per port, in port order, each holding one span of samples in the precision of the render, little-endian.

- A run starts on a multiple of 64 bytes, and each buffer starts on one: a buffer's **stride** is the bytes of its span rounded up to a multiple of 64.
- A span is 1 to 2^20 frames, the sizes an engine block may have (decision 2).
- A run of no buffer is valid (a box without inlets or outlets) and takes no byte, at any aligned offset inside the memory or at its end.
- Refused, each with its reason: an offset that is not a multiple of 64, a span outside 1 to 2^20 frames, a run whose end is beyond the module's memory.

The bytes between the end of a span and the end of its stride are the **padding**: the engine writes them as zeros in the buffers it fills, and never reads them.

## 3. The canonical NaN

WebAssembly leaves the sign and the payload of a computed NaN open, so the same module may give two different NaNs on two machines. Every sample the engine reads from a module has its NaN made **canonical**: any NaN, whatever its sign and payload, quiet or signaling, becomes the positive quiet NaN with no payload, `0x7ff8000000000000` in binary64 and `0x7fc00000` in binary32. Every other value keeps its bits (zeros of both signs, subnormal numbers, infinities). The decision reads the bits, so that no floating-point mode of a host changes it.

## 4. The metadata

A module describes its box type in one run of bytes, read whole before anything of the module runs. Integers are unsigned and little-endian. A **text** is its length in bytes (32 bits), then that many bytes of UTF-8. A **list** is its count (32 bits), then its items.

| Field | Form |
|---|---|
| algorithm version (decision 29) | 32 bits, at least 1 |
| execution mode (decision 20) | 8 bits: 0 live-capable, 1 offline-only |
| instantaneous response (decision 3) | 8 bits: 0 none, 1 linear, 2 non-linear with its derivative |
| name | text, not empty, without whitespace |
| description | text, not blank |
| inlets, then outlets | two lists of ports: a name and a description, then 8 bits: 0 a signal, 1 sub-sample discontinuities (decision 31) |
| parameters | list: a name and a description, then the bits of the default, the minimum and the maximum, in binary64 (decision 10) |
| capabilities | list, each 8 bits: 0 audio output |

The name and the description of a port or a parameter are texts, not blank. Names are unique within the inlets, within the outlets and within the parameters, and no capability is listed twice. A parameter's minimum and maximum are finite, the minimum at most the maximum, and neither is below 2^-126 in magnitude but zero, since a value there is zero (decision 5) and none would reach such a bound; its default, made zero if it is below 2^-126 in magnitude, is within them. For binary32 values this is the rule of a built-in box (`ParamSpec::new` in `audio-core`).

The whole description is at most `METADATA_BYTES_MAX` bytes (`audio-core`, the largest size of a patch document), the one bound on it: nothing inside it has a limit of its own. It bounds the memory a hostile module makes a host use while it loads, off the audio thread, and is provisional. A length or a count announced beyond the bytes left is refused, and reserves nothing: texts are read in place and lists grow with the items read.

The bytes are refused, naming the field, when there are too many, when they end before it, when a number is none of those above, when a text is not UTF-8 or breaks the rules above, when a name or a capability repeats, when a parameter's range or default is refused, and when bytes follow the last field.
