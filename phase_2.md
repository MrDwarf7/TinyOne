# TinyOne Runtime, Type System, and Data Model Plan

## Purpose

This document formalizes the intended direction for TinyOne after the transition from lexical transpiler into a bytecode VM runtime system.

The goals are:

* Stabilize runtime semantics before major feature expansion
* Define the type system and numeric semantics explicitly
* Clarify ownership and mutation rules
* Establish runtime invariants for VM/JIT correctness
* Reduce ambiguity before deeper optimization work
* Preserve architectural simplicity and correctness

---

# Current Architectural State

TinyOne currently contains:

* Lexer
* Parser/compiler
* Bytecode compiler
* Bytecode verifier
* VM runtime
* Adaptive JIT/quickening layer
* Heap/runtime object system
* Unsafe-gated operations
* Checked arithmetic

The architecture has crossed from transpiler experimentation into full runtime-system engineering.

At this stage, semantic stability matters more than feature count.

---

# Pre-v1.0.0 Target Surface

Before `v1.0.0`, TinyOne intends to support:

* Full signed integer spectrum: `i8`, `i16`, `i32`, `i64`
* Full unsigned integer spectrum: `u8`, `u16`, `u32`, `u64`
* IEEE 754 floating-point support
* `bf16` and `fp16`
* Boolean logic with `true` and `false`
* Composite data types

  * structs
  * records
* Sum types

  * enums
  * tagged unions
* Arrays
* Vectors
* Pointers
* References
* Maps
* Dictionaries

This is a broad v1 surface. The main risk is feature count outrunning the type system, verifier, memory model, and runtime invariants.

---

# Development Order

## Recommended Order

```text
1. Finish core builtins
2. Define numeric semantics
3. Define type conversion rules
4. Design static/hybrid type system
5. Implement typed IR / HIR / MIR pipeline
6. Add typed bytecode variants
7. Redesign String / Char / CharBuffer / Buffer
8. Define memory ownership and pointer provenance rules
9. Expand verifier into a type-aware verifier
10. Harden VM/JIT equivalence testing
11. Add native-code JIT phases only after VM semantics stabilize
```

## Non-goal Before v1.0.0

Do not promise stable bytecode compatibility before the runtime model and type system stabilize.

---

# 1. Core Builtins

## Design Intent

Builtins will be a mixture of:

* VM-native opcodes
* standard-library functions
* Rust-compiled libraries for networking, I/O, and other system integrations

## Unsafe Builtins

A builtin is considered unsafe if it directly affects:

* memory
* controlled data
* runtime state
* pointer state
* allocation state
* system resources

Rule of thumb:

```text
If it could hurt the system, it is unsafe.
```

Safety checks should be applied wherever possible, but there is no guarantee of memory safety for unsafe operations.

## Shadowing and Overloading

Current rule:

* Users cannot define overloads yet.
* Shadowing a builtin throws an error.
* A script that attempts builtin shadowing becomes unusable.

## ABI and Signature Stability

Builtin ABI/signature guarantees are planned for `v1.0.0`.

Before `v1.0.0`:

* No stable ABI guarantee
* No stable builtin signature guarantee
* Builtin behavior may change as runtime semantics stabilize

## Remaining Questions

1. Will unsafe builtins require explicit syntax at call sites?

Example:

```tinyone
unsafe memory.free(ptr)
```

2. Are unsafe builtins allowed in otherwise safe modules?

3. Can unsafe capability be scoped per file, block, function, or module?

4. Are Rust-compiled libraries dynamically loaded, statically linked, or both?

5. Will external Rust libraries use TinyOne ABI, C ABI, or an internal Rust-only ABI?

---

# 2. Numeric Semantics

## Integer Spectrum

TinyOne intends to move from a primarily `i64` numeric runtime to:

```text
i8, i16, i32, i64
u8, u16, u32, u64
```

## Literal Assignment

Integer literals use smallest-fit inference.

Examples:

```text
16   -> u8
-16  -> i8
300  -> u16
```

The exact signedness rule for positive integer literals must be specified because positive values could fit either signed or unsigned types.

## Overflow

Overflow remains a runtime error.

This applies across:

* signed integers
* unsigned integers
* all build modes
* all runtime configurations
* all versions and semantic layers

There is no debug/release distinction.

Rule:

```text
Overflow is overflow.
```

If an action moves outside owned memory or the legal numeric range, it does not happen and the VM reports an error.

## Future Overflow Avoidance

A future runtime-managed pointer/container may help avoid overflow conditions.

Possible candidates:

* `Array<>`
* `Vec<>`
* another runtime-managed allocation/container type

This is not planned before the foundational numeric and memory rules are established.

## Remaining Questions

1. Are integer operations checked before execution or after attempted execution?

2. What is the exact error type for overflow?

3. Is arithmetic on mixed-width values allowed without explicit cast?

4. What is the result type of:

```tinyone
i8 + u8
u8 + u16
i32 + i64
```

5. Does division by zero use the same runtime-error pathway as overflow?

---

# 3. Type Conversion Rules

## Widening

Implicit widening conversions are allowed.

However, edge cases are expected and not yet fully resolved.

Current intent:

* Build the foundation first
* Add defenses and guardrails around widening behavior over time

Example:

```tinyone
let x: i8 = 5
let y: i64 = x
```

## Narrowing

Implicit narrowing conversions are not forbidden.

This is intentionally one of the first areas where guardrails will be implemented.

This is a high-risk rule. If implicit narrowing remains legal, the verifier and runtime must make narrowing behavior explicit and checked.

## Signed and Unsigned Movement

Moving from signed to unsigned is not supported, but currently will not throw an error.

This is semantically ambiguous.

If unsupported behavior does not throw, the runtime must define what happens instead:

* no-op?
* warning?
* rejected by verifier later?
* conversion skipped?
* value preserved with source type?
* runtime poison/error state?

## Casts

Casts do not have to use explicit syntax, but explicit casts will be available.

Example syntax candidate:

```tinyone
let x = i32(value)
```

## Remaining Questions

1. What exactly happens when signed-to-unsigned movement is attempted?

2. Should implicit narrowing be allowed only when the value is provably in range?

3. Should narrowing be runtime-checked when not statically provable?

4. Are lossy casts ever allowed?

5. Should explicit casts be allowed to fail?

6. Does smallest-fit inference prefer unsigned for all non-negative literals?

7. What type does `0` infer to?

8. What type does `-0` infer to, if accepted?

---

# 4. Type System Architecture

## Design Intent

The type system is expected to become one of the largest and most important parts of TinyOne.

Typing is intended to be:

```text
static / hybrid
```

The goal is a large type-safety system, not merely lightweight annotation support.

## Compilation Layers

Planned layers include:

* HIR
* MIR
* typed IR
* typed bytecode variants

Potential future:

* AST-to-bytecode compilation path

This future path should not undermine the typed pipeline unless it is explicitly limited to simple or trusted cases.

## Generics and Templates

Planned:

* generics
* templates

## Interfaces / Traits / Protocols

Planned:

* traits
* interfaces
* protocols

Exact distinctions between these constructs are not yet defined.

## Runtime Values

Current intent:

* Runtime values remain dynamically tagged for now.
* A safer and more optimal approach may replace this later.
* Runtime values are only tagged when the bytecode/runtime has a reason to preserve the tag.

## Type Erasure

Type erasure is expected at runtime.

This must be reconciled with:

* typed bytecode variants
* runtime provenance checks
* generics/templates
* JIT specialization
* reflection/introspection, if any

## Remaining Questions

1. What type information survives after erasure?

2. Are generics monomorphized, type-erased, or hybrid?

3. Are templates compile-time only?

4. What is the difference between traits, interfaces, and protocols in TinyOne?

5. Are sum types nominal, structural, or both?

6. Are structs nominal, structural, or both?

7. Does the type system include ownership/lifetime checking?

8. Are unsafe operations type-system-visible?

---

# 5. String Model

## Design Intent

`String` is:

* UTF-8
* interned
* immutable at the VM level
* immutable at the language level
* indexed by Unicode scalar
* zero-copy sliced

UTF-8 is the standard encoding for all of TinyOne.

## Reference Counting

Reference counting is avoided where possible.

If a reference counter is implemented, then strings may participate in reference-counted ownership. Otherwise, strings remain non-reference-counted under the chosen ownership/allocation model.

## Slices

Everything is intended to use zero-copy slices.

This creates direct lifetime and ownership implications because zero-copy string slices must not outlive their backing string allocation.

