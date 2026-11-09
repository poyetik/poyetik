# Patch format, version 1

A patch is a UTF-8 JSON document. Version 1 is deliberately minimal: boxes, their parameters, and the cables between them. Later versions add what the engine needs, with migrations from older versions; [`engine-design.md`](engine-design.md) records what they will hold (subpatches, message cables, buses, polyphony, box state) and why.

## Example

A 440 Hz sine wave on both output channels:

```json
{
  "format": 1,
  "nodes": [
    { "id": 1, "box": "osc~", "params": { "frequency": 440 } },
    { "id": 2, "box": "dac~" }
  ],
  "connections": [
    { "from": [1, 0], "to": [2, 0] },
    { "from": [1, 0], "to": [2, 1] }
  ]
}
```

## Fields

| Field | Type | Meaning |
|---|---|---|
| `format` | integer | Format version. Must be `1`. |
| `nodes` | array | The boxes of the patch, in any order. |
| `nodes[].id` | integer, 0 to 4294967295 | Identifier, unique in the patch. Also orders execution among boxes that are ready at the same time (smallest first). |
| `nodes[].box` | string | Box type, as named in the engine registry. |
| `nodes[].params` | object, optional | Parameter values (numbers) by name, each name at most once. Unset parameters use their default. |
| `connections` | array | Cables, in any order, each at most once. |
| `connections[].from` | `[box id, outlet index]`, index 0 to 65535 | Source outlet (indices start at 0). |
| `connections[].to` | `[box id, inlet index]`, index 0 to 65535 | Destination inlet (indices start at 0). |

## Rules

- **Unknown fields are rejected**, at every level: a typo never goes unnoticed.
- **Repetitions are rejected**: two boxes with the same id, the same cable twice, or the same parameter name twice in one box.
- **Size limits**: a document of at most 16 MiB (16,777,216 bytes), at most 65,536 boxes and 262,144 cables. Larger patches are rejected, never truncated.
- Box types, parameter names and ranges, and port indices are checked against the engine registry when the patch is loaded.
- A single cable passes its samples unchanged, bit for bit. Several cables into one inlet are summed in the reduction order ([`engine-design.md`](engine-design.md), decision 28) of a fixed order of their terms that does not depend on the order of the document: by source box id, then source outlet index (floating-point addition is not associative, so the order is part of the result). One outlet may feed several inlets.
- A cycle without delay is rejected. Cycles through an explicit delay box are planned ([`engine-design.md`](engine-design.md), decision 3).
- **Parameter values are IEEE 754 binary32** (single precision): a number in the document is rounded to the nearest binary32 value (ties to even), identically on every target. `440.1` is played as 440.100006103515625 Hz; tools that compute what a patch sounds like use that value too. Version 2 reads parameters as binary64 ([`engine-design.md`](engine-design.md), decision 10); version 1 documents keep their binary32 values.
- **A subnormal parameter is zero** (`engine-design.md`, decisions 5 and 11): a binary32 value that is nonzero and below the smallest normal, 2^-126, in magnitude, becomes a zero of its sign before its range is checked, and the box is given that zero; `1e-40` plays as 0 Hz, `-1e-40` is accepted where a range starts at 0. The test is made on the binary32 value, exact in binary64: a number below 2^-126 that rounds to 2^-126 is that normal value. Version 2's binary64 parameters will be compared with 2^-126 in binary64, before any rounding to binary32.
