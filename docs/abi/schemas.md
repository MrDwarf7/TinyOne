# ABI Response Schemas

This document specifies the exact JSON schema of the `"value"` object
returned on success by each entry point. Error response shapes are
documented in [`contract.md`](contract.md).

The machine-readable contract is committed at
[`tinyone-response-schema.json`](../../tinyone-response-schema.json). Consumers
may validate complete responses against that schema; the Rust ABI tests also
assert the exact response keys emitted by every current JSON entry point.

All fields and encodings in this document are frozen for ABI version 1.
Unknown future fields must be ignored by consumers.

---

## `tinyone_lex_source_json`

```json
{
  "tokens": 5
}
```

| Field | Type | Status | Notes |
| --- | --- | --- | --- |
| `tokens` | integer | STABLE | Count of tokens produced by the lexer |

---

## `tinyone_compile_source_json` / `tinyone_compile_file_json`

```json
{
  "artifact": {
    "format":     "tinyone-bytecode",
    "version":    1,
    "code":       [ {"op": "PUSH_INT", "arg": 42, "arg2": 0} ],
    "slot_count": 0,
    "names":      [],
    "functions":  [],
    "strings":    [],
    "structs":    [],
    "fields":     [],
    "modules":    []
  },
  "fingerprint": "a1b2c3d4e5f60718"
}
```

| Field | Type | Status | Notes |
| --- | --- | --- | --- |
| `artifact` | object | STABLE (structure) | Full bytecode artifact; see [bytecode.md](../bytecode.md) for field semantics |
| `artifact.format` | string | STABLE | Always `"tinyone-bytecode"` |
| `artifact.version` | integer | STABLE | Always `1` until a format break |
| `fingerprint` | string | STABLE | Blake2b512 truncated to 16 hex bytes; matches `Program::fingerprint()` |

---

## `tinyone_run_source_json` / `tinyone_run_file_json` / `tinyone_run_artifact_json`

```json
{
  "stdout": "42\n",
  "memory": [
    {"type": "int",     "value": 42},
    {"type": "heap",    "address": 0, "generation": 1},
    {"type": "pointer", "address": 0, "kind": "array", "index": 0,
                        "field": null, "generation": 1, "cast": null}
  ],
  "heap_before_shutdown": {
    "live_objects":      1,
    "live_bytes":        64,
    "peak_objects":      1,
    "peak_bytes":        64,
    "total_allocations": 1,
    "total_frees":       0,
    "shutdown_frees":    0
  },
  "heap_after_shutdown": {
    "live_objects":      0,
    "live_bytes":        0,
    "peak_objects":      1,
    "peak_bytes":        64,
    "total_allocations": 1,
    "total_frees":       1,
    "shutdown_frees":    1
  }
}
```

| Field | Type | Status | Notes |
| --- | --- | --- | --- |
| `stdout` | string | STABLE | All text written to stdout during execution, including newlines |
| `memory` | array | STABLE | Top-level stack frame slots at program exit; one object per slot |
| `memory[*].type` | string | STABLE | Discriminator for the exact runtime-value object shape in the machine-readable schema |
| `memory[*].value` | number or boolean | STABLE | Numeric JSON value for integer/float variants; boolean for `bool`; integers retain their signedness and width through `type` |
| `memory[*].address` | integer | STABLE | Address field for `heap`, `pointer`, and `reference` values |
| `memory[*].generation` | integer | STABLE | Generation counter at allocation time |
| `memory[*].kind` | string | STABLE | One of `null`, `object`, `array`, `buffer`, or `field` |
| `memory[*].index` | integer | STABLE | Signed element/byte offset for a pointer |
| `memory[*].field` | string | STABLE | Field name; empty string unless the pointer refers to a named field |
| `memory[*].cast` | string | STABLE | Cast type tag; empty string when no cast is present |
| `heap_before_shutdown` | object | STABLE | Heap stats immediately before runtime cleanup |
| `heap_after_shutdown` | object | STABLE | Heap stats immediately after runtime cleanup |
| `heap_*.live_objects` | integer | STABLE | Live heap object count |
| `heap_*.live_bytes` | integer | STABLE | Live heap payload bytes |
| `heap_*.peak_objects` | integer | STABLE | Peak live object count during the run |
| `heap_*.peak_bytes` | integer | STABLE | Peak live bytes during the run |
| `heap_*.total_allocations` | integer | STABLE | Total allocations over the run |
| `heap_*.total_frees` | integer | STABLE | Total explicit frees (`unsafe free`) over the run |
| `heap_*.shutdown_frees` | integer | STABLE | Objects freed during runtime shutdown drain |

---

## `tinyone_jit_listing_json`

```json
{
  "listing": "; tinyone adaptive-jit a1b2c3d4e5f60718\n; chunks=1 ops=3\n.chunk 0 main slots=0\n  0000 push.i 42\n  0001 print\n  0002 halt\n"
}
```

| Field | Type | Status | Notes |
| --- | --- | --- | --- |
| `listing` | string | STABLE | Human-readable JIT assembly text; consumers must treat its contents as opaque text |

---
