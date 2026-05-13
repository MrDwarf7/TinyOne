# typing_system.md

# TinyOne Type System

## Status

This document defines the Phase 2 type-system direction for TinyOne.

Phase 2 is intended to ship incrementally. Every type category listed in this
document is in scope for Phase 2, but the implementation may land across
multiple compatible updates.

Release labels and artifact metadata are non-semantic. They must not affect type
identity, type compatibility, runtime layout, dispatch, or program behavior.

The type system is intended to be:

```text
static / hybrid
```

The compiler performs static checks where possible. The VM remains responsible for runtime validation, ownership enforcement, lifetime checks, pointer provenance, and final safety decisions.

TinyOne does not use traditional garbage collection and will not adopt it. Type-system rules must be compatible with ownership, deterministic destruction, VM-managed provenance, arenas, pools, manual free where allowed, and explicit lifecycle mechanisms.

---

# Core Principles

## 1. Safety First

The type system exists to prevent invalid programs before execution and to provide the VM enough metadata to reject unsafe runtime states.

Unsafe behavior must be explicit.

Current expression syntax:

```tinyone
let ok = unsafe free(ptr)
```

Planned block syntax:

```tinyone
unsafe {
  free(ptr)
}
```

Unsafe operations are visible to the type system.

## 2. Static Where Possible, Runtime Where Necessary

The compiler should prove as much as possible statically.

The VM remains the source of truth for:

- allocation provenance
- lifetime validity
- pointer validity
- runtime bounds
- overflow rollback
- unsafe failure recovery

## 3. Structural by Default

TinyOne uses structural typing for core composite and algebraic types.

Current rule:

- Records are structural.
- Plain data structs may be structural.
- Resource-owning or invariant-bearing structs are nominal.
- Sum types are structural.
- Protocols are structural contracts.

## 4. Explicit Runtime Failure

Operations that may fail must report failure through runtime error states, boolean fallback, or unsafe-block rollback semantics.

Overflow uses:

```text
Runtime.Memory_Overflow
```

Division by zero has a separate dedicated runtime error.

## 5. Type Erasure Is Discouraged

Type erasure destroys type information.

If a type remains inferable after erasure, metadata may remain constant, but semantically the erased type is not available as a full type.

The design goal is to discourage unnecessary type erasure.

---

# Type Categories

TinyOne types are grouped into the following categories:

- Primitive scalar types
- Text scalar and text buffer types
- Composite/product types
- Algebraic/sum types
- Reference and ownership types
- Function types
- Generic types
- Phantom types
- Zero-sized types
- Unsafe/runtime types

---

# Primitive Scalar Types

Primitive scalar types are direct scalar values understood by the compiler, bytecode verifier, VM, and JIT.

## Integer Types

Signed integers:

```text
i8
i16
i32
i64
```

Unsigned integers:

```text
u8
u16
u32
u64
```

## Integer Literal Inference

Integer literals use smallest-fit inference.

Non-negative literals prefer unsigned types.

Examples:

```text
0    -> bool false / null arithmetic special case depending on context
16   -> u8
255  -> u8
256  -> u16
-16  -> i8
```

The string literal:

```text
"0"
```

is text and may produce a `Char` when used as a single-character scalar context.

## Negative Zero

`-0` has no ordinary numeric type.

Current semantic intent:

```text
-0 -> null pointer / null arithmetic
```

This is intentionally special and must be handled explicitly by the compiler and VM.

## Overflow

Overflow is always a runtime error.

This applies to:

- signed integers
- unsigned integers
- debug builds
- release builds
- interpreted VM execution
- JIT execution
- optimized bytecode

Overflow error:

```text
Runtime.Memory_Overflow
```

Before overflow-capable operations, the VM creates a snapshot. If the operation fails, the snapshot is restored and execution returns `False` or an error state.

## Mixed-Width Integer Arithmetic

Mixed-width arithmetic is allowed.

Defined promotion examples:

```text
i8 + u8   -> i16
u8 + u16 -> u32
i32 + i64 -> i64
```

