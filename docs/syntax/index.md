---
title: Language Syntax Reference
---

This section documents the current TinyLang syntax. It is not a promise
that the syntax, grammar, keywords, or semantics will remain unchanged.
The v1 ABI is frozen for the v1 lifecycle, but syntax remains subject to
discovery and revision before TinyLang v2.

The [README](../../README.rst) includes an inline language overview with
examples; these files are the current reference, not a permanent
contract.

Send syntax comments, concerns, and questions to the [TinyLang community
forum](https://tl.404connernotfound.dev).

# Documents in This Area

- [types.md](types.md) \-- All value types: int, string, array, struct,
  buffer, cell, pointer, and null; creation, mutation, runtime errors,
  and ownership.
- [statements.md](statements.md) \-- Every statement form: let,
  assignment, print, set, if/else, while, break, continue, return,
  struct, fn, export, and import.
- [expressions.md](expressions.md) \-- Expression grammar, the full
  operator precedence table, arithmetic, comparisons, and the unsafe
  gate.
- [modules.md](modules.md) \-- Import/export system, path resolution,
  `tinyone.json` manifest, circular-import detection, and a worked
  example.
