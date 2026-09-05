---
title: Memory Model
---

This is the current TinyOne memory and allocator design. It may change
during v1 as implementation work continues; the v1 ABI freeze does not
freeze the internal allocator or memory model. TinyLang v2 is expected
to make major changes. Send comments, concerns, and questions to the
[TinyLang community forum](https://tl.404connernotfound.dev).

The TinyOne runtime implementation manages two memory regions: the
**stack-frame memory** (fixed slots per function call) and the **heap**
(dynamic allocation for all aggregate values). This document covers how
each region works, how references are validated, and which resource
limits apply.

For the value types that use heap allocation, see
[syntax/types.md](syntax/types.md). For the opcodes that interact with
memory, see [bytecode.md](bytecode.md).

# Stack-Frame Memory

Each function call allocates a fixed number of slots determined at
compile time (`Function.slot_count`). Slots are stored in a flat
`Vec<Value>` (`TinyMemory`) shared across the entire call chain as
contiguous frame slices.

- Slots are **zero-initialized** at frame entry (value `Int(0)`).
- Slots are **not freed** between loop iterations -- a slot retains its
  last-written value until the function returns.
- Slot count is fixed; the compiler allocates new slots for every `let`
  declaration. Block exit does not reclaim slots; names are hidden after
  their scope but the slot remains allocated in the frame.
- At function return, the frame slice is discarded.
- Function chunks can read top-level slots through `LOAD_GLOBAL` when
  the variable was declared before the function. They cannot assign
  those slots directly; shared mutation should happen through heap
  objects stored in a top-level slot.
- A spawned thread receives a snapshot of the top-level frame taken when
  `thread_spawn` is called. Heap references in that snapshot still
  resolve through the shared runtime heap; coordinate shared mutation
  with mutexes or atomic values.
- Spawned threads inherit the runtime's system arguments and
  environment. Deterministic input queues are not shared between
  threads; pass input values as function arguments before spawning.

# Heap Architecture

`TinyHeap` is a **generational slab**:

- An `objects: Vec<Option<HeapObject>>` vector holds all heap objects.
  Each slot is either `None` (free) or `Some(HeapObject)`.
- A parallel `generations: Vec<u64>` vector holds the generation counter
  for each slot. New slots start at generation 1; reused slots increment
  immediately before allocation.
- A `free: Vec<usize>` list holds the indices of currently free slots.

**Allocation:** Claims the next free slot from `free` (or appends a new
slot if `free` is empty), increments the generation, and stores the
object.

**Deallocation (`unsafe free`):** Sets the slot to `None` and adds
the index to `free`. The generation increments if that slot is later
reused.

# `HeapRef` and Generation Validation

Every reference to a heap object is a
`HeapRef { address: usize, generation: u64 }`.

Before any access to a heap object, the runtime checks:

    stored_generation[address] == ref.generation

If they differ -- because the slot was freed and possibly reallocated
since `ref` was created -- the runtime returns
`TinyOneError::runtime("Stale heap reference ...")`.

This catches **use-after-free** and prevents a new allocation at the
same address from being mistaken for the old object.

# `RawPointer` and Validation

A `RawPointer { address, kind, index, field, generation, cast }` derives
from a `HeapRef` and adds:

- `kind` -- `"null"`, `"object"`, `"array"`, `"buffer"`, or `"field"`.
- `index` -- element or byte offset (for array and buffer pointers).
- `field` -- field name (for struct-field pointers).
- `generation` -- generation at pointer creation time.
- `cast` -- optional type annotation set by `cast_ptr`.

Before any pointer use, the runtime validates, in order:

1.  **Base generation** --
    `stored_generation[address] == pointer.generation`.
2.  **Kind** -- the live object at `address` matches `pointer.kind`.
3.  **Bounds** -- `index` is within the object's element or byte
    count.

A stale base object, kind mismatch, or out-of-bounds access each produce
a structured runtime error rather than undefined behavior.

# Ownership Rules

TinyLang does not use garbage collection or compile-time borrow checking
in the current TinyOne implementation. The Rust runtime owns the heap
for the entire run.

**Aliasing:** Copying a `HeapRef` or `RawPointer` aliases the same heap
object. It does not clone, move, or transfer ownership.

**Freeing:** `unsafe free(value)` releases the heap slot. Freeing is
**shallow** -- if the freed object contains references to other heap
objects, those referenced objects are not freed; they remain live until
separately freed or the run ends.

**Stale references:** After `unsafe free(value)`, all `HeapRef` and
`RawPointer` values that point to the freed slot become stale. Any
access to them produces a runtime error, even if a new object is later
allocated at the same address.

**Pointers to fields/elements:** Raw pointers to array elements or
struct fields remain valid across mutation of the same live object. They
become stale when the base object is freed.

# Resource Limits

All limits are enforced before allocation. Exceeding a limit produces a
`TinyOneError::Runtime` rather than unbounded host allocation.

- **Dynamic array length:** 65,536 elements. `push` beyond the limit or
  `array(count, ...)` with `count` above it fails.
- **Single buffer allocation:** 1,048,576 bytes (1 MiB). `buffer(size)`
  above the limit fails.
- **Total live heap payload:** 4 MiB. Any allocation that would exceed
  the total live bytes fails.
- **Live heap object slots:** 1,000,000 objects. Any allocation at the
  limit fails.
- **Nested TinyLang calls:** 16 calls. `CALL` at the call-depth limit
  fails.

# Shutdown Drain

At program exit, the runtime drains all remaining live heap objects. The
`TinyRunReport` includes:

- `heap_before_shutdown` -- statistics immediately before the drain.
- `heap_after_shutdown` -- statistics after the drain; `shutdown_frees`
  counts objects freed by the drain.

The drain is not triggered by `unsafe free` -- only by runtime
shutdown.
