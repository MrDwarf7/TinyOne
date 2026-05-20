# ABI Versioning and Stability

**Current ABI status: UNSTABLE.** Do not pin to a specific ABI version
until v1 is tagged and stability is declared. See the
[v1 roadmap](../v1-roadmap.md) for the work required before that
declaration.

## What Constitutes a Breaking Change

The following changes break binary or source compatibility for callers:

**Function-level breaks:**
- Removing or renaming an entry point declared in `tinyone.h`
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
- Adding new Phase-2 builtin slots above index 34
- Changing internal implementation details with no observable effect on
  inputs or outputs
- Changing error message text within the `"error"` field (do not parse
  error strings)

## Current Stability Status

| Area | Status | Notes |
| --- | --- | --- |
| Function signatures in `tinyone.h` | UNSTABLE | May change before v1 |
| Response envelope shape (4 kinds) | STABLE | Frozen now |
| `value` object keys per endpoint | UNSTABLE | Audit pending (roadmap item 1) |
| `memory` array encoding | UNSTABLE | Encoding not yet frozen |
| Phase-1 opcode ordinals (1–29) | STABLE | Frozen; artifact round-trips depend on them |
| Phase-1 builtin slots (0–34) | STABLE | Frozen |
| Phase-2 builtin slots (35+) | UNSTABLE | Order may change before v1 |
| Artifact `format`/`version` fields | STABLE | `"tinyone-bytecode"` / `1` |

## v1 Stability Declaration

When v1 is tagged, the following will be declared stable and will not
change without a major version bump:

1. All function signatures in `tinyone.h`
2. All four response envelope shapes
3. All `value` object keys for every entry point
4. The `memory` array encoding
5. Phase-1 opcode ordinals and Phase-2 builtin slot order

Before v1 can be declared, the [v1 roadmap](../v1-roadmap.md) items 1–5
(ABI blocking) must be resolved. Key items:

- **Item 1:** JSON response schema audit and contract tests committed
- **Item 2:** `Program` field visibility scoped to `pub(crate)`
- **Item 3:** `VerifiedProgram` adopted on all execution paths
- **Item 4:** `tinyone_free_string` wrapped in `catch_unwind`
- **Item 5:** Void `extern "C"` entry point policy decided and documented

## Decay Policy

After v1 is declared, deprecated features will be marked in `tinyone.h`
with a `// DEPRECATED(vX.Y): reason` comment and kept for at least one
minor version cycle before removal. Removals require a major version
bump.
