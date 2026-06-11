# TinyLang FFI v2 — Unified Typed C ABI

- **Status:** Draft / Planned
- **ABI status:** UNSTABLE. This document defines the *intended* shape of the
  v1-stable FFI surface. Nothing here is stable until v1 is tagged.
- **Naming:** the public header is `tinylang.h` (branding), but the exported
  symbols remain `tinyone_*` and the library remains `libtinyone`. Name and
  symbols are intentionally decoupled — no ABI-wide symbol rename is implied.
- **Supersedes (eventually):** the JSON-over-C-string surface documented in
  `docs/ffi/c-integration.md`, which is retained as a convenience/debug façade
  (see §5).

---

## 1. Summary

Today TinyOne's FFI is a single, *inbound* surface: a C host calls
`tinyone_*_json` functions that take and return JSON-over-C-strings
(`TinyOne/src/ffi.rs`). That design is correct for coarse, one-shot calls
(compile, run) and has a tiny unsafe surface — but it cannot express the two
things this proposal targets:

1. **Outbound library calls** — TinyOne code calling *into* a native C library
   (e.g. OpenGL) without a maintainer hand-writing builtin glue per function.
2. **IPC** — two processes exchanging TinyOne values over a transport.

FFI v2 introduces **one canonical, `repr(C)` typed value representation
(`TinyValue`) and one generation-checked handle model**, and uses it across all
three communication directions. JSON is kept only as a debug/convenience
façade. cbindgen generates `tinylang.h` and `ralloc.h` from the Rust source so
the surface stays in sync for C, C++, Rust, and Zig consumers.

The unifying principle: **the C ABI is the only universal transport; we choose
*one* encoding on top of it and reuse it everywhere.** In-process boundaries
pass `TinyValue` by value/pointer (zero serialization). IPC encodes it to a
length-prefixed binary frame (serialization only because a process boundary
forces it). Outbound calls marshal `TinyValue` to and from the native C ABI.

---

## 2. Motivation

### 2.1 What JSON-over-C does well

- Universally consumable — every language has a JSON parser.
- Self-describing and human-debuggable.
- Minimal unsafe surface: the only raw pointer crossing the boundary is one
  owned `char *` with a documented ownership contract (`tinyone_free_string`).
- Version-tolerant (unknown fields can be ignored).

These properties are why the inbound control API (`lex`, `compile`, `run`,
`jit_listing`) stays on JSON. A full compile or program run dwarfs any
serialization cost, so JSON is the right trade there.

### 2.2 Where it breaks down

- **Stringly typed, hot-path cost.** Every consumer links a JSON parser and pays
  serialize/parse on each call. The `value_to_json` 20-variant dump
  (`ffi.rs:273`) is parsed by every consumer that inspects results.
- **JSON number semantics.** Float precision, NaN, and infinity have no faithful
  JSON representation; today floats are emitted as raw bits to work around this.
- **No compile-time schema.** Consumers match on string keys; drift is silent.
- **Inbound only.** There is no mechanism for TinyOne code to call *out* to a C
  library. Using OpenGL today would mean writing a builtin in `builtins.rs` for
  every GL entry point — weeks of glue, per library.
- **No IPC story.** Nothing defines how a TinyOne value crosses a process
  boundary.

### 2.3 Goals

1. A typed, zero-serialization inbound path for embedders, *alongside* JSON.
2. Let TinyOne call arbitrary C libraries with little or no hand-written glue.
3. Enable IPC of TinyOne values — same-machine **and networked**.
4. Keep **one** value representation across all three directions.
5. Preserve the soft-safety memory model: TinyOne code never holds raw native
   pointers; validity is enforced via generation-checked handles.
6. Serve C / C++ / Rust / Zig from one cbindgen-generated header.

### 2.4 Non-goals (v2)

- A stable ABI commitment before v1.
- Marshalling every TinyOne type outbound (only a C-ABI-compatible subset is
  permitted in `extern` signatures — see §6).
- A general C++-name-mangled or Rust-native ABI (the boundary is always C ABI).

---

## 3. The Three Directions