Rule:

```text
integer_binary(lhs, rhs) -> smallest built-in integer type that can represent
both operand domains
```

The promoted type is the operation type and the result type.

Arithmetic still uses checked runtime execution. Promotion does not widen every
operation to a mathematically unbounded result domain, because that would make
ordinary fixed-width arithmetic impossible to represent. If the promoted result
overflows at execution time, the VM reports the appropriate runtime overflow
error and restores the protected state snapshot where applicable.

If no built-in integer type can represent both operand domains, the expression is
a compile-time type error unless an explicit unsafe conversion is used.

## Signed and Unsigned Movement

Signed-to-unsigned movement is not implicit.

Rules:

- lossless widening is allowed implicitly
- narrowing requires a checked explicit conversion
- signed-to-unsigned conversion requires a checked explicit conversion
- unsigned-to-signed conversion requires a checked explicit conversion unless
  the source range is statically known to fit
- bit reinterpretation is an unsafe operation, not a numeric conversion
- sign-bit clearing, if exposed, is an explicit unsafe bit operation

This removes ambiguity between mathematical conversion, range-checked casting,
and raw bit manipulation.

## Narrowing

Narrowing is allowed only when the value is provably in range.

Rules:

- narrowing is runtime checked
- static proof may allow delayed or optimized runtime checks
- lossy casts are rejected by default
- unsafe syntax may allow explicit lossy or bit-level conversion attempts
- failed checked narrowing is a runtime error, not silent truncation

Example excessive loss:

```text
fp32 -> i16 = error unless an explicit unsafe conversion is used
```

---

# Floating-Point Types

TinyOne intends to comply with IEEE 754 floating-point behavior.

Planned floating types:

```text
bf16
fp16
fp32
fp64
```

Rules:

- NaN follows IEEE 754 comparison behavior: NaN is unordered and not equal to
  itself
- quiet NaNs propagate through ordinary arithmetic
- VM and compiled execution may canonicalize NaN payloads at observable
  boundaries for deterministic behavior
- signed zero is preserved
- infinities are preserved
- the default rounding mode is round-to-nearest, ties-to-even
- TinyOne code does not change the ambient floating-point rounding mode
- float overflow produces infinity according to IEEE 754
- float underflow follows IEEE 754 subnormal/zero behavior
- float-to-integer conversion is checked
- converting NaN, infinity, or an out-of-range finite float to an integer is a
  runtime error
- integer-to-float conversion follows IEEE 754 rounding for the target type

Floating-point behavior must remain compatible across VM and JIT tiers.

---

# Boolean Type

TinyOne supports explicit boolean logic:

```text
true
false
```

The literal `0` may infer to boolean false depending on context.

Boolean values are not arbitrary integers unless explicitly converted.

Rules:

- `bool` is a distinct scalar type
- `true` and `false` are boolean values, even if optimized as constants
- nonzero integers do not implicitly convert to `true`
- integer-to-boolean conversion must be explicit
- booleans do not participate in arithmetic
- comparisons produce `bool`
- control-flow conditions require `bool` in typed code
- untyped compatibility execution may continue treating integer `0` and `null`
  as false until typed boolean bytecode is complete

---

# Text Scalar and Text Buffer Types

UTF-8 is the standard encoding for all TinyOne text.

Invalid UTF-8 cannot enter:

- `String`
- `Char`
- `CharBuffer`

UTF-8 validation is the first type-checking stage for text values.

---

## Char

`Char` represents exactly one UTF-8-compatible Unicode scalar.

Rules:

- immutable
- pointer-backed
- tied to its stack lifetime
- represented as a pointer to an interned scalar object
- exactly large enough to store its UTF-8 scalar payload
- may use pointer equality or scalar equality depending on context

Examples of storage width:

```text
1-byte UTF-8 scalar -> 1-byte Char payload
4-byte UTF-8 scalar -> 4-byte Char payload
```

`Char` is not a growable text buffer.

---

## String

`String` is:

- UTF-8
- interned
- immutable at language level
- immutable at VM level
- Unicode-scalar-indexed
- zero-copy sliced
- VM lifecycle managed