## Remaining Questions

1. How are interned strings freed without GC?

2. Are interned strings immortal for the life of the VM/process?

3. Can interned strings be manually freed?

4. What happens to zero-copy slices if the source string is freed?

5. Are string slices owning or borrowing?

6. Does the verifier track string slice lifetimes?

7. Are Unicode scalar indexes cached or computed on demand?

8. What is the complexity guarantee for string indexing?

9. Can invalid UTF-8 ever enter a `String`?

---

# 6. Char and CharBuffer

## Char

`Char` semantically represents a single UTF-8-compatible Unicode scalar.

Rules:

* `Char` is immutable.
* `Char` remains its original value.
* `Char` is pointer-backed.
* `Char` is globally stack allocated.
* `Char` contains exactly one Unicode scalar.
* `Char` is fixed-width and only large enough for its original scalar value.

## CharBuffer

`CharBuffer` is a growing buffer of `Char` values.

Rules:

* Each individual `Char` remains immutable.
* The `CharBuffer` as a collection is mutable.
* `CharBuffer` may contain multiple Unicode scalars because it contains multiple `Char` values.
* `CharBuffer` differs from `String` because `String` is immutable and remains at its created size.

## Important Clarification

The earlier ambiguous idea of `Char[growing]` should be treated as:

```text
CharBuffer[growing]
```

not:

```text
Char[growing]
```

A `Char` is not the growable unit. A `CharBuffer` is.

## Remaining Questions

1. What does "globally stack allocated" mean precisely?

2. Can a pointer-backed `Char` outlive the stack frame that introduced it?

3. Are all possible `Char` values preallocated globally?

4. Is `Char` represented as UTF-8 bytes, `u32`, or pointer to interned scalar object?

5. What does "fixed-width, only large enough for original scalar value" mean when UTF-8 scalars require 1 to 4 bytes?

6. Is `Char` equality pointer equality, scalar-value equality, or both depending on context?

7. Can `CharBuffer` be converted to `String` without copying?

8. Can `String` be converted to `CharBuffer` without copying?

---

# 7. Buffer Model

## Design Intent

`Buffer` is generic and behaves conceptually like:

```text
alloc(sizeof(x))
```

Rules:

* Buffers are VM objects.
* Buffers are generic.
* Buffers are contiguous.
* Buffers are resizable.
* Buffers are owning.
* Buffers are governed by the ownership model.
* Buffers are capable of holding any type or raw data.
* Growth behavior is VM-configurable.
* Buffers and arrays both remain, but with different jobs.

## Resize Operations

Given:

```tinyone
x = buffer(sizeof(x))
```

A developer may call:

```tinyone
x.expand(size)
x.shrink(size)
```

where `size` is the requested resize amount.

## Arrays vs Buffers

Buffers and arrays remain separate constructs.

This distinction must be formally defined.

Possible distinction:

* `Buffer`: raw, low-level, allocation-oriented VM object
* `Array<T>`: typed, bounds-checked, high-level sequence

This is only a candidate distinction and must be confirmed.

## Slices and Views

Slices/views are not planned yet.

Potential future target:

* `v2.0.0`

## Remaining Questions

1. Is `Buffer<T>` typed at compile time, runtime, or both?

2. Can `Buffer` hold mixed types?

3. If `Buffer` can hold any type or raw data, how does the VM know layout?

4. Does `Buffer` store values inline or pointers to values?

5. Are `expand()` and `shrink()` byte-based, element-based, or type-based?

6. Does `expand()` preserve existing pointers into the buffer?

7. Does `shrink()` invalidate pointers into removed regions?

8. Are buffer bounds checked on every access?

9. Does `Buffer` participate in typed bytecode verification?

10. Can buffers be stack allocated?

---

# 8. Memory Model

## Design Intent

TinyOne does not use a traditional garbage collector.

Heap allocation is owned by the programmer.

Memory may be:

* manually freed
* arena allocated
* pooled
* eventually reference counted
* freed over time by a built-in system similar to arena allocation/deallocation

This built-in freeing system is close to a GC in effect, but is intended to be closer to arena allocation/deallocation.

## Ownership

All memory is based on an ownership model.

The exact ownership rules are not yet fully planned.

## Pointer Invalidation

Pointers can become invalid when:

* they are no longer in scope
* they become null
* the developer destroys/frees them
* the VM kills them off

Pointer movement and stability depend on:

* pointer type
* owner
* allocation mechanism
* whether the value came from `Buffer`, `Alloc`, `Ptr`, `Array`, or `Vec`

Pointer stability is not yet guaranteed.

Pointer-stability safeties are intended before `v1.0.0`.

## Borrowing and Lifetimes

Planned:

* borrow semantics
* lifetime semantics

## Destruction

Planned:

* destructors

Not planned:

* finalizers

## Cyclic Ownership

Cyclic ownership can exist, but it is discouraged.

This must be made precise because cyclic ownership without GC, reference counting, or ownership restrictions can leak memory or create invalid teardown order.

## Allocation Provenance

The VM is the source of truth for:

* allocation provenance
* allocation rules
* pointer validity
* memory ownership constraints

## Remaining Questions

1. What is the exact difference between `Ptr`, `Ref`, `Alloc`, `Array`, `Vec`, and `Buffer`?

2. Which allocations are programmer-owned versus VM-owned?

3. What does it mean for the VM to "kill" a pointer?

4. Can the VM invalidate a pointer while user code still has a reference?

5. Are references non-owning?

6. Are pointers nullable by default?

7. Are references nullable?

8. Are arenas lexically scoped, manually scoped, or VM scheduled?

9. Are destructors deterministic?

10. How are cyclic ownership cases detected or reported?

11. How do zero-copy slices remain safe without a complete lifetime system?

---

# 9. VM and Bytecode Stability

## VM Model

TinyOne intends to remain a stack VM.

## Bytecode Mutation

Opcode specialization may mutate bytecode in-place through:

* JIT hot paths
* optimizers
* adaptive execution mechanisms

## Serialization

Bytecode is intended to be serializable.

Possible formats:

* picklization
* any developer-needed serialization format
* future standardized TinyOne bytecode package format

## Compatibility

Bytecode will not always be backward compatible.

Intent:

* preserve as much compatibility as possible
* do not promise full compatibility during early versions
* expect substantial bytecode instability during `v1.x`

## Sandboxing

Sandboxed execution is not planned until at least `v2.0.0`.

## Verifier

The verifier will become type-aware.

This is required if TinyOne supports:

* typed bytecode
* pointer provenance
* ownership rules
* unsafe restrictions
* typed numeric operations
* generics/templates

## Remaining Questions

1. Can optimized bytecode be persisted?

2. Can JIT-mutated bytecode be serialized?

3. Should bytecode mutation preserve original debug/source mapping?

4. Is bytecode verification required before every execution?

5. Are bytecode files trusted or always verified?

6. Will bytecode carry type metadata after type erasure?

---

# 10. JIT Direction

## Design Intent

Native code generation is the next step in TinyOne's multi-phase JIT system.

The JIT will not be entirely architecture-independent.

The long-term JIT strategy includes:

* folding
* hot-path compilation
* assembly/native code generation
* opcode optimization
* threaded VM-style execution
* tracing JIT behavior
* method/baseline JIT behavior
* optimizing JIT behavior
* speculative optimization

In practice, all major JIT modes may exist simultaneously as different tiers or phases.

## Profiling

Full profiling is intended as part of the standard library.

Profiling areas:

* hot paths
* memory
* pointer allocation
* I/O
* general runtime behavior

## IR Sharing

JIT tiers will share IR with the compiler.

This is the correct long-term direction if TinyOne intends to avoid duplicate semantic implementations.

## Remaining Questions

1. Which native architectures are first-class targets?

2. Will native JIT be optional at build time?

3. How will executable memory be managed safely?

4. How will deoptimization work?

5. What speculative assumptions are allowed?

6. How are speculative assumptions invalidated?

7. Can unsafe code be JIT-optimized?

8. Are JIT tiers deterministic enough for test reproducibility?

9. Is the JIT allowed to change observable performance only, or can it change error timing?

---

# 11. Open Semantic Areas

## Char

Resolved intent:

* `Char` is a single UTF-8-compatible Unicode scalar.
* `Char` is immutable.
* `CharBuffer` is the mutable growing collection.

Remaining ambiguity:

* exact allocation model
* exact fixed-width representation
* pointer/equality semantics

## Ownership