| Direction | Who calls whom | Example | Frequency |
|-----------|----------------|---------|-----------|
| **Inbound** | C host → TinyOne VM | embed the VM, run a program, read results | coarse |
| **Outbound** | TinyOne → native C library | call OpenGL, libc, a user `.so` | hot |
| **IPC** | TinyOne process ↔ another process | two VMs, tooling, a host service | medium |

All three share the **`TinyValue`** value model (§4.1) and the **handle** model
(§4.2). They differ only in how that representation is *moved*: by
value/pointer in-process, marshalled to the native ABI outbound, or framed as
bytes for IPC.

---

## 4. Core: the `TinyValue` ABI and handle model

This is the foundation every sub-design builds on.

### 4.1 `TinyValue` — a `repr(C)` tagged value

The binary equivalent of today's `value_to_json` output. A tag enum plus a
union payload. Illustrative shape; exact field layout is finalized during
implementation:

```c
typedef enum TinyValueKind {
  TINY_KIND_I8 = 0, TINY_KIND_I16, TINY_KIND_I32, TINY_KIND_I64,
  TINY_KIND_U8, TINY_KIND_U16, TINY_KIND_U32, TINY_KIND_U64,
  TINY_KIND_BF16, TINY_KIND_FP16, TINY_KIND_FP32, TINY_KIND_FP64,
  TINY_KIND_BOOL, TINY_KIND_UNIT, TINY_KIND_NULL,
  TINY_KIND_FUNCTION, TINY_KIND_REFERENCE, TINY_KIND_PHANTOM,
  TINY_KIND_ZST, TINY_KIND_UNSAFE,
  TINY_KIND_HEAP,     /* opaque handle into the GAT */
  TINY_KIND_POINTER,  /* provenance-tracked pointer descriptor */
  /* append-only: never renumber existing tags */
} TinyValueKind;

typedef struct TinyValue {
  TinyValueKind kind;
  union {
    int64_t  i;      /* sized ints, sign per kind */
    uint64_t u;      /* sized uints + float bit patterns */
    uint8_t  b;      /* bool */
    TinyHandle handle;        /* HEAP / FUNCTION */
    TinyPointerDesc pointer;  /* POINTER: addr, kind, index, field, gen, cast */
  } payload;
} TinyValue;
```

Mirrors the variants in `RuntimeValue` (`ffi.rs:273-308`) and the 43 `TypeKind`
members from the Phase 1 type work. Floats carry their bit pattern (faithful
NaN/inf/-0, matching the IEEE-754 rules in `typing_system.md`).

Aggregates (`String`, `Buffer<T>`, `Vec<T>`, `Array<T,N>`) cross the boundary as
`{ ptr, len, cap, elem_kind }` descriptors, not inline. Text is **length +
pointer UTF-8**, not NUL-only (TinyOne strings may contain interior NULs and are
always validated UTF-8 per `typing_system.md`).

### 4.2 `TinyHandle` — opaque, generation-checked

```c
typedef struct TinyHandle {
  uint64_t id;          /* GAT allocation id / foreign-table id */
  uint64_t generation;  /* bumped on free; stale handle => recoverable fault */
} TinyHandle;
```

Matches the `Handle { addr, generation }` recommendation in
`ownership_semantics_and_memory_safety.md`. The generation counter is what makes
the expanded ABI *safe*: a freed-then-reused id is detected, so use-after-free
across the boundary becomes a recoverable VM fault instead of process
corruption. Heap objects resolve against the GAT; foreign resources resolve
against the Foreign Handle Table (§6.3).

### 4.3 Error model

Replace the JSON `{"ok":false,...}` envelope with a typed result for the
handle-based path:

```c
typedef enum TinyStatus {
  TINY_OK = 0,
  TINY_ERR_COMPILE, TINY_ERR_RUNTIME, TINY_ERR_NULL_ARG,
  TINY_ERR_BAD_HANDLE, TINY_ERR_LIMIT, TINY_ERR_FOREIGN, TINY_ERR_PANIC,
} TinyStatus;

/* thread-local detail string for the last error on this thread */
const char *tinyone_last_error(void);
```

Functions return `TinyStatus` and write results through out-params. Every
`extern "C"` entry point keeps the `catch_unwind` guard already used by
`respond()` (`ffi.rs:143`); a panic becomes `TINY_ERR_PANIC`, never an unwind
into the caller.

### 4.4 Why one representation serves all three