Interned strings are not immortal but may survive most of the VM/process lifetime.

Manual freeing of interned strings is possible through the standard library, not through default builtins.

If a source string dies, its borrowing slices die with it.

---

## CharBuffer

`CharBuffer` is a mutable growing sequence of immutable `Char` values.

Rules:

- each `Char` remains immutable
- the buffer is mutable
- may contain multiple Unicode scalars
- can be converted to `String`

Conversion rules:

```text
CharBuffer -> String     // allowed
String -> CharBuffer     // copy required, not zero-copy
```

---

# Composite / Product Types

Composite/product types combine multiple fields into one value.

TinyOne intends to support:

- structs
- records
- arrays
- vectors
- buffers
- maps
- dictionaries

---

## Structs

Structs are product types that may carry behavior, invariants, ownership
responsibilities, destructor behavior, or protocol/trait participation.

A plain struct may use structural compatibility. A resource-owning or
invariant-bearing struct must use nominal compatibility.

Example candidate form:

```tinyone
struct Point {
  x: i32
  y: i32
}
```

Rules:

- typed fields
- ownership rules apply per field
- destructors are deterministic where defined
- structural compatibility is allowed for plain data structs
- nominal compatibility is required for structs with destructors, private
  invariants, unsafe capabilities, explicit resource ownership, or external
  handles

Structural compatibility means two values are compatible when their field names,
field order where layout-sensitive, and field types match. The declared type name
does not need to match.

Nominal compatibility means the declared type identity participates in type
checking. Two structs with identical fields are not interchangeable unless an
explicit conversion exists.

Example nominal form:

```tinyone
nominal struct FileHandle {
  fd: i32
}
```

Practical rule:

- use `record` for plain structural data
- use plain `struct` for data that may have methods or protocol participation
- use `nominal struct` when accidental compatibility would be unsafe

---

## Records

Records are plain structural product types intended for structured data.

Rules:

- always structural
- no destructor
- no hidden invariant
- no unsafe capability
- no external resource ownership
- no nominal identity requirement
- suitable for data interchange and typed records

Distinction:

- `record`: plain data product type
- `struct`: product type that may participate in behavior, ownership, and
  protocol/trait rules
- `nominal struct`: product type whose declared identity matters

---

## Arrays

An array is:

- fixed size
- homogeneous
- stack allocated
- compile-time sized

Rules:

```text
Array<T, N>
```

where:

- `T` is the element type
- `N` is known at compile time

Arrays are distinct from buffers and vectors.

---

## Vec

A `Vec<T>` is:

- growable
- heap allocated
- homogeneous
- backed by a stack-resident control structure

The control structure stores:

- length
- capacity
- pointers

---

## Buffer

`Buffer<T>` is a low-level VM object around pointers.

Rules:

- generic
- typed at compile time
- verified at runtime
- enforced at runtime
- contiguous
- resizable
- owning
- may be stack allocated
- stores pointers to values
- uses tagged pointers and separation
- can hold raw data or typed values

Buffers are primarily used for:

- I/O
- printing
- file reads/writes
- text construction
- `CharBuffer`
- runtime data movement

Resize operations:

```tinyone
buf.expand(size)
buf.shrink(size)
```

`expand()` preserves existing pointers and appends additional owned pointer slots.

`shrink()` kills trailing pointers until the requested shrink size is achieved.

Bounds are checked on write/modify.

Read bounds behavior:

- out-of-bounds buffer reads are runtime errors
- reads never wrap
- reads never clamp
- reads never zero-fill missing bytes
- reads never expose undefined behavior
- unsafe syntax permits the read operation syntactically, but does not disable
  VM provenance, lifetime, or bounds checks

For byte buffers, multi-byte reads use little-endian unsigned interpretation
unless a typed view explicitly specifies another layout.

---

## Maps and Dictionaries

TinyOne intends to support maps and dictionaries.

Definitions:

- `Map<K, V>`: typed key/value associative collection
- `Dictionary`: dynamic recursive key/value data