Current status:

* not fully clear
* more work needed before complete planning

This is the highest-priority unresolved area because it affects:

* zero-copy slices
* buffers
* pointers
* references
* destructors
* cyclic ownership
* VM invalidation
* JIT safety

## Pointer Invalidation

Pointers can become:

* null
* destroyed by the developer
* killed by the VM
* invalid due to scope exit

The exact rules must be formalized before safety claims are made.

## Buffer Movement

Buffers are partially movable.

There is no promise that movement is zero-copy or direct.

This must be reconciled with pointers into buffers.

## Runtime Type Model

Currently in planning.

Known intent:

* runtime tagging only where useful
* type erasure expected
* typed bytecode planned
* static/hybrid type system planned

## Compile-Time Typing Amount

Typing before bytecode emission depends on:

* optimizer flags
* bytecode form
* IR layer
* VM mode

This flexibility is powerful but increases risk. The compiler must still preserve a single source of semantic truth.

## Overflow

Current rule:

```text
On overflow, the VM throws an error.
```

## Bytecode Stability

Eventually planned.

Not promised in `v1.0.0`.

`v1.x` may still have bytecode instability across versions.

## Native Code

Long-term target:

* native code
* hot-path compilation
* optimized bytecode
* full multi-phase JIT

---

# Highest-Risk Contradictions / Ambiguities

These should be resolved before implementation hardens.

## 1. Type Erasure vs Typed Bytecode

TinyOne plans both:

* typed bytecode variants
* runtime type erasure

This is valid only if the compiler defines what type information remains available at runtime and what is erased.

## 2. Zero-Copy Slices vs Manual Ownership

TinyOne plans:

* zero-copy slices
* no traditional GC
* programmer-owned heap allocation
* future lifetime semantics

Until lifetimes are implemented, zero-copy slices are a major dangling-reference risk.

## 3. Interned Strings Without GC

String interning requires a lifetime strategy.

Options:

* immortal intern table
* arena-lifetime intern table
* manual intern release
* reference-counted intern table
* VM-owned intern table with epoch cleanup

## 4. Implicit Narrowing

Implicit narrowing is allowed but intended to receive guardrails.

This is risky for a correctness-first type system.

The minimum safe rule should be:

```text
Implicit narrowing is allowed only when statically proven safe or dynamically checked.
```

## 5. Signed-to-Unsigned Unsupported But Non-Erroring

Unsupported behavior that does not error needs a defined result.

Otherwise it becomes silent semantic ambiguity.

## 6. Pointer Stability Not Yet Guaranteed

Many planned features depend on pointer stability or clear invalidation rules:

* buffers
* zero-copy slices
* references
* Char pointer backing
* native JIT
* optimizer assumptions

This should be addressed before v1.0.0 safety claims.

---

# Immediate Next Questions

The following areas remain architecturally important even after clarification.

1. Exact native JIT executable-memory model
2. Final deoptimization strategy, if any
3. Formal ownership graph model
4. Exact verifier/type-checker/VM responsibility boundaries
5. Pointer provenance encoding inside bytecode and IR
6. Formal lifetime semantics
7. Exact tagged-pointer encoding rules
8. Formal ABI specification
9. Final interned-string cleanup lifecycle
10. Formal unsafe semantic guarantees

---

# Final Clarifications and Design Decisions

## Unsafe Semantics

Unsafe builtins require explicit unsafe syntax.

Planned forms:

```tinyone
unsafe {
  memory.free(ptr)
  // other unsafe operations
}
```

Unsafe code may exist inside otherwise safe contexts.

Rule:

* Unsafe operations are allowed.
* Unsafe operations are expected to fail.
* Runtime safeguards and rollback systems attempt to recover safely.

Unsafe capability may be scoped:

* per file
* per function
* per module

## Foreign Libraries

Rust-compiled libraries require wrappers similar to Python extension wrapping.

Foreign ABI rules:

* Rust libraries may use TinyOne ABI or Rust ABI.
* C/C++ libraries require TinyOne wrappers.
* Wrapper overhead is accepted as a safety-first tradeoff.

---

# Overflow and Snapshot Semantics

## Snapshot Execution Model

Before any operation where overflow is possible:

1. The VM creates a snapshot.
2. The operation executes.
3. If overflow or failure occurs:

   * the snapshot is restored
   * execution returns `False` or an error state

