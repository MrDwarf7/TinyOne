# ABI Versioning and Stability

**ABI version 1 is frozen for the entire v1 lifecycle.** Consumers must check
`tinyone_abi_version()` against `TINYONE_ABI_VERSION` before using the API.
TinyLang v2 is a major version jump with an expected redesign of the language
boundary and does not promise backward compatibility with v1.

TinyLang will not keep every old implementation or historical language version
available forever. Pin the specific release or source revision you depend on.
Send comments, concerns, and questions to the [TinyLang community
forum](https://tl.404connernotfound.dev).

## What Constitutes a Breaking Change

The following changes break binary or source compatibility for callers:

**Function-level breaks:**
- Removing or renaming an entry point declared in `tinylang.h`
- Changing the type or order of any parameter
- Changing the return type of any entry point

**Response-level breaks:**
- Removing a key from a success `value` object
- Changing the type of an existing key in any response shape
- Removing one of the four envelope shapes (`ok/value`, `compile`,
  `runtime`, `panic`)
- Changing the meaning of `"kind"` values

**Bytecode-level breaks:**
- Reordering or removing any opcode in `Op` ordinal positions 1–29
- Reordering or removing any Phase-1 builtin in slots 0–34 of `BUILTINS`
- Changing the JSON artifact `"format"` or `"version"` field values

## What Is Not a Breaking Change

- Adding new keys to a success `value` object (callers should ignore
  unknown keys)
- Adding new entry points to `tinyone.h`
- Adding new opcode ordinals above the frozen Phase-1 range
- Adding new Phase-2 builtin slots above index 34
- Changing internal implementation details with no observable effect on
  inputs or outputs
- Changing error message text within the `"error"` field (do not parse
  error strings)

## Current Stability Status

| Area | Status | Notes |
| --- | --- | --- |
| Function signatures in `tinyone.h` | STABLE | Frozen at ABI version 1 |
| Response envelope shape (4 kinds) | STABLE | Frozen now |
| `value` object keys per endpoint | STABLE | Frozen by the committed JSON schema |
| `memory` array encoding | STABLE | Frozen by the committed JSON schema and contract tests |
| Phase-1 opcode ordinals (1–29) | STABLE | Frozen; artifact round-trips depend on them |
| Phase-2 opcode ordinals (30+) | INTERNAL | May change with a future artifact version; not exposed as C declarations |
| Phase-1 builtin slots (0–34) | STABLE | Frozen |
| Phase-2 builtin slots (35+) | INTERNAL | Order may change with a future language/artifact version |
| Artifact `format`/`version` fields | STABLE | `"tinyone-bytecode"` / `1` |

## ABI Version 1 Stability Declaration

The following are stable and will not change without a major ABI version bump:

1. All function signatures in `tinyone.h`
2. All four response envelope shapes
3. All `value` object keys for every entry point
4. The `memory` array encoding
5. Phase-1 opcode ordinals and Phase-1 builtin slot order used by artifact version 1

The exact nullability, ownership, threading, panic, and error rules are part
of the contract in [`contract.md`](contract.md) and the generated header.

## Compatibility Guidance

At startup, compare the library result from `tinyone_abi_version()` with the
header constant `TINYONE_ABI_VERSION`. Refuse to load the library when the
values differ; do not infer compatibility from the package version.

For JSON responses, branch on the documented `ok` and `kind` fields, validate
the fields your application requires, and ignore unknown success fields. Do
not parse human-readable `error` text. A `compile`, `runtime`, or `panic`
response is an application-visible failure, not an ABI mismatch.

For bytecode artifacts, accept only `format: "tinyone-bytecode"` and
`version: 1` unless the consumer explicitly supports another format version.
ABI version 1 does not promise compatibility for internal Rust layouts, heap
addresses, JIT listing text, or the Rust API.

## Decay Policy

After v1 is declared, deprecated features will be marked in `tinyone.h`
with a `// DEPRECATED(vX.Y): reason` comment and kept for at least one
minor version cycle before removal. Removals require a major version
bump.