`Map<K, V>` is homogeneous. Every key has type `K` and every value has type
`V`.

`Dictionary` is dynamic data. A dictionary entry is `K: V`, where `V` may be a
scalar dynamic value, collection value, or nested dictionary value:

```text
K: V
K: { K: V }
K: { K: { K: V } }
```

Map rules:

- keys must support stable equality
- the default map preserves insertion order for iteration
- duplicate-key insertion replaces the old value
- replaced keys/values are destroyed according to ownership rules
- hash-backed implementation details must not leak nondeterminism into language
  behavior
- keys and values are owned by the map unless borrowed view syntax is explicit
- mutation follows ordinary ownership and borrow rules

Dictionary rules:

- keys are dynamic key values constrained to hashable scalar data
- dynamic values carry runtime type tags
- dynamic values may be scalar values, arrays/vectors of dynamic values, maps, or
  nested dictionaries
- nested dictionaries are explicit values, not implicit type erasure of `Map`
- recursive dictionary expansion is bounds-checked and allocation-checked by the
  VM
- dictionary access returns a dynamic value that must be checked or pattern
  matched before use as a concrete static type
- dictionary use is allowed, but it is not the default replacement for typed
  records, structs, or maps

---

# Algebraic Types

Algebraic types describe types formed by alternatives or combinations.

TinyOne intends to support:

- enums
- tagged unions
- sum types
- product types

---

## Sum Types

Sum types are structural.

A sum type represents one of several possible variants.

Candidate form:

```tinyone
type Result<T, E> = Ok(T) | Err(E)
```

Rules:

- variant compatibility is structural
- exhaustive matching should be type checked
- ownership rules apply to the active variant
- inactive variants do not own live data

---

## Enums

Enums are named sets of variants.

Candidate form:

```tinyone
enum Color {
  Red
  Green
  Blue
}
```

Enums may be treated as structural sum types if their variants and payloads match.

---

## Tagged Unions

Tagged unions store:

- active tag
- active payload

Candidate form:

```tinyone
tagged union Value {
  Int(i64)
  Text(String)
  Bool(bool)
}
```

Rules:

- active tag determines valid payload
- tag checks are verifier-visible or VM-visible
- invalid tag/payload combinations are runtime errors

---

# Reference and Ownership Types

TinyOne has explicit reference and ownership-oriented types.

These include:

- raw pointers
- references
- `Box<T>`
- allocations

---

## Pointer

A pointer is a stack value that stores a raw memory address.

Rules:

- may be raw
- may be dangling
- may be null
- can point to heap or other runtime memory depending on provenance
- is unsafe or restricted when used for memory mutation

Pointer validity is determined by the VM.

The VM tracks allocation provenance and pointer lifecycle.

---

## Reference: `&T`

A reference is:

- stack allocated
- non-owning
- zero-copy
- an alias for an existing object
- safeguarded by VM lifetime checks
- non-null by default

References do not own the value they reference.

The VM cannot invalidate a referenced pointer while a valid reference exists.

Nullable reference behavior must be represented explicitly, for example as a sum
type containing `null` and `&T`. Raw `&T` is not nullable.

Candidate syntax:

```tinyone
let r: &T = &value
```

Rules:

- lifetime checked by VM
- ownership checked by type system
- borrowing slices use reference-like behavior

---

## Box<T>

`Box<T>` represents owned heap allocation.

Rules:

- owns exactly one `T`
- is non-null
- deterministic destruction
- may not be implicitly copied
- moves transfer ownership and invalidate the old binding
- copying requires an explicit clone operation and only works when `T` supports
  cloning
- may expose references through `&T`
- may expose unsafe raw pointers only through unsafe syntax
- raw pointer escape keeps VM provenance metadata tied to the allocation
- freeing or moving the box must respect outstanding borrow metadata

Candidate syntax:

```tinyone
let b: Box<i32> = Box::new(10)
```

`Box<T>` is the preferred high-level ownership type for single heap values.

---

## Alloc

`Alloc` represents a heap memory reservation.

