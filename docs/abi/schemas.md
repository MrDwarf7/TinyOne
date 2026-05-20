# ABI Response Schemas

This document specifies the exact JSON schema of the `"value"` object
returned on success by each entry point. Error response shapes are
documented in [`contract.md`](contract.md).

Fields marked **UNSTABLE** may gain or lose keys before v1. Fields
marked **STABLE** are frozen. See [`versioning.md`](versioning.md).

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
| `memory` | array | UNSTABLE | Top-level stack frame slots at program exit; encoding may change before v1 |
| `memory[*].type` | string | UNSTABLE | `"int"`, `"heap"`, or `"pointer"` |
| `memory[*].value` | integer | UNSTABLE | Present when `type` is `"int"` |
| `memory[*].address` | integer | UNSTABLE | Heap slot index; present when `type` is `"heap"` or `"pointer"` |
| `memory[*].generation` | integer | UNSTABLE | Generation counter at allocation time |
| `memory[*].kind` | string | UNSTABLE | Pointer kind: `"object"`, `"array"`, `"buffer"`, `"struct"`, `"cell"`, or `"null"` |
| `memory[*].index` | integer | UNSTABLE | Element/byte offset; present for array and buffer pointers |
| `memory[*].field` | string\|null | UNSTABLE | Field name; present for struct field pointers |
| `memory[*].cast` | string\|null | UNSTABLE | Cast type tag set by `cast_ptr`; `null` if not set |
| `heap_before_shutdown` | object | UNSTABLE | Heap stats immediately before runtime cleanup |
| `heap_after_shutdown` | object | UNSTABLE | Heap stats immediately after runtime cleanup |
| `heap_*.live_objects` | integer | UNSTABLE | Live heap object count |
| `heap_*.live_bytes` | integer | UNSTABLE | Live heap payload bytes |
| `heap_*.peak_objects` | integer | UNSTABLE | Peak live object count during the run |
| `heap_*.peak_bytes` | integer | UNSTABLE | Peak live bytes during the run |
| `heap_*.total_allocations` | integer | UNSTABLE | Total allocations over the run |
| `heap_*.total_frees` | integer | UNSTABLE | Total explicit frees (`unsafe free`) over the run |
| `heap_*.shutdown_frees` | integer | UNSTABLE | Objects freed during runtime shutdown drain |

---

## `tinyone_jit_listing_json`

```json
{
  "listing": "; tinyone adaptive-jit a1b2c3d4e5f60718\n; chunks=1 ops=3\n.chunk 0 main slots=0\n  0000 push.i 42\n  0001 print\n  0002 halt\n"
}
```

| Field | Type | Status | Notes |
| --- | --- | --- | --- |
| `listing` | string | UNSTABLE | Human-readable JIT assembly text; format may change |

---
