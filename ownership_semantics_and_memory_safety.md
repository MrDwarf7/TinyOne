# Ownership Semantics and Memory Safety for TinyOne

## Overview

TinyOne requires a memory management model that preserves:

* Deterministic cleanup semantics
* Predictable latency
* High execution throughput
* Python-like ergonomics
* Rust-grade safety guarantees inside the VM runtime
* Minimal runtime coordination overhead

Traditional tracing garbage collectors introduce several costs that conflict with these goals:

* Stop-the-world pauses
* Heap scanning overhead
* Poor cache locality
* Increased allocator pressure
* Non-deterministic destruction timing

At the same time, a purely explicit ownership system comparable to Rust introduces syntax and semantic complexity that conflicts with a Pythonic language model.

This document proposes a hybrid architecture based on:

* Indexed Reference Counting (IRC)
* Manifest Ownership semantics
* Region-based lifetime batching
* VM-level address validation
* Deferred cleanup
* Rust-backed allocator safety

The result is a deterministic, soft-safe runtime with low coordination overhead and predictable destruction behavior.

---

# Design Goals

## Primary Goals

1. Deterministic object destruction
2. No tracing garbage collector
3. No stop-the-world pauses
4. Safe handling of dangling references
5. Pythonic syntax ergonomics
6. Efficient multi-threaded execution
7. Minimal atomic reference traffic
8. Strong VM integrity guarantees

## Non-Goals

TinyOne intentionally does not attempt to:

* Eliminate all runtime overhead
* Provide compile-time ownership verification
* Guarantee lock-free object access everywhere
* Fully emulate Rust borrow checking semantics
* Transparently solve arbitrary cyclic graphs

The VM prioritizes practical runtime determinism over formal ownership proofs.

---

# Core Architecture

## Hybrid Ownership Model

TinyOne uses a hybrid ownership model:

| Layer                         | Responsibility                      |
| ----------------------------- | ----------------------------------- |
| Rust runtime                  | Hard memory safety for VM internals |
| VM allocator                  | Object lifecycle coordination       |
| Global Allocation Table (GAT) | Address validity source of truth    |
| IRC subsystem                 | Lifetime accounting                 |
| Region system                 | Batched cleanup                     |
| Bytecode validator            | Soft runtime protection             |

The VM itself owns all heap allocations.

User-level objects never directly manage native memory.

All object references are represented as VM-managed addresses or handles.

---

# Indexed Reference Counting (IRC)

## Motivation

Traditional reference counting systems often colocate the reference count with the object header.

This creates several issues:

* Poor cache locality
* Frequent atomic writes
* Cross-core cache invalidation
* High contention in concurrent workloads
* Increased allocator fragmentation

TinyOne separates object metadata from object payloads.

## Global Allocation Table (GAT)

The runtime maintains a centralized allocation index.

The GAT tracks:

* Allocation state
* Reference count
* Ownership flags
* Region membership
* Thread ownership
* Escape state
* Weak reference tracking
* Finalization metadata

Example conceptual structure:

```rust
pub struct AllocationEntry {
    pub state: AllocationState,
    pub strong_refs: u32,
    pub weak_refs: u32,
    pub region_id: RegionId,
    pub owner_thread: ThreadId,
    pub escaped: bool,
}
```

The GAT becomes the canonical runtime authority for object validity.

---

## Deferred Reference Counting

TinyOne uses deferred reference counting rather than eager increment/decrement operations.

The VM aggressively avoids reference updates in common short-lived execution paths.

### Borrow-First Semantics

Function calls default to borrowing semantics.

Example:

```python
fn process(value):
    return value.name
```

No ownership transfer occurs.

No reference increment occurs.

The caller retains ownership responsibility.

### Promotion Rules

Reference promotion only occurs when:

* An object escapes the current frame
* An object is stored in a heap structure
* An object becomes globally reachable
* Ownership crosses thread boundaries

This dramatically reduces reference churn.

---

## Weighted Reference Counting

To further reduce synchronization overhead, TinyOne may implement weighted references.

Instead of incrementing by single units:

* Ownership can be subdivided into weighted shares
* Local ownership changes avoid global synchronization
* Threads can batch ownership transfers

Example:

```text
Parent owner: weight = 1024
Child borrow: consumes partial local weight
Global synchronization only occurs when local weight exhausts
```

Benefits:

* Fewer atomic operations
* Better thread scalability
* Reduced cache invalidation
* Improved throughput under parallel workloads

Tradeoff:

* Increased implementation complexity
* More complicated debugging semantics
* Additional allocator bookkeeping

Weighted references should remain an implementation detail.

---

# Region-Based Ownership

## Motivation

Reference counting overhead becomes problematic when every temporary allocation requires synchronization.

Most temporary objects are stack-scoped.

TinyOne exploits this behavior using region-based cleanup.

---

## Shadow Stack

Each VM frame maintains a shadow allocation list.

The shadow stack tracks:

* Allocations created within the frame
* Temporary ownership bindings
* Escape candidates
* Deferred decrements

Conceptually:

```rust
pub struct FrameRegion {
    pub allocations: Vec<Address>,
    pub escaped: BitSet,
}
```

---

## Bulk Release

At frame exit:

1. The VM scans the frame region
2. Escaped objects are preserved
3. Non-escaped objects are released in bulk
4. Deferred decrements are flushed
5. Dead allocations are finalized

This avoids thousands of individual decrement operations.

Benefits:

* Better cache locality
* Reduced allocator contention
* Lower synchronization overhead
* Faster temporary object cleanup

This model resembles arena allocation behavior while preserving object-level ownership semantics.

---

# Address Validation and Soft Safety

## Safety Philosophy

TinyOne cannot rely entirely on compile-time ownership validation.

Instead, runtime safety is enforced through:

* VM-controlled address validation
* Rust runtime integrity
* Centralized allocation tracking

The VM prevents undefined behavior by rejecting invalid accesses before dereferencing.

---

## Address Validator

Before any object dereference:

1. The VM validates the address against the GAT
2. The allocation state is checked
3. Ownership visibility is verified
4. Thread access rules are enforced

Conceptual implementation:

```rust
fn access_memory(vm: &VM, addr: Address) -> Result<&Object, MemoryError> {
    if vm.gat.contains_key(&addr) {
        Ok(unsafe { &*addr.as_ptr() })
    } else {
        Err(MemoryError::UseAfterFree(addr))
    }
}
```

This provides soft runtime safety.

Invalid accesses become recoverable VM faults rather than process corruption.

---

## Hard Safety Boundary

Rust remains responsible for:

* Internal allocator integrity
* VM metadata correctness
* Thread synchronization correctness
* Prevention of native memory corruption
* Safe destruction ordering

TinyOne code never directly manipulates native pointers.

This separation is critical.

The VM can fail safely without compromising host process integrity.

---

# Multi-Threading Strategy

## Problem

A single global allocation index becomes a scalability bottleneck.

Heavy contention would occur under:

* Frequent allocation
* Cross-thread sharing
* High reference mutation rates

---

## Thread-Local Allocation Buffers (TLABs)

Each VM thread maintains:

* Local allocation regions
* Local deferred decrement queues
* Local ownership caches
* Local reference deltas

Cross-thread synchronization only occurs when:

* Ownership transfers across threads
* Objects become shared
* Regions merge
* Global cleanup occurs

This minimizes contention on the global allocator structures.

---

## Shared Object Promotion

Objects begin as thread-local.

Promotion to shared state occurs lazily.

State transitions:

```text
Thread Local -> Shared Candidate -> Shared Global
```

Only shared objects require:

* Atomic synchronization
* Cross-thread visibility tracking
* Global ownership coordination

Most objects remain thread-local for their entire lifetime.

---

# Cyclic Reference Handling

## Problem

Pure reference counting cannot reclaim cycles.

Example:

```text
A -> B
B -> A
```

Both objects remain permanently reachable.

---

## Explicit Design Decision

TinyOne intentionally avoids a full tracing cycle collector.

Reasons:

* Complexity growth
* Runtime unpredictability
* Pause amplification
* Increased metadata overhead
* More difficult debugging

Instead, TinyOne uses explicit cycle management strategies.

---

## Weak References

Heap structures that may participate in cycles should support weak references.

Weak references:

* Do not contribute to strong ownership
* Can be invalidated safely
* Avoid retention loops

Conceptual structure:

```rust
pub struct WeakHandle {
    pub addr: Address,
}
```

Dereferencing a weak handle requires GAT validation.

---

## Finalizer Queues

The VM may additionally support scoped finalization queues.

At:

* Process shutdown
* Region destruction
* Module unload
* VM reset

The runtime may force-release remaining allocations associated with the scope.

Rust destructors then reclaim native resources.

Tradeoff:

* Cycles may persist longer than ideal
* Cleanup becomes scope-driven rather than immediate

This is acceptable for a deterministic non-GC runtime.

---

# Ownership Semantics

## Recommended Model

TinyOne should use:

* Inferred ownership
* Explicit borrowing
* Implicit move semantics where safe

This preserves Pythonic readability while exposing ownership intent when necessary.

---

## Example Semantics

### Borrowing

```python
fn print_name(user):
    println(user.name)
```

`user` is borrowed.

No ownership transfer occurs.

---

### Escaping Ownership

```python
global cache

fn store(user):
    cache = user
```

The VM promotes `user` to escaped ownership.

Reference state updates occur.

---

### Explicit Borrow

Optional explicit syntax may exist:

```python
fn inspect(&value):
    println(value)
```

This can improve VM optimization opportunities.

However, explicit ownership syntax should remain minimal.

---

# VM Object Model

## Recommended Handle Layout

Objects should be referenced indirectly.

Recommended handle structure:

```rust
pub struct Handle {
    pub addr: Address,
    pub generation: u32,
}
```

Generation counters help prevent stale handle reuse.

This protects against:

* ABA reuse bugs
* Accidental stale dereferences
* Address recycling hazards

---

## Allocation State Machine

Recommended states:

```text
Allocated
Borrowed
Escaped
Shared
PendingRelease
Released
```

Transitions should remain explicit inside the runtime.

---

# Performance Characteristics

## Expected Benefits

Compared to tracing GC systems:

* Lower latency variance
* Better destruction determinism
* Reduced heap scanning
* Better cache locality
* Improved real-time responsiveness
* More predictable frame execution

Compared to eager reference counting:

* Fewer atomic operations
* Better batching behavior
* Lower synchronization overhead
* Reduced temporary object churn

---

## Expected Costs

The system still introduces overhead:

* Allocation table lookups
* Metadata maintenance
* Escape analysis tracking
* Deferred decrement flushing
* Validation checks

However, these costs remain more predictable than tracing collection pauses.

---

# Failure Modes

## Potential Risks

### Global Index Contention

Mitigation:

* TLABs
* Partitioned allocators
* Sharded GAT design

---

### Reference Leaks

Mitigation:

* Scoped cleanup
* Weak references
* Leak diagnostics
* Finalizer queues

---

### Validation Overhead

Mitigation:

* Inline fast-path checks
* Generation-based caches
* Thread-local validation caches

---

### Escape Misclassification

Mitigation:

* Conservative promotion
* Runtime instrumentation
* Ownership debugging tools

---

# Recommended Runtime Strategy

## Final Recommendation

TinyOne should adopt:

1. Deferred indexed reference counting
2. Region-based bulk cleanup
3. VM-level address validation
4. Thread-local allocation regions
5. Explicit weak reference support
6. Inferred ownership semantics
7. Optional explicit borrowing syntax

This provides:

* Deterministic cleanup
* Soft runtime safety
* Predictable latency
* Pythonic usability
* Rust-backed integrity
* Minimal tracing overhead

without requiring a traditional garbage collector.

---

# Conclusion

TinyOne should not attempt to replicate Rust’s compile-time ownership model.

Instead, it should leverage:

* Runtime ownership inference
* VM-managed safety boundaries
* Deferred deterministic cleanup
* Rust-enforced allocator integrity

The result is a pragmatic hybrid runtime:

* Safer than traditional scripting VMs
* Simpler than a borrow-checked language
* More deterministic than tracing GC systems
* More ergonomic than explicit ownership languages

The VM itself becomes the ownership authority.

Rust guarantees runtime integrity.

The GAT guarantees address validity.

Deferred ownership coordination preserves performance.

This architecture aligns closely with TinyOne’s goals:

* Semi-clean syntax
* Predictable execution
* High performance
* Strong runtime safety
* Minimal runtime complexity growth