It is lower-level than `Box<T>`.

Rules:

- may be unsafe
- requires provenance tracking
- may require explicit free
- may participate in arena/pool ownership

---

## VM Lifetime Metadata

Lifetime metadata is VM-sided. Source code does not need explicit lifetime
parameters for ordinary ownership and borrowing.

The VM tracks:

- allocation id
- heap address
- generation
- region or scope id
- owner slot or owner handle
- current owner state
- active shared borrow count
- active mutable borrow state
- raw pointer escape state
- allocation kind
- layout/type id
- destructor id, if any

Pointers and references carry enough metadata to validate:

- base allocation
- generation
- offset or field identity
- access width
- pointer/reference kind
- cast/type view
- borrow permission
- region/scope validity

The compiler may emit lifetime hints, but the VM remains the authority for
validity, use-after-free detection, stale pointer detection, and borrow
enforcement.

---

# Function Types

Function types describe callable values.

Candidate syntax:

```tinyone
fn(i32, i32) -> i32
```

Example:

```tinyone
let add: fn(i32, i32) -> i32
```

## Function Type Components

A function type includes:

- parameter types
- return type
- unsafe marker, if applicable
- ownership/lifetime constraints
- effect metadata where needed
- calling convention / ABI metadata

## Function ABI Representation

TinyOne uses a typed frame ABI for language-level calls.

The operand stack remains an execution detail. The callable ABI is larger than a
raw stack convention and includes enough metadata for type checking, ownership,
closures, and runtime validation.

A function ABI record contains:

- function identity
- parameter count
- parameter types
- return type
- unsafe marker
- effect flags
- capture/environment layout, if any
- local slot layout
- return slot or return-value convention
- caller return point
- borrow/lifetime metadata needed by the VM
- provenance snapshot metadata needed for recoverable runtime failures

Call behavior:

- arguments are evaluated left-to-right
- argument values are moved or copied according to their type rules
- callee parameters are bound into typed frame slots
- the callee receives its environment pointer when called through a closure
- exactly one value is returned unless the return type is an explicit unit/ZST
- external/native ABIs are separate from the internal TinyOne ABI

## Unsafe Functions

Unsafe functions require explicit unsafe call syntax.

Candidate form:

```tinyone
unsafe fn free(ptr: Ptr) -> bool
```

Call:

Current expression form:

```tinyone
let ok = unsafe free(ptr)
```

Planned block form:

```tinyone
unsafe {
  free(ptr)
}
```

## Closures

Closures are callable values represented as:

```text
Closure = { function_id, environment }
```

The environment stores captured values.

Capture semantics:

- closures use move capture by default
- moved captures transfer ownership into the closure environment
- non-copy captured values cannot be used through the old binding after capture
- copyable captured values may be copied into the environment
- reference captures require explicit reference syntax and VM lifetime metadata
- closures with captured values are heap-backed or arena-backed environment
  objects
- non-capturing closures may compile to plain function values
- closure calls use the same typed frame ABI as named functions, with an
  additional environment pointer

---

# Generics

Generics are monomorphized.

This means generic code produces concrete specialized forms for each used type instantiation.

Example:

```tinyone
fn identity<T>(x: T) -> T {
  x
}
```

Used as:

```tinyone
identity<i32>(1)
identity<String>("text")
```

may generate concrete forms equivalent to:

```text
identity_i32
identity_String
```

## Generic Constraints

Generics may be constrained by:

- interfaces
- traits
- protocols
- ownership capabilities
- lifetime requirements
- unsafe capabilities

Candidate form:

```tinyone
fn print_all<T: Display>(items: Vec<T>)
```

## Templates

Templates are compile-time generics with runtime usability.

They may be used for:

- generated code
- specialized structures
- compile-time shape construction
- runtime-accessible generic artifacts

Boundary:

- generics specialize functions and types by type parameters
- generics do not run arbitrary compile-time generation logic
- templates generate concrete declarations, layouts, or helper code before
  runtime execution
