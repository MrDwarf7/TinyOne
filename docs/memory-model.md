# Memory Model

TinyOne's runtime manages two memory regions: the **stack-frame memory** (fixed slots per function call) and the **heap** (dynamic allocation for all aggregate values). This document covers how each region works, how references are validated, and what resource limits apply.

For the value types that use heap allocation, see [`syntax/types.md`](syntax/types.md). For the opcodes that interact with memory, see [`bytecode.md`](bytecode.md).

---

## Stack-Frame Memory

Each function call allocates a fixed number of slots determined at compile time (`Function.slot_count`). Slots are stored in a flat `Vec<Value>` (`TinyMemory`) shared across the entire call chain as contiguous frame slices.

- Slots are **zero-initialized** at frame entry (value `Int(0)`).
- Slots are **not freed** between loop iterations — a slot retains its last-written value until the function returns.
- Slot count is fixed; the compiler allocates new slots for every `let` declaration. Block exit does not reclaim slots; names are hidden after their scope but the slot remains allocated in the frame.
- At function return, the frame slice is discarded.
- Function chunks can read top-level slots through `LOAD_GLOBAL` when the
  variable was declared before the function. They cannot assign those slots
  directly; shared mutation should happen through heap objects stored in a
  top-level slot.

---

## Heap Architecture

`TinyHeap` is a **generational slab**:

- An `objects: Vec<Option<HeapObject>>` vector holds all heap objects. Each slot is either `None` (free) or `Some(HeapObject)`.
- A parallel `generations: Vec<u64>` vector holds the generation counter for each slot. The counter increments on every allocation and free at that slot.
- A `free: Vec<usize>` list holds the indices of currently free slots.

**Allocation:** claims the next free slot from `free` (or appends a new slot if `free` is empty), increments the generation, and stores the object.

**Deallocation (`unsafe free`):** sets the slot to `None`, increments the generation, and adds the index to `free`.

---

## `HeapRef` and Generation Validation

Every reference to a heap object is a `HeapRef { address: usize, generation: u64 }`.

Before any access to a heap object, the runtime checks:

```
stored_generation[address] == ref.generation
```

If they differ — because the slot was freed and possibly reallocated since `ref` was created — the runtime returns `TinyOneError::runtime("Stale heap reference …")`.

This catches **use-after-free** and prevents a new allocation at the same address from being mistaken for the old object.

---

## `RawPointer` and Validation

A `RawPointer { address, kind, index, field, generation, cast }` derives from a `HeapRef` and adds:

- `kind` — `"object"`, `"array"`, `"buffer"`, `"struct"`, `"cell"`, or `"null"`
- `index` — element or byte offset (for array and buffer pointers)
- `field` — field name (for struct field pointers)
- `generation` — generation at pointer creation time
- `cast` — optional type annotation set by `cast_ptr`

Before any pointer use, the runtime validates in order:

1. **Base generation** — `stored_generation[address] == pointer.generation`
2. **Kind** — the live object at `address` matches `pointer.kind`
3. **Bounds** — `index` is within the object's element or byte count

A stale base object, kind mismatch, or out-of-bounds access each produce a structured runtime error rather than undefined behavior.

---

## Ownership Rules

TinyOne does not use garbage collection or compile-time borrow checking. The Rust runtime owns the heap for the entire run.

**Aliasing:** copying a `HeapRef` or `RawPointer` aliases the same heap object. It does not clone, move, or transfer ownership.

**Freeing:** `unsafe free(value)` releases the heap slot. Freeing is **shallow** — if the freed object contains references to other heap objects, those referenced objects are not freed; they remain live until separately freed or the run ends.

**Stale references:** after `unsafe free(value)`, all `HeapRef` and `RawPointer` values that point to the freed slot become stale. Any access to them produces a runtime error, even if a new object is later allocated at the same address.

**Pointers to fields/elements:** raw pointers to array elements or struct fields remain valid across mutation of the same live object. They become stale when the base object is freed.

---

## Resource Limits

All limits are enforced before allocation. Exceeding a limit produces a `TinyOneError::Runtime` rather than unbounded host allocation.

| Resource | Limit | Error condition |
| --- | --- | --- |
| Dynamic array length | 65,536 elements | `push` beyond limit; `array(count, ...)` with `count` > limit |
| Single buffer allocation | 1,048,576 bytes (1 MiB) | `buffer(size)` with `size` > limit |
| Total live heap payload | 4 MiB | Any allocation that would exceed total live bytes |
| Live heap object slots | 1,000,000 objects | Any allocation when object count is at limit |
| Nested TinyOne calls | 16 calls | `CALL` when call depth is at limit |

---

## Shutdown Drain

At program exit, the runtime drains all remaining live heap objects. The `TinyRunReport` includes:

- `heap_before_shutdown` — stats immediately before the drain
- `heap_after_shutdown` — stats after the drain (`shutdown_frees` counts objects freed by the drain)

The drain is not triggered by `unsafe free` — only by runtime shutdown.