Overflow error type:

```text
Runtime.Memory_Overflow
```

This is effectively transactional arithmetic.

## Division by Zero

Division-by-zero has its own dedicated runtime error.

---

# Mixed-Width Arithmetic

Mixed-width arithmetic is allowed.

Known promotion rules:

```text
i8 + u8   -> i16
u8 + u16 -> u32
```

The rule for:

```text
i32 + i64
```

remains unresolved.

The current system appears to prefer widening toward a type capable of safely representing both domains.

---

# Signed to Unsigned Conversion

Current rule:

```text
signed -> unsigned
```

means:

```text
flip the sign bit to zero
```

This is simple but dangerous because it is not mathematically equivalent conversion.

Example concern:

```text
-1 -> 1?
```

This rule should be documented very explicitly because most programmers will assume two's-complement reinterpretation or checked conversion semantics.

---

# Narrowing and Runtime Checks

Rules:

* Narrowing is allowed only when provably in range.
* Narrowing is runtime checked.
* If statically provable, runtime checks may be delayed or optimized.
* TinyOne still validates arithmetic before trusting static assumptions.

This is a correctness-first design.

## Lossy Casts

Lossy casts are allowed only when the loss is not considered excessive.

Example:

```text
fp32 -> i16 = error
```

unless performed inside:

```tinyone
unsafe {}
```

## Explicit Cast Failure

Explicit casts may fail inside unsafe blocks.

Unsafe blocks are expected to permit failure.

Snapshot rollback restores previous runtime state.

---

# Literal Rules

## Smallest-Fit Preference

Smallest-fit inference always prefers unsigned types over signed types for non-negative values.

Examples:

```text
16 -> u8
255 -> u8
256 -> u16
```

## Zero Semantics

`0` becomes boolean.

`"0"` becomes a `Char`.

`-0` has no type.

Current semantic intent:

```text
Zero is non-negative.
Negative zero is null arithmetic.
```

`-0` becomes a null pointer because the arithmetic is considered null.

This is highly unconventional and should be documented aggressively if preserved.

---

# Type Erasure

Type erasure destroys the type.

If the type remains inferable, type metadata may remain constant.

The stated goal is:

```text
Encourage developers NOT to use type erasure.
```

## Generics

Generics are monomorphized.

## Templates

Templates are:

* compile-time generics
* runtime-usable constructs

---

# Interface / Trait / Protocol Distinction

## Interface

Strict contract for method signatures.

## Trait

Provides:

* default implementations
* retroactive modeling support

## Protocol

Focused on:

* structural typing
* communication rules

## Structural Typing

Current rules:

* Sum types are structural.
* Structs are structural.

## Ownership and Lifetimes

* Ownership checking belongs to the type system.
* Lifetime checking belongs to the VM.
* Unsafe operations are visible to the type system.

---

# Interned String Lifecycle

Interned strings are VM managed.

The VM uses a "health check" style lifecycle system instead of traditional GC.

Analogy:

```text
Periodic server pinging / validation
```

Rules:

* Interned strings are not immortal.
* Interned strings usually survive most of the VM lifetime.
* Interned strings may be manually freed via stdlib.
* Interned strings are not freed through default builtins.

## String Slices

Rules:

* Slices are borrowing.
* If the source string dies, the slice dies.
* The contract is dead once the host dies.
* The verifier quantifies string-slice lifetimes for the VM.

## UTF-8 Validation

Invalid UTF-8 cannot enter:

* String
* Char
* CharBuffer

UTF-8 validation is the first type-checking stage for text.

## String Indexing

Unicode scalar indexes:

* cached when necessary
* computed otherwise

No complexity guarantee exists for indexing.

---

# VM Stack Model

## Stack Layout

TinyOne has:

* local function stacks
* VM-maintained global stack

The global stack exists for:

* hot-path optimization
* fast read/write behavior

## Char Lifetime

`Char` values are contractually tied to their stack.

They cannot outlive it.

## Char Allocation

All possible `Char` values are NOT globally preallocated.

Reason:

* excessive RAM usage

Instead:

* Char representations exist in cold-cache/code form
* values are pulled into memory when required

## Char Representation

`Char` is represented as:

```text
pointer -> interned scalar object
```