- templates may accept type parameters and compile-time constant parameters
- template output must become ordinary typed TinyOne constructs before bytecode
  verification
- runtime-accessible template artifacts are metadata or generated declarations,
  not a separate runtime type system

---

# Phantom Types

Phantom types are type parameters used for semantics only.

They do not necessarily exist at runtime.

They are used to encode compile-time meaning such as:

- ownership state
- lifetime category
- permission state
- unit type
- protocol role
- memory region
- unsafe capability

Example candidate form:

```tinyone
struct Ptr<T, Region> {
  addr: usize
  _region: Phantom<Region>
}
```

Here, `Region` may not exist at runtime, but it helps the type checker reject invalid pointer movement.

## Rules

- Phantom types carry semantic constraints.
- Phantom types do not require runtime storage.
- Phantom types may be erased after type checking.
- Phantom types may influence verifier metadata if needed.
- `Phantom<T>` is the standard marker form.
- A phantom field has no runtime payload unless the VM explicitly preserves
  verifier metadata for safety checks.

---

# Zero-Sized Types

Zero-sized types, or ZSTs, are types with no runtime data payload.

They may be used for:

- markers
- states
- capabilities
- compile-time proofs
- protocol roles
- ownership states
- unsafe permissions

Example:

```tinyone
struct Owned {}
struct Borrowed {}
struct Freed {}
```

These can be used as marker states:

```tinyone
struct Handle<State> {
  ptr: Ptr
  state: Phantom<State>
}
```

Possible states:

```text
Handle<Owned>
Handle<Borrowed>
Handle<Freed>
```

## Runtime Layout

A ZST has no ordinary runtime payload.

Rules:

- may occupy no storage
- may exist only in type metadata
- may be erased after verification
- may still affect code generation and verifier behavior
- empty struct syntax defines a ZST unless fields or runtime payload are added
- ZST fields may be optimized out of runtime layout

---

# Interfaces, Traits, and Protocols

## Interface

An interface is a strict method-signature contract.

It defines what methods a type must expose.

Example candidate:

```tinyone
interface Display {
  fn display(self) -> String
}
```

## Trait

A trait may provide:

- default method implementations
- retroactive modeling support
- reusable behavior declarations

Example candidate:

```tinyone
trait Debug {
  fn debug(self) -> String {
    "<debug>"
  }
}
```

## Protocol

A protocol focuses on:

- structural typing
- communication rules
- behavioral contracts

Protocols are suited for systems where shape and interaction matter more than nominal identity.

## Dispatch Model

Default dispatch is static.

Rules:

- interface satisfaction is structural
- protocol satisfaction is structural
- trait satisfaction is structural for required method signatures
- trait default methods are statically expanded or monomorphized where possible
- generic constraints use static dispatch after monomorphization
- method lookup is type checked before bytecode emission
- dynamic dispatch requires explicit `dyn` syntax or an equivalent explicit
  existential type

Dynamic dispatch representation:

```text
Dyn<Trait> = { value_pointer, dispatch_table, runtime_type_id }
```

Dynamic dispatch is explicit type erasure. The erased value must keep enough VM
metadata to preserve ownership, lifetime, and destructor behavior.

---

# Runtime and Bytecode Type Metadata

Typed bytecode variants are planned.

Bytecode carries type metadata even when some source-level types are erased.

The verifier becomes type-aware.

## Responsibilities

Compiler:

- static type checking
- generic monomorphization
- ownership checking
- unsafe visibility
- HIR/MIR/typed IR generation

Verifier:

- typed bytecode validation
- stack correctness
- opcode/type compatibility
- lifetime metadata validation where applicable
- pointer provenance metadata checks where possible

VM:

- allocation provenance
- lifetime checking
- pointer validity
- overflow snapshots
- bounds checks
- unsafe rollback
- destructor execution

JIT:

- preserves VM semantics
- respects type metadata
- maintains deterministic behavior
- may optimize unsafe code
- must preserve source/debug mappings when mutating bytecode