- **Inbound:** results returned as `TinyValue` arrays + `repr(C)` stat structs —
  no JSON parse.
- **Outbound:** the marshaller converts `TinyValue` to/from native C types per a
  declared signature (§6).
- **IPC:** a `TinyValue` tree is encoded to a length-prefixed frame and decoded
  on the far side (§7).

---

## 5. Sub-design A — Inbound typed/handle ABI (hardening over JSON)

### 5.1 Handles and entry points

Opaque handles, freed explicitly:

```c
typedef struct TinyEngine  TinyEngine;   /* owns JitCache, config */
typedef struct TinyProgram TinyProgram;  /* an Arc<Program> */

TinyEngine *tinyone_engine_new(void);
void        tinyone_engine_free(TinyEngine *);

TinyStatus tinyone_compile_source(TinyEngine *, const char *src,
                                  TinyProgram **out);
TinyStatus tinyone_program_run(TinyProgram *, const char *mode,
                               const TinyValue *inputs, size_t n_inputs,
                               TinyRunReport *out);   /* repr(C) report */
TinyStatus tinyone_program_fingerprint(TinyProgram *, uint64_t *out);
void       tinyone_program_free(TinyProgram *);
```

`TinyRunReport` is the `repr(C)` form of the current run payload: a `TinyValue`
array (the top-of-stack memory snapshot) plus `TinyHeapStats`
(`heap_before/after_shutdown` — already a Rust struct in `ffi.rs:310`).

### 5.2 What "hardening" means here

- **No JSON on the hot path.** Embedders that run many programs or inspect many
  values skip serialize/parse entirely.
- **Generation-checked handles.** Running a freed `TinyProgram` returns
  `TINY_ERR_BAD_HANDLE`, not UB.
- **Length-bounded inputs.** Generalize the existing 8 MiB `MAX_ARTIFACT_BYTES`
  scan (`ffi.rs:201`) to all length-taking entry points.
- **Explicit ownership.** One `*_free` per handle type; documented in the
  generated header via cbindgen doc-comments.

### 5.3 JSON façade retained

The existing `tinyone_*_json` functions stay, unchanged, as a convenience/debug
surface. They become thin wrappers that build a `TinyValue`/handle internally
and serialize. Same `catch_unwind` discipline. `docs/ffi/c-integration.md` is
updated to describe both surfaces and when to use which.

### 5.4 Threading

The current API is stateless per call and safe to call concurrently with
distinct arguments. Handles introduce shared state, so the header must document
which handles are `Send`/shareable. `JitCache` is **not** thread-safe (existing
limitation); `TinyEngine` therefore is not shareable across threads without
external synchronization. `TinyProgram` (an `Arc<Program>`, immutable) is
shareable for reads.

---

## 6. Sub-design B — Outbound library FFI (the OpenGL goal)

The largest new subsystem, and the reason for "users shouldn't spend weeks
building handlers over OpenGL."

### 6.1 Layer 1 — dynamic FFI core (libffi)

TinyOne declares foreign signatures in an `extern` block; the VM resolves
symbols with `dlopen`/`dlsym` and calls them through **libffi**, marshalling
`TinyValue`s to/from the native C ABI at runtime. **No per-function Rust glue.**

```tinyone
extern "C" from "libGL.so.1" {
  unsafe fn glClear(mask: u32)
  unsafe fn glGenBuffers(n: i32, buffers: &mut Buffer<u32>)
  unsafe fn glBindBuffer(target: u32, buffer: u32)
  unsafe fn glBufferData(target: u32, size: i64, data: &Buffer<u8>, usage: u32)
  unsafe fn glGetString(name: u32) -> ForeignPtr
}
```

The type system already reserves "calling convention / ABI metadata" on function
types (`typing_system.md`, Function ABI). `extern "C"` selects the platform C
calling convention; other conventions (`stdcall`, etc.) are declarable.

### 6.2 Layer 2 — binding generation

A tool ingests a C header via libclang and emits a TinyOne `extern` module (plus
optional safe wrappers). Point it at `GL/gl.h` and get typed bindings without
writing them. This is the ergonomic layer on top of Layer 1; Layer 1 works
without it.

### 6.3 Safety model (critical)

Native code is outside the VM's safety boundary, so:

- **Foreign calls are `unsafe` by default** — matching the unsafe-call rule in
  `typing_system.md`. They appear only inside `unsafe` expressions/blocks.
- **Foreign pointers never enter TinyOne as raw pointers.** A returned `void*`,
  `GLuint`-as-handle, or `char*` is wrapped as a `ForeignPtr` — an opaque,
  generation-checked entry in a **Foreign Handle Table** parallel to the GAT.
  Stale/freed foreign handles fault recoverably.
- **Buffers handed to C are VM-owned `Buffer<T>`** (Ralloc-backed, aligned),
  *pinned* for the duration of the call; the VM enforces bounds and lifetime.
- **`catch_unwind` wraps the trampoline** so a marshalling fault never unwinds
  into or out of C.
- **Honest limit:** the VM guarantees *TinyOne-side* handle validity. Once a raw
  pointer is in C's hands, what C does with it (e.g. retaining it past the call)
  is the caller's responsibility. This preserves "TinyOne code never manipulates
  native pointers" (`ownership_semantics_and_memory_safety.md`) without
  over-promising memory safety for arbitrary C behavior.

### 6.4 Type marshalling

| TinyOne | C | Notes |
|---------|---|-------|
| `i8..i64` / `u8..u64` | `int8_t..int64_t` / `uint8_t..uint64_t` | exact width |
| `fp32` / `fp64` | `float` / `double` | bf16/fp16 require explicit widening |
| `bool` | `_Bool` / `int` | per signature |
| `String` | `const char *` | UTF-8, NUL-appended copy for the call |
| `Buffer<T>` | `T* + size` | pinned, bounds-checked, aligned |
| `ForeignPtr` | `void *` / opaque | generationed foreign handle |
| `Vec`/`Map`/sum/closure | — | **rejected** in `extern` signatures (§2.4) |

Returned-pointer ownership is declared per function (the C library frees it, or
TinyOne frees it through a declared finalizer — see open questions).

### 6.5 Walkthrough (illustrative)

```tinyone
let gl = extern_load("libGL.so.1")        // ForeignPtr to the lib handle
let mut ids = Buffer<u32>.with_len(1)
unsafe { glGenBuffers(1, &mut ids) }       // C writes into pinned VM buffer
let verts = Buffer<f32>.from([ -1.0, -1.0, 1.0, -1.0, 0.0, 1.0 ])
unsafe {
  glBindBuffer(GL_ARRAY_BUFFER, ids[0])
  glBufferData(GL_ARRAY_BUFFER, verts.byte_len(), &verts, GL_STATIC_DRAW)
}
```

No GL-specific Rust code exists in TinyOne; the calls route through libffi, and
`ids`/`verts` stay VM-owned and bounds-checked.

---

## 7. Sub-design C — IPC transport

### 7.1 Wire format

The same `TinyValue` tree, encoded as a **length-prefixed binary frame** with a
small versioned header (magic + ABI version + length). Little-endian, matching
the byte-buffer read rule in `typing_system.md`. The encoding is
transport-agnostic — it rides any byte stream (Unix socket, pipe, or TCP/TLS).
Because a frame may arrive from an **untrusted remote peer**, decoding validates
every length, tag, and the version against bounds *before* allocating, enforces
a maximum frame size, and decodes into an isolated arena (§7.4).

### 7.2 Transports

- **Same-machine:**
  - **Unix domain sockets** for control and small messages.
  - **Shared-memory ring buffer** (Ralloc-backed region) for large payloads,
    avoiding copies.
  - Pipes for simple parent/child cases.
- **Networked:**
  - **TCP** for the stream, with **TLS** for encryption and peer authentication.
  - The same framed `TinyValue` encoding rides the encrypted stream unchanged.

(QUIC and the precise peer-authentication model are open questions — §12.)

### 7.3 Handles are process-local (hard rule)

An `id`/`generation` is meaningful only inside the originating VM's GAT. **A
handle must never be sent to another process — local or remote — and
dereferenced there.** Across any process boundary, values are sent **by copy**;
a shared resource is represented by a capability/token the receiver re-resolves
in its own table. Over a network this is not merely a correctness rule but a
security boundary: a remote peer must never be able to name an address in this
VM's heap.

### 7.4 Untrusted input and Ralloc's multi-arena isolation

