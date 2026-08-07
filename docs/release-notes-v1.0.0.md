# TinyOne v1.0.0

TinyOne v1.0.0 is the first stable v1 release of the TinyLang implementation.
It freezes the public C ABI at version 1 and establishes the compatibility
rules in [`docs/abi/versioning.md`](abi/versioning.md).

## Highlights

- Stable C ABI entry points and generated `tinylang.h` header.
- Version handshake through `tinyone_abi_version()` and
  `TINYONE_ABI_VERSION`.
- Frozen JSON response envelopes and success schemas.
- Frozen version-1 bytecode artifact format, Phase-1 opcode ordinals, and
  Phase-1 builtin slots.
- Checked integer arithmetic: integer overflow is a runtime error and never
  silently wraps.
- Explicit string indexing: `s[i]` uses Unicode-scalar positions;
  `str_byte_at` provides UTF-8 byte access and `str_char_at` provides explicit
  Unicode-scalar access. Index bounds are checked at runtime.

## Compatibility

Consumers should perform the ABI version check before calling any entry point,
handle the documented JSON envelope, ignore unknown success fields, and avoid
parsing human-readable error text. See the [compatibility guidance](abi/versioning.md#compatibility-guidance)
for artifact and Rust API boundaries.

## Known v1 boundaries

The Rust API and internal heap/layout details are not part of the stable C ABI.
JIT listing text is opaque and may change. Language features deferred to v2 are
tracked in [`docs/v2-roadmap.md`](v2-roadmap.md).