Native executable-memory details are an implementation concern, not a source
type-system feature. The type-system requirement is that any compiled tier runs
only verified typed bytecode, preserves VM provenance/lifetime/bounds behavior,
and can bail out to VM helpers when it cannot prove an operation locally.

---

# Type Erasure

Type erasure means the type is destroyed.

After erasure:

- full source type information is unavailable
- only inferable metadata may remain
- constant metadata may be retained if needed

The goal is to avoid unnecessary erasure.

Generics are monomorphized specifically to reduce the need for runtime type erasure.

---

# Unsafe Type Rules

Unsafe operations are type-system-visible.

Unsafe calls require explicit unsafe syntax.

Unsafe code may appear in safe files/functions/modules when explicitly scoped.

Current unsafe syntax is expression-scoped:

```tinyone
let value = unsafe ptr_load(ptr)
```

Block-scoped unsafe syntax is planned for grouped unsafe operations:

```tinyone
unsafe {
  ptr_store(ptr, value)
  ptr_load(ptr)
}
```

Rules:

- unsafe may fail
- unsafe failure is expected
- snapshots protect recoverable state
- unsafe memory operations may still violate guarantees if misused
- fallbacks are required where safety failures can occur

---

# Phase 2 Priorities

The type system should be implemented in this order:

This is a delivery sequence, not a scope cut. Items later in the list remain
part of the Phase 2 type system.

```text
1. Primitive scalar types
2. Boolean and overflow semantics
3. String / Char / CharBuffer validation
4. Array / Vec / Buffer separation
5. Struct and record compatibility rules
6. Reference and Box<T> ownership rules
7. Function types
8. Sum types / enums / tagged unions
9. Generics via monomorphization
10. Phantom types and ZSTs
11. Interfaces / traits / protocols
12. Type-aware verifier
13. Typed bytecode variants
14. JIT type metadata integration
```

---

# Hard Constraints

TinyOne type-system design must not depend on traditional garbage collection.

All type and memory rules must work with:

- ownership
- lifetimes
- VM provenance tracking
- deterministic destruction
- scoped arenas
- pools
- explicit free where allowed
- lifecycle/reference-count mechanisms only where justified

---

# Resolved Phase 2 Decisions

The following decisions close the prior ambiguity list:

1. `i32 + i64 -> i64`; mixed-width integer arithmetic promotes to the smallest
   built-in integer type that can represent both operand domains, then executes
   with checked runtime overflow.
2. Signed/unsigned movement is explicit and checked. Bit reinterpretation and
   sign-bit clearing are unsafe bit operations, not ordinary numeric casts.
3. Buffer reads are bounds-checked runtime operations. Out-of-bounds reads are
   runtime errors even inside unsafe syntax.
4. `record` is plain structural data. `struct` may participate in behavior,
   ownership, and protocols. `nominal struct` is used when declared identity is
   required for resource safety or invariants.
5. `Map<K, V>` is typed homogeneous associative data. `Dictionary` is dynamic
   recursive key/value data.
6. Closures are `{ function_id, environment }` callable values with move capture
   by default.
7. The function ABI is a typed frame ABI with function identity, parameter
   types, return type, local layout, environment layout, borrow metadata, and
   provenance snapshot metadata.
8. `Box<T>` is non-null, move-only, non-copy by default, and deterministically
   destroyed.
9. Lifetime metadata is VM-sided and tracks allocation identity, generation,
   region/scope, owner state, borrow state, pointer/reference kind, and layout
   identity.
10. `Phantom<T>` is the marker form for phantom data. Empty struct syntax defines
    zero-sized marker types.
11. Interface, trait, and protocol constraints use static structural dispatch by
    default. Dynamic dispatch requires explicit `dyn`/existential syntax.
12. Floating point follows IEEE 754, including NaN/unordered comparison,
    signed zero, infinities, round-to-nearest ties-to-even, and checked
    float-to-integer conversion.
13. Native executable-memory mechanics are not a source type-system feature.
    The type-system requirement is preservation of verified typed bytecode
    semantics, VM provenance checks, lifetime checks, bounds checks, and VM
    bailout behavior.
