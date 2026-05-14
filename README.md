# TinyOne

TinyOne is a tiny systems-language sketch implemented in Rust. It includes a
lexer, recursive-descent compiler, bytecode optimizer, verifier, portable VM,
heap/runtime model, bytecode artifact support, and CLI.

`Rust/` is the source of truth and test plane for language development. New
syntax, bytecode, VM, JIT, and runtime behavior is built in Rust first. The
Python implementation under `Python/` is maintained as the validation plane and
portable runtime: it follows Rust semantics, catches portability drift, and
keeps a compile-less implementation available.

Future maintained implementations are planned for Go and C++.

## Compatibility Notice

Rust owns TinyOne behavior. Python validates that behavior and keeps a portable
runtime aligned with the Rust implementation. Minor implementation differences
and edge-case inconsistencies may still exist between runtimes.

The VM architecture is specifically designed to reduce behavioral divergence
across implementations, but exact parity is not yet guaranteed in all cases.

If you encounter compatibility issues, runtime inconsistencies, or unexpected
edge-case behavior between implementations, please report them on [GitHub](https://github.com/ConnerAdamsMaine/TinyOne):

## Features

- Rust implementation in `Rust/src/lib.rs` and `Rust/src/main.rs`
- Integer arithmetic with precedence and parentheses
- String literals
- Heap-backed arrays, structs, buffers, and pointer cells
- `let` bindings and `print` statements
- `set` mutation for array indexes and struct fields
- Top-level `fn` functions with parameters, calls, and `return`
- Top-level `struct` definitions
- Namespaced source-file `import` declarations with `export` visibility
- `if`/`else` conditionals
- `while` loops with brace-delimited bodies
- `break` and `continue` loop control
- Comparison expressions that produce `0` or `1`
- Stack-machine bytecode compiler
- Peephole constant folding before execution
- Static control-flow-aware bytecode stack-depth verification
- Lexical block scopes backed by zero-initialized stack-frame slots
- Explicit heap allocation and unsafe deallocation for arrays, structs, strings,
  buffers, and cells
- Raw pointers for heap objects, array elements, struct fields, buffers, and
  pointer cells
- Unsafe-gated raw pointer dereference and raw address construction/arithmetic
- Small standard library plus `null`: `len`, `array`, `alloc`, `load`,
  `store`, `unsafe free`, `read`, `read_int`, `read_str`, `to_int`, `ptr`,
  `fieldptr`, `ptr_addr`, `ptr_at`, `ptr_add`, `ptr_load`, `ptr_store`,
  `ptr_type`, `is_null`, `ptr_eq`, `ptr_ne`, `ptr_base`, `ptr_offset`,
  `ptr_kind`, `ptr_field`, `buffer`, `read8`, `write8`, `read16`, `write16`,
  `read32`, `write32`, `cast_ptr`, `push`, and `pop`
- Deterministic input queues through CLI flags or stdin
- JSON bytecode artifact emission and execution with module dependency metadata
- `jit` and `vm` CLI modes. The Rust `jit` mode now compiles verified TinyOne
  bytecode into a lower-level adaptive bytecode tier, caches it by program
  fingerprint, and quickens hot loop paths in-place.
- Rust benchmark runner with correctness preflight and timing table
- Rust unit tests under `Rust/src/lib.rs` and integration tests under
  `Rust/tests/`
- Python validation tests and portable-runtime benchmark harness under
  `Python/Tests/`

## Project Goals

TinyOne exists for several reasons:

1. To demonstrate that programming languages and runtime systems do not need to
   be optimistically overengineered to remain useful, understandable, or
   extensible.

2. To serve as a practical educational project for studying low-level language
   implementation, VM architecture, memory models, compiler pipelines, runtime
   verification, and systems design.

3. To act as an architectural foundation and experimentation platform for a
   future AI-focused language and runtime ecosystem designed around portable,
   customizable compute kernels across AMD and NVIDIA hardware.

The future AI-oriented language project is not part of TinyOne itself yet.
Additional details and announcements will be published through the TinyOne
GitHub repository in the future.

## Requirements

- Rust toolchain with Cargo
or
- Python 3.10 or newer

## Quick Start

Create a TinyOne source file:

```tinyone
let x = 1 + 2 * 3
let y = (x - 4) / 2
while y < 5 {
  y = y + 1
}
print x
print y
```

Run it with the default adaptive `jit` mode:

```sh
cargo run --manifest-path Rust/Cargo.toml --bin tinyone -- example.tinyone
```

Expected output:

```text
7
5
```

Run the same program through the VM backend:

```sh
cargo run --manifest-path Rust/Cargo.toml --bin tinyone -- --mode vm example.tinyone
```

Print a compiler/runtime summary to stderr:

```sh
cargo run --manifest-path Rust/Cargo.toml --bin tinyone -- --verbose example.tinyone
```

## Command Line

```text
usage: tinyone [--mode {jit,vm}] [--check]
               [--emit-bytecode PATH] [--emit-jit PATH]
               [--run-bytecode PATH]
               [--input VALUE] [--stdin] [--verbose]
               [path]
```

Arguments:

- `path`: TinyOne source file to execute
- `--mode jit`: compile verified bytecode to the adaptive JIT bytecode tier and
  execute it
- `--mode vm`: execute bytecode with the portable VM
- `--check`: compile and verify without running
- `--emit-bytecode PATH`: write a JSON bytecode artifact after compilation
- `--emit-jit PATH`: write the lowered adaptive-JIT listing after compilation
- `--run-bytecode PATH`: execute a previously emitted JSON artifact
- `--input VALUE`: append one deterministic input item for `read*` builtins
- `--stdin`: append stdin lines to the deterministic input queue
- `--verbose`: print a compiler/runtime summary to stderr

TinyOne exits with status `0` on success and status `1` for file, compile, or
runtime errors.

## Release/Audit Hash Tool

`Tools/hash.py` hashes individual files or deterministic directory trees for
release manifests, audit checkpoints, and source-tree integrity checks. It is a
stdlib-only developer utility; it is not part of the TinyOne compiler, VM, JIT,
or language runtime.

```sh
./Tools/hash.py README.md
./Tools/hash.py --tree . --exclude manifest.json --format json > manifest.json
./Tools/hash.py --expected DIGEST README.md
./Tools/hash.py --check manifest.json
```

File mode prints stable checksum lines for one or more files. Tree mode walks a
directory in deterministic path order, frames each path and file digest into one
tree digest, and can optionally emit per-file entries with `--list-files`.
Manifest verification re-hashes every emitted entry and fails when a digest or
recorded tree file count changes.

Tree mode excludes .gitignore by default. Add repeatable `--exclude PATTERN` entries
for local artifacts or use `--no-default-excludes` when a fully explicit tree is
needed. `--include SUFFIX` can still narrow tree hashing to source-like files.
When writing a manifest inside the hashed tree, exclude the manifest path.

Verification exits with status `0` when every entry matches, status `1` when a
digest or tree file count mismatches, and status `2` for usage, manifest, file,
or hashing errors.

`Tools/` is reserved for future repo-maintenance utilities: release manifest
generation, compatibility/audit helpers, benchmark result summarizers,
cross-implementation parity scripts, and source-tree integrity checks. Tools in
this directory should remain optional, deterministic where practical, and
separate from the runtime semantics of the Rust and Python implementations.

## Language

TinyOne has integers, strings, heap-backed aggregates, top-level statements,
top-level declarations, brace-delimited loop/function bodies, and expressions.
Whitespace separates tokens, but newlines are not significant. `#` starts a
line comment.

### Statements

```tinyone
let name = expression
name = expression
print expression
set name[index] = expression
set name.field = expression
if expression { statements }
if expression { statements } else { statements }
while expression { statements }
break
continue
return expression
```

`let` declares a variable in the current block. Plain assignment updates an
existing visible variable slot. Variables must be defined before they are read,
and slots start at `0` inside the current stack frame. Block-local names are
hidden after the block. `if` runs its first block when the expression is
non-zero; the optional `else` block runs otherwise. `while` repeats while its
expression is non-zero. `break` exits the innermost loop, `continue` jumps back
to that loop's condition, and `return` is only valid inside a function.

### Imports

Imports are source-file package boundaries. They must appear before declarations
and executable statements. Imported files may contain only `import`, `struct`,
`fn`, and `export` declarations, so importing a module does not run hidden
top-level code.

```tinyone
import "math.to" as math
let result = math.add(40, 2)
```

Import paths are resolved relative to the importing file. An imported module's
public API is only the declarations marked with `export`; private helpers remain
visible inside that module but are not visible to importers.

```tinyone
# math.to
fn normalize(value) {
  return value
}

export fn add(left, right) {
  return normalize(left) + normalize(right)
}
```

If the `as name` alias is omitted, TinyOne uses the imported filename stem as
the namespace. Imports may also resolve through a `tinyone.json` package
manifest in the importing file's directory or an ancestor:

```json
{
  "package": "demo",
  "modules": {
    "math": "lib/math.to"
  }
}
```

With that manifest, `import "math" as m` resolves `lib/math.to`.

### Structs

Structs are top-level declarations with named fields. Constructors use the
struct name and positional field values.

```tinyone
struct Point { x, y }

let point = Point(3, 4)
print point.x
set point.y = point.y + 1
print point
```

### Functions

Functions are declared at top level and return one value. Function
parameters are local slots initialized from the call arguments. Function-local
variables and parameters do not read or write top-level variables directly; pass
values as arguments and return results.

```tinyone
fn fact(n) {
  let acc = 1
  while n > 1 {
    acc = acc * n
    n = n - 1
  }
  return acc
}

print fact(5)
```

Function calls are expressions:

```tinyone
fn add(a, b) {
  return a + b
}

let answer = add(40, 2)
print answer
```

Functions must be defined before they are called. A function may call itself
from its own body because its name is reserved before the body is compiled.

### Arrays, Strings, And Pointers

Array and string values live on the heap. Variables hold handles to those heap
objects, not inline copies.

```tinyone
let values = [10, 20, 30]
set values[1] = 99
print values[1]
print push(values, 40)
print pop(values)

let word = "tiny"
print len(word)
print word[0]
```

`push(array, value)` mutates an array and returns its new length. `pop(array)`
removes and returns the last value, and reports a runtime error for an empty
array.

Pointer cells are created through the standard library. They model explicit
allocation, load/store, and unsafe free.

```tinyone
let ptr = alloc(41)
print load(ptr)
print store(ptr, load(ptr) + 1)
let ignored = unsafe free(ptr)
```

Manual deallocation requires `unsafe`. Using a freed pointer is a runtime error.

Raw pointers are a separate runtime value. They can point at heap objects, array
elements, struct fields, buffers, or pointer cells. `null` is the null raw
pointer. Use `is_null`, `ptr_eq`, and `ptr_ne` for pointer control-flow checks;
ordinary `==` and `!=` remain integer comparisons. Raw pointer dereference, raw
address construction, buffer loads/stores, and raw pointer arithmetic require
the `unsafe` prefix.

```tinyone
struct Pair { left, right }

let values = [10, 20, 30]
let second = ptr(values, 1)
print unsafe ptr_load(second)
print unsafe ptr_store(unsafe ptr_add(second, 1), 77)
print values[2]

let pair = Pair(4, 5)
let field = fieldptr(pair, "right")
print unsafe ptr_load(field)
print unsafe ptr_store(field, 99)
print pair.right

let cell = alloc(12)
let raw = ptr(cell)
print unsafe ptr_load(raw)
print unsafe ptr_store(raw, 13)
print load(cell)

let mem = buffer(16)
let p = ptr(mem, 0)
print unsafe read8(p)
print unsafe write8(unsafe ptr_add(p, 1), 255)
print unsafe write16(unsafe ptr_add(p, 2), 4660)
print unsafe read16(unsafe ptr_add(p, 2))
```

`ptr(value)` creates an object pointer for a heap value. `ptr(array, index)`
creates an array-element pointer, and `ptr(buffer, offset)` creates a byte
pointer into a buffer. `fieldptr(struct, "field")` creates a struct-field
pointer. `ptr_addr(value)` exposes a heap address. `unsafe ptr_at(address)`
turns an integer address back into an object pointer, and `unsafe
ptr_add(pointer, offset)` performs pointer arithmetic on array and buffer
pointers. Object and field pointers are not byte-addressable; use buffers for
raw memory.

`ptr_base(ptr)`, `ptr_offset(ptr)`, `ptr_kind(ptr)`, and `ptr_field(ptr)` expose
pointer metadata for debugging and tests. `ptr_type(ptr)` returns the explicit
cast type when present, otherwise the pointer kind. `cast_ptr(ptr, "i32")`
records a small pointer cast tag without introducing static pointer types.

Buffers are zero-initialized byte arrays. `read8`, `read16`, `read32`, `write8`,
`write16`, and `write32` use fixed little-endian unsigned integer semantics.
Writes return the stored value. Out-of-bounds raw memory access is a runtime
error.

Pointers to array elements and struct fields remain valid across mutation of
that same array or struct. If the base heap object is freed, every derived
pointer to that base fails, even if the address is later reused by another heap
allocation.

### Standard Library

TinyOne reserves these builtin names. Phase-1 (core) builtins occupy
slots 0..34 in the canonical builtin table; Phase-2 stdlib bridge builtins
follow them and are also bytecode-stable.

Phase-1 (core):

```text
len(value)
array(count, fill)
buffer(size)
alloc(value)
load(ptr)
store(ptr, value)
unsafe free(ptr)
read()
read_int()
read_str()
to_int(value)
ptr(value)
ptr(array, index)
fieldptr(struct, field_name)
ptr_addr(value)
unsafe ptr_at(address)
unsafe ptr_add(ptr, offset)
unsafe ptr_load(ptr)
unsafe ptr_store(ptr, value)
ptr_type(ptr)
is_null(ptr)
ptr_eq(left, right)
ptr_ne(left, right)
ptr_base(ptr)
ptr_offset(ptr)
ptr_kind(ptr)
ptr_field(ptr)
unsafe read8(ptr)
unsafe write8(ptr, value)
unsafe read16(ptr)
unsafe write16(ptr, value)
unsafe read32(ptr)
unsafe write32(ptr, value)
cast_ptr(ptr, type_name)
push(array, value)
pop(array)
```

Phase-2 stdlib bridge builtins (the higher-level modules under `stdlib/`
wrap these; you can also call them directly):

```text
# Dynamic arrays / hash maps
vec_new(), vec_clear(v)
map_new(), map_set(m, k, v), map_get(m, k), map_has(m, k), map_del(m, k)
map_len(m), map_keys(m), map_values(m)

# I/O abstractions (deterministic stdin/stdout/stderr buffers)
io_stdout(), io_stderr(), io_stdin()
io_write(fd, text), io_writeln(fd, text), io_read_line()
io_flush(fd), io_capture_stdout(), io_capture_stderr()

# String / Unicode (UTF-8)
str_byte_len(s), str_char_len(s)
str_byte_at(s, byte_index), str_char_at(s, char_index)
str_slice(s, start_char, end_char), str_concat(a, b)
str_is_utf8(value), str_from_buffer(buf)

# Threading & sync (single-threaded semantic shells; misuse is a runtime error)
mutex_new(), mutex_lock(m), mutex_unlock(m)
atomic_new(init), atomic_load(a), atomic_store(a, v), atomic_add(a, delta)

# Result / Option (heap-struct encoding; tag 1 = Ok/Some, tag 0 = Err/None)
result_ok(v), result_err(v)
result_is_ok(r), result_is_err(r), result_unwrap(r), result_unwrap_err(r)
option_some(v), option_none()
option_is_some(o), option_is_none(o), option_unwrap(o)

# System introspection (deterministic, host args/env injected by runtime)
sys_argc(), sys_argv(index)
sys_env_has(name), sys_env_get(name)

# Paths & FS (FS ops are unsafe per phase_2.md "could hurt the system")
path_join(left, right), path_basename(p), path_dirname(p)
unsafe fs_read(path), unsafe fs_write(path, buffer)
fs_exists(path), unsafe fs_list_dir(path)

# Math & logic
math_const(name), math_abs(v), math_min(a, b), math_max(a, b)
logic_and(a, b), logic_or(a, b), logic_not(v), logic_xor(a, b)

# Typing system (typing_system.md)
type_of(value), type_id(name)
smallest_fit(value), promote(lhs, rhs), check_int_range(value, type_name)
typed_add(lhs, rhs, type_name), typed_sub(lhs, rhs, type_name)
typed_mul(lhs, rhs, type_name), typed_div(lhs, rhs, type_name)
typed_neg(value, type_name)
assert(condition)
assert(condition, message)
```

Stdlib modules under `stdlib/` (loadable via the existing `import "name" as
alias` namespacing with a `tinyone.json` package manifest) wrap these:

```text
vec, map, io, string, sync, result, option, sys, path, fs, math, logic, typing
```

`read()` consumes one deterministic input item and returns an integer when the
input text is numeric, otherwise a heap string. `read_int()` requires numeric
input, and `read_str()` always returns a heap string.

### Expressions

Supported expression forms:

```tinyone
123
null
"text"
name
-expression
unsafe expression
(expression)
name(expression, ...)
namespace.name(expression, ...)
name[index]
name.field
[expression, ...]
left + right
left - right
left * right
left / right
left < right
left <= right
left > right
left >= right
left == right
left != right
```

Operator precedence is:

1. Parentheses and literals
2. Function calls, qualified module calls, constructors, array literals,
   variable reads, unsafe expressions, and postfix index/field access
3. Unary minus
4. Multiplication and integer division
5. Addition and subtraction
6. Comparisons and equality

Division uses Python-style integer floor division through `//`. Division by zero
is reported as a TinyOne runtime error. Comparisons evaluate to integer `1` for
true and `0` for false, which makes them usable as bounded loop conditions.
Arithmetic and comparisons require integer operands.

### Identifiers

Identifiers may contain letters, digits, and underscores. The first character
must be a letter or underscore.

## Runtime Pipeline

TinyOne executes source code through this pipeline:

```text
source -> lexer -> compiler -> bytecode -> optimizer -> verifier -> VM/JIT
```

The compiler emits stack-machine bytecode. Function bodies are stored as
separate bytecode chunks, imported modules are compiled as namespaces with
export tables, and `while` emits branch opcodes. The peephole
optimizer folds constant arithmetic and comparison patterns in branch-free
chunks, such as:

```text
PUSH_INT 2, PUSH_INT 3, MUL -> PUSH_INT 6
```

The verifier checks stack depth across reachable control-flow paths before
execution. Invalid bytecode, mismatched function or builtin arity, invalid
branch targets, invalid string/field/struct indexes, and stack imbalance fail
before either backend runs.

## Memory Safety And Ownership Model

Each function call allocates a fresh stack frame with fixed slots for that
compiled function. Heap objects are separate from stack frames and are reached
through `HeapRef` handles:

- strings: immutable text objects
- arrays: mutable indexed value vectors
- structs: mutable named-field records
- buffers: mutable zero-initialized byte vectors for raw memory operations
- cells: pointer-like single-value allocations used by `alloc`, `load`,
  `store`, and `unsafe free`

Heap references and raw pointers carry a runtime generation tag, so stale
references fail after `unsafe free` even when the numeric heap address is
reused. The heap detects invalid pointers, stale derived pointers, and
use-after-free at runtime.

TinyOne does not use garbage collection or compile-time borrow checking. The
Rust runtime owns the heap for the whole run. TinyOne variables, array elements,
struct fields, cells, and raw pointers hold values or handles; copying one of
those handles aliases the same heap object and does not clone, move, or transfer
ownership of that object.

| Runtime value | Stored where | Ownership behavior | Safety checks |
| --- | --- | --- | --- |
| `Int` | Stack slot or heap payload | Copied by value | Arithmetic overflow and divide-by-zero are runtime errors |
| `HeapRef` | Stack slot, array, struct field, or cell | Aliases one heap object by address and generation | Invalid, stale, and freed references fail before access |
| `RawPointer` | Stack slot, array, struct field, or cell | Derived alias into an object, array element, struct field, buffer byte range, or cell | Null, kind, generation, bounds, and stale-base checks run before pointer use |

The ownership rules are deliberately explicit:

- The runtime owns every heap allocation until `unsafe free(value)` releases
  that exact object or runtime shutdown drains all remaining live objects.
- `unsafe free(value)` is shallow. If a freed array, struct, or cell contains
  handles to other heap objects, those referenced objects stay alive until they
  are separately freed or the run shuts down.
- Handle and pointer aliases remain valid across ordinary mutation of the same
  live object. They become invalid once the base object is freed, even if the
  numeric heap address is later reused.
- `unsafe` is a source-level gate for operations that can violate TinyOne-level
  lifetime or address rules: manual free, raw address reconstruction, pointer
  arithmetic, raw pointer load/store, and buffer reads/writes. These operations
  are still checked by the Rust runtime and report TinyOne runtime errors
  instead of exposing host memory directly.
- Stack frames are fixed per compiled function call. Function locals and
  parameters live in that frame, start as `0`, and are discarded when the call
  returns.

Host-memory hazards are bounded before allocation:

| Resource | Limit |
| --- | --- |
| Dynamic array length | 65,536 elements |
| Single buffer allocation | 1 MiB |
| Total live heap payload | 4 MiB |
| Live heap object slots | 1,000,000 objects |
| Nested TinyOne calls | 16 calls |

Exceeding any of these limits is a TinyOne runtime error rather than unbounded
host allocation or host stack growth. At shutdown the runtime drains live heap
objects and reports the before/after heap state through the report APIs.

## Bytecode Artifacts

TinyOne can emit and run a JSON artifact containing bytecode, function chunks,
string literals, struct definitions, field metadata, and module dependency
metadata. Module metadata records stable module identities and original import
strings; it does not embed canonical source paths from the build machine.

```sh
cargo run --manifest-path Rust/Cargo.toml --bin tinyone -- --check --emit-bytecode program.tobc.json program.to
cargo run --manifest-path Rust/Cargo.toml --bin tinyone -- --run-bytecode program.tobc.json
```

Artifacts are verified again before execution.

## JIT Backend

The Rust JIT backend is an adaptive bytecode tier, not a native machine-code
JIT. It compiles verified TinyOne bytecode into a lowered internal bytecode
with decoded operands, separate compiled chunks for functions, simple
assignment superinstructions such as `store.i` and `slot.add.i`, and a
fingerprint-keyed cache.

During execution, the JIT records backward branches. Once a loop back-edge is
hot, the compiled chunk is quickened in-place: arithmetic, comparison, and jump
operations inside that hot range are rewritten to faster specialized forms such
as `add.int`, `cmp.int.lt`, and `jmp.hot`. `--emit-jit PATH` writes the lowered
listing before execution so the compiled form can be inspected.

## VM Backend

The VM backend interprets the same optimized and verified bytecode. It is
simpler, easier to debug, and useful for checking behavior against the adaptive
JIT backend.

## Tests and Benchmarks

### General Correctness Check

Use this as the normal repo-wide sanity pass before comparing runtimes,
publishing hashes, or changing language behavior:

```sh
cargo fmt --manifest-path Rust/Cargo.toml --all --check
cargo test --manifest-path Rust/Cargo.toml
cargo clippy --manifest-path Rust/Cargo.toml --all-targets -- -D warnings
python3 -m unittest discover -s Python/Tests -p 'test_*.py'
python3 -m py_compile Python/main.py Tools/hash.py Python/Tests/test_vm_jit.py Python/Tests/bench_vm_jit.py
./Tools/hash.py --tree . --include .py --include .rs --include .toml --include .md --format json
```

The Rust check covers the source-of-truth implementation: compiler, verifier,
heap, VM, adaptive JIT, artifact round trips, CLI-facing APIs,
benchmark-surface coverage, and VM/JIT runtime parity. The Python check
validates that the portable runtime still follows Rust semantics across the
same broad semantic categories. The `py_compile` pass catches Python syntax and
import-time parse errors in the portable runtime, tests, benchmark harness, and
tool scripts. The tree hash command is not a semantic test; it is an integrity
checkpoint for the source-like files that should change intentionally.

Run the Rust correctness suite:

```sh
cargo test --manifest-path Rust/Cargo.toml
```

Run the feature-gated Rust language fixture suite and testing hooks:

```sh
cargo test --manifest-path Rust/Cargo.toml --features testing-hooks
```

This command prints an explicit per-fixture report for every `.to` file in the
language suite, grouped by passing programs, module/import programs,
compile-fail programs, and runtime-fail programs. It covers both the newer
`Rust/tests/Language/` fixtures and the legacy compatibility fixtures under
`Rust/tests/Programs/`.

The Rust tests cover VM/adaptive-JIT parity for straight-line code, loops,
conditionals, function calls, nested control flow, runtime errors, memory slot
behavior, heap arrays, dynamic array storage, structs, strings, buffers, pointer
cells, raw pointers, null checks, pointer metadata, stale pointer rejection,
deterministic input, namespaced imports, export visibility, package manifest
resolution, artifact round trips, diagnostics, lexical scopes, hot-loop
quickening, JIT listing emission, cache reuse, and verifier failures. The
`testing-hooks` feature adds non-default external and internal Rust inspection
hooks plus `.to` language fixtures under `Rust/tests/Language/` and
`Rust/tests/Programs/`; it is for testing only, not the production API
contract. The Python validation suite remains under `Python/Tests/`.

Run the Python validation correctness suite:

```sh
python3 -m unittest discover -s Python/Tests -p 'test_*.py'
```

Run the Rust benchmark suite:

```sh
cargo run --release --manifest-path Rust/Cargo.toml --bin tinyone-bench
```

For a fast smoke timing run:

```sh
cargo run --release --manifest-path Rust/Cargo.toml --bin tinyone-bench -- --quick --repeats 1
```

The Rust benchmark runner prints correctness checks first, then timing rows for
memory allocation/load/store/reset/snapshot, lexer/compiler/optimizer/verifier
stages, program fingerprinting, artifact round trips, adaptive-JIT codegen and
cache hits, VM execution, JIT execution, full compile-and-run APIs, function
calls, heap/struct workloads, input-backed standard-library calls, and
control-transfer opcodes.

## Programmatic Use

The Rust crate exposes `compile_source`, `compile_file`, `run_source`,
`run_program`, `run_source_report`, `run_program_report`, `load_artifact`, and
`write_artifact` from `tinyone`.

```rust
let mut stdout = Vec::new();
tinyone::run_source(
    "fn add(a, b) { return a + b } print add(40, 2)",
    "jit",
    &mut stdout,
    Vec::new(),
)?;
assert_eq!(String::from_utf8(stdout).unwrap(), "42\n");
```

## Current Limitations

- No nested functions or closures
- No direct global-variable access from functions
- No user-defined methods or traits
- No static type checker yet
- No multi-statement `unsafe { ... }` block syntax yet; use single-expression
  `unsafe`
- No native object-file linker; imported source modules are separately compiled
  and linked into one verified bytecode artifact
- No garbage collector; explicit deallocation is available through `unsafe
  free(...)`

## Repository Layout

```text
.
├── Tools/
│   └── hash.py
├── Python/
│   ├── main.py
│   └── Tests/
│       ├── README.md
│       ├── bench_vm_jit.py
│       └── test_vm_jit.py
├── Rust/
│   ├── Cargo.toml
│   ├── src/
│   │   ├── bin/
│   │   │   └── tinyone-bench.rs
│   │   ├── lib.rs
│   │   └── main.rs
│   └── tests/
│       └── runtime_parity.rs
├── README.md
```

`Rust/src/lib.rs` contains the compiler, verifier, bytecode runtime, heap, and
public API. `Rust/src/main.rs` contains the CLI entrypoint. `Python/` keeps the
portable runtime and validation tests aligned with Rust. `Tools/` contains
optional repo-maintenance utilities that support release, audit, and parity
work without becoming part of the language runtime.
