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