This is why Ralloc's multi-arena allocator is a system requirement, not an
optimization. Networked IPC ingests bytes from peers the VM does not trust, so
decoded peer data is quarantined in **per-connection Ralloc arenas**, separate
from the VM's trusted internal allocations. That arena boundary buys four safety
properties:

- **Budgeting / DoS containment.** A peer's decoded data is capped at its
  arena's capacity, so a flood of large or numerous frames exhausts only that
  peer's arena — not the global VM heap, and not other connections.
- **Blast-radius containment.** A decode bug that corrupts memory is confined to
  the offending peer's arena, away from VM-critical structures and other peers'
  data.
- **Deterministic teardown.** When a connection closes or a peer is dropped for
  misbehavior, the whole arena is released in one bulk operation — every
  allocation tied to that peer is reclaimed at once (the region-based
  bulk-release model in `phase_2_allocator.md`, Part 5).
- **Forgery resistance.** A crafted frame cannot manufacture a usable handle:
  `TinyValue` handles are validated against the GAT by generation, `VmAllocation`
  is move-only (`!Clone !Copy`, type-level double-free prevention), and Rust owns
  native-memory integrity — so the worst case is a recoverable VM fault, never
  host-process corruption.

**Dependency:** per-connection arenas rely on real Ralloc native backing and
arena growth (`phase_2_allocator.md` [P3B-1], [P3B-2]); the current
4 × 64 KiB static ceiling must be lifted first.

### 7.5 Later: IDL option

If third-party, cross-language IPC consumers proliferate and need formal schema
evolution, the hand-rolled framing can be replaced by FlatBuffers or Cap'n
Proto for the *wire* only. Start hand-rolled; revisit if that need is real.

---

## 8. cbindgen integration (`tinylang.h` + `ralloc.h`)

### 8.1 Generation

- Add `cbindgen.toml` to the TinyOne crate and to the Ralloc crate.
- Mark every FFI-exposed type `#[repr(C)]` (`TinyValue`, `TinyHandle`,
  `TinyHeapStats`, `TinyRunReport`, the enums). Functions are already
  `#[no_mangle] extern "C"`.
- Output: TinyOne → **`tinylang.h`** (containing the `tinyone_*` symbols + the
  `repr(C)` types); Ralloc → **`ralloc.h`** (replacing the current hand-written
  header at `Ralloc/include/ralloc.h`).

### 8.2 Delivery: committed headers + CI drift-check

Generate via an `xtask`/`make headers` step; **commit** the generated headers
and add a CI job that regenerates and fails on any diff. Rationale: consumers
don't need cbindgen installed, and every ABI change shows up as a reviewable
header diff. (Preferred over pure `build.rs` autogen, which hides ABI changes
and adds a hard build dependency.)

### 8.3 Header reconciliation

The existing hand-written `tinyone.h` at the repo root becomes a one-line shim:

```c
#include "tinylang.h"
```

so existing includers keep working. `docs/ffi/c-integration.md` is updated to
reference `tinylang.h`.

### 8.4 Ralloc is a required dependency — wire it to canonical (P3B-4)

Ralloc is a foundational requirement of the TinyLang system — it underpins the
IPC safety model (§7.4), not an optional backend. The blocker is *sourcing*. Per
`phase_2_allocator.md` [P3B-4], the in-repo `TinyOne/Ralloc/` is an **embedded
git repository** (it has its own `.git`) that is **stale**: its `lib.rs`
predates the VM API layer and exports no `VmAllocation`/`VmAllocator`/`vm_api`.
The canonical checkout at `/Desktop/Ralloc/` is the one that contains
`vm_api.rs`. Generating `ralloc.h` from the embedded copy would ship a header
for an allocator that lacks the VM API the rest of this design depends on.
Resolve first — remove the embedded repo and add a submodule (or a
version-pinned published crate) pointing at canonical Ralloc — then wire
cbindgen against that.

### 8.5 Consumer story

- **C:** include `tinylang.h`.
- **C++:** include it (cbindgen emits `extern "C"` guards); wrap handles in RAII.
- **Zig:** `@cImport(@cInclude("tinylang.h"))` — direct.
- **Rust:** re-bind via `bindgen`, or hand-write against the header.

---

## 9. Safety model summary

