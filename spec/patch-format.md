# Patch format, version 1

A patch is a UTF-8 JSON document. Version 1 is deliberately minimal: boxes, their parameters, and the cables between them. Later versions add what the engine needs, with migrations from older versions.

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
| `connections[].from` | `[node id, outlet index]`, index 0 to 65535 | Source outlet (indices start at 0). |
| `connections[].to` | `[node id, inlet index]`, index 0 to 65535 | Destination inlet (indices start at 0). |

## Rules

- **Unknown fields are rejected**, at every level: a typo never goes unnoticed.
- **Repetitions are rejected**: two nodes with the same id, the same cable twice, or the same parameter name twice in one node.
- **Size limits**: a document of at most 16 MiB (16,777,216 bytes), at most 65,536 nodes and 262,144 cables. Larger patches are rejected, never truncated.
- Box types, parameter names and ranges, and port indices are checked against the engine registry when the patch is loaded.
- A single cable passes its samples unchanged, bit for bit. Several cables into one inlet are summed, starting from the first, in a fixed order that does not depend on the order of the document: by source node id, then source outlet index (floating-point addition is not associative, so the order is part of the result). One outlet may feed several inlets.
- A cycle without delay is rejected.
- **Parameter values are IEEE 754 binary32** (single precision): a number in the document is rounded to the nearest binary32 value (ties to even), identically on every target. `440.1` is played as 440.100006103515625 Hz; tools that compute what a patch sounds like use that value too.