Storage width is exactly large enough for the UTF-8 scalar.

Examples:

```text
1-byte scalar -> 1-byte char
4-byte scalar -> 4-byte char
```

## Equality

`Char` equality may be:

* pointer equality
* scalar equality

depending on context.

## Conversion Rules

```text
CharBuffer -> String
```

is allowed.

```text
String -> CharBuffer
```

creates a copied buffer.

This is NOT zero-copy.

---

# Buffer Semantics

## Buffer Typing

`Buffer<T>` is:

* typed at compile time
* verified at runtime
* enforced at runtime

## Storage Model

Buffer uses:

* tagged pointers
* separation

Buffers store pointers to values.

## Resize Semantics

`expand()` and `shrink()` are:

* byte-based
* element-based

based on the argument type.

## Pointer Preservation

`expand()` preserves existing pointers.

The buffer appends additional pointers and maintains ownership.

## Shrink Behavior

`shrink()` kills trailing pointers until the requested shrink size is achieved.

## Bounds Checking

Buffer bounds are checked on:

* write
* modify

No statement currently guarantees read bounds checks.

## Verification

Buffers do not participate directly in typed bytecode verification.

Reason:

```text
Buffer is a large generic pointer box.
```

## Allocation

Buffers may be stack allocated.

---

# Pointer / Reference / Allocation Model

## Pointer

A pointer:

* stores a raw memory address
* may be raw
* may be dangling
* lives on the stack

## Reference

A reference:

* lives on the stack
* aliases an existing object
* is non-owning
* is zero-copy
* includes safety safeguards
* is nullable

## Alloc

An alloc:

```text
heap memory reservation
```

## Array

An array:

* fixed size
* compile-time known size
* same-type elements
* stack allocated

## Vec

A vec:

* growable
* heap allocated
* stack-resident control structure
* stores:

  * length
  * capacity
  * pointers

## Buffer

A buffer:

* moving block of data
* context-dependent location
* primarily intended for:

  * I/O
  * printing
  * String/Char/CharBuffer
  * file reads/writes

---

# Pointer Lifecycle

## VM Killing Pointers

When the VM kills a pointer:

* the lifecycle is considered over
* the allocation is deallocated

## Reference Safety

The VM cannot invalidate a referenced pointer.

This is a major semantic guarantee.

## Nullability

Null pointers exist explicitly.

References are nullable.

## Arena Scheduling

Arena behavior is:

* VM scheduled
* lexically scoped

## Destructors

Destructors are deterministic.

## Cyclic Ownership

Cyclic ownership detection involves:

* verifier
* type checker
* VM

working together.

## Zero-Copy Slice Death

When the host dies:

```text
the zero-copy slice dies with it
```

---

# Bytecode and Verification

## Persistence

Optimized bytecode can be persisted.

JIT-mutated bytecode can also be serialized.

This requires substantial metadata.

## Debug Mapping

Mutation must preserve:

* debug mapping
* source mapping

## Verification

Bytecode is verified.

Verification does NOT occur before every execution because of latency concerns.

## Type Metadata

Bytecode carries type metadata.

---

# Native JIT

## Build Mode

Native JIT is optional.

Currently enabled through explicit flags.

## Safety Model

No finalized executable-memory safety model exists yet.

## Deoptimization

No clear deoptimization strategy currently exists.

Current sentiment:

```text
Why would we want deoptimization?
```

This is acceptable early on, but speculative optimization generally requires invalidation/deoptimization mechanisms once assumptions can fail.

## Speculative Optimization

Rules:

* assumptions are allowed once discovered
* assumptions are invalidated once disproven

## Unsafe Optimization

Unsafe code may be JIT optimized.

## Determinism

JIT tiers are intended to be highly deterministic and reproducible.

## Optimization Authority

The JIT has broad optimization freedom.

---

# Current Safety Strategy

## Zero-Copy Slice Risk

The project explicitly acknowledges:

```text
Zero-copy slices are a dangling-reference risk.
```

Current mitigation:

* VM tracks memory addresses in use
* VM kills addresses on EOF/lifecycle end

## Reference Counting Direction

Planned direction:

* reference counting
* lifecycle-based management

Current implementation still relies heavily on:

```text
16 calls rule
```

inside the VM.

The exact meaning of the 16-calls rule should eventually be formalized.