The typed/handle ABI and especially outbound FFI **expand** the unsafe surface
relative to JSON's single owned `char *`. The hardening keeps it bounded:

- **Generation-checked handles** (GAT for heap, Foreign Handle Table for native)
  turn use-after-free / stale access into recoverable faults.
- **`catch_unwind` at every `extern "C"` boundary**; panics become
  `TINY_ERR_PANIC`, never an unwind into C.
- **Inbound:** null args return `TINY_ERR_NULL_ARG` (no crash); length caps on
  all inputs; explicit per-handle free.
- **Outbound:** `unsafe` by default; VM-owned, pinned, bounds-checked buffers;
  honest scoping of what the VM can and cannot guarantee once a pointer is in C.
- **IPC:** frames are untrusted (especially from remote peers) — strict
  length/tag/version validation and max-frame-size caps, decoded into
  **per-connection Ralloc arenas** so a hostile peer is budget-bounded and
  blast-radius-contained (§7.4); never decode into a raw pointer; handles never
  cross the process boundary.
- **Ralloc hard boundary:** native memory integrity stays in Rust; TinyOne never
  holds a raw address.

**Known limitation carried over:** `tinyone_free_string` calls
`CString::from_raw` without a `catch_unwind` guard (`c-integration.md`, Known
Limitations). The v2 free functions should either adopt the same documented
contract or add guards; decide during implementation.

---

## 10. Versioning

- The FFI ABI is UNSTABLE until v1; this surface is the **candidate v1-stable
  ABI**.
- `TinyValueKind` and all status/enum tags are **append-only** — never renumber
  or remove a tag.
- `repr(C)` structs evolve additively (no field reordering; version the struct
  or use sized/headered structs where growth is expected).
- Expose `uint32_t tinyone_abi_version(void)`; the IPC frame header carries the
  same version so peers can reject mismatches.
- Align with the rules in `docs/abi/versioning.md` and `docs/abi/contract.md`.

---

## 11. Decomposition and build order

Each piece gets its own spec → plan → implementation cycle. (1)–(4) depend on
(0).

0. **Core** — `TinyValue` + `TinyHandle` + error model; cbindgen wiring for
   `tinylang.h`/`ralloc.h`; resolve Ralloc M-3.
1. **Inbound typed/handle ABI** — engine/program handles, `TinyRunReport`, JSON
   façade rewrite.
2. **Outbound libffi core** — `extern` blocks, the libffi trampoline, the
   Foreign Handle Table, buffer pinning.
3. **Outbound binding generator** — libclang-based header importer.
4. **IPC transport** — framing + Unix sockets and the shared-memory ring
   (same-machine), then TCP/TLS (networked) with per-connection arenas. Depends
   on Ralloc native backing + arena growth ([P3B-1], [P3B-2]).

---

## 12. Open questions and risks

1. **libffi dependency.** Use the `libffi`/`libffi-sys` crate (C dependency,
   broad platform support) vs. a narrower hand-rolled calling-convention shim?
   Variadic functions (`printf`) and struct-by-value arguments are the hard
   cases libffi handles but that a hand-rolled shim would not.
2. **Networked IPC specifics.** Transport beyond TCP/TLS (QUIC?); the peer
   authentication model (mutual TLS, tokens, a capability handshake?);
   per-connection arena sizing and the backpressure/flow-control policy.
3. **Non-C-mappable types.** `Vec`/`Map`/sum types/closures are rejected in
   `extern` signatures. Confirm that's acceptable, or define an opt-in
   marshalling (e.g. expose a `Vec<u8>` as `ptr+len` only).
4. **Foreign destructors.** How does a `ForeignPtr` know how to free itself
   (e.g. `glDeleteBuffers`)? Likely a user-declared finalizer in the `extern`
   block. Needs design.
5. **C → TinyOne callbacks.** Passing a TinyOne function as a C function pointer
   (GLFW-style callbacks) needs a trampoline and a pinned environment. Hard
   sub-problem; candidate to defer past v2 core.
6. **Handle thread model.** Precisely which handles are `Send`/shareable, given
   `JitCache` is not thread-safe.
7. **cbindgen + tagged unions.** cbindgen's union support may need manual
   annotations or a hand-written portion for `TinyValue`; verify early.
