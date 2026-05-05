# TinyOne

TinyOne is a tiny systems-language sketch implemented as a single Python file. It
includes a lexer, recursive-descent compiler, bytecode optimizer, verifier,
portable VM, and a generated-Python JIT backend.

The project is intentionally small and dependency-free. It is useful as a
readable example of how a minimal language runtime can move from source text to
verified bytecode and then execute through more than one backend.

## Features

- Single-file implementation in `main.py`
- Python standard library only
- Integer arithmetic with precedence and parentheses
- String literals
- Heap-backed arrays, structs, buffers, and pointer cells
- `let` bindings and `print` statements
- `set` mutation for array indexes and struct fields
- Top-level `fn` functions with parameters, calls, and `return`
- Top-level `struct` definitions
- Namespaced source-file `import` declarations with `export` visibility
- `while` loops with brace-delimited bodies
- Comparison expressions that produce `0` or `1`
- Stack-machine bytecode compiler
- Peephole constant folding before execution
- Static control-flow-aware bytecode stack-depth verification
- Lexical block scopes backed by zero-initialized stack-frame slots
- Explicit heap allocation/free for arrays, structs, strings, buffers, and cells
- Raw pointers for heap objects, array elements, struct fields, buffers, and
  pointer cells
- Unsafe-gated raw pointer dereference and raw address construction/arithmetic
- Small standard library plus `null`: `len`, `array`, `alloc`, `load`,
  `store`, `free`, `read`, `read_int`, `read_str`, `to_int`, `ptr`,
  `fieldptr`, `ptr_addr`, `ptr_at`, `ptr_add`, `ptr_load`, `ptr_store`,
  `ptr_type`, `is_null`, `ptr_eq`, `ptr_ne`, `ptr_base`, `ptr_offset`,
  `ptr_kind`, `ptr_field`, `buffer`, `read8`, `write8`, `read16`, `write16`,
  `read32`, `write32`, and `cast_ptr`
- Deterministic input queues through CLI flags or stdin
- JSON bytecode artifact emission and execution with module dependency metadata
- Two execution backends:
  - `jit`, the default, emits generated Python functions
  - `vm`, a portable bytecode interpreter
- Dependency-free tests and VM/JIT benchmark harness under `Tests/`

## Requirements

- Python 3.10 or newer
- No package installation step

## Quick Start

Create a TinyOne source file:

```tinyone
let x = 1 + 2 * 3
let y = (x - 4) / 2
while y < 5 {
  let y = y + 1
}
print x
print y
```

Run it with the default JIT backend:

```sh
python3 main.py example.tinyone
```

Expected output:

```text
7
5
```

Run the same program through the VM backend:

```sh
python3 main.py --mode vm example.tinyone
```

Enable compiler/runtime debug logging:

```sh
python3 main.py --verbose example.tinyone
```

## Command Line

```text
usage: main.py [-h] [--mode {jit,vm}] [--check]
               [--emit-bytecode PATH] [--run-bytecode PATH]
               [--input INPUT] [--stdin] [--verbose]
               [path]
```

Arguments:

- `path`: TinyOne source file to execute
- `--mode jit`: compile bytecode into a generated Python function
- `--mode vm`: execute bytecode with the portable VM
- `--check`: compile and verify without running
- `--emit-bytecode PATH`: write a JSON bytecode artifact after compilation
- `--run-bytecode PATH`: execute a previously emitted JSON artifact
- `--input VALUE`: append one deterministic input item for `read*` builtins
- `--stdin`: append stdin lines to the deterministic input queue
- `--verbose`: enable debug logging

TinyOne exits with status `0` on success and status `1` for file, compile, or
runtime errors.

## Language

TinyOne has integers, strings, heap-backed aggregates, top-level statements,
top-level declarations, brace-delimited loop/function bodies, and expressions.
Whitespace separates tokens, but newlines are not significant. `#` starts a
line comment.

### Statements

```tinyone
let name = expression
print expression
set name[index] = expression
set name.field = expression
while expression { statements }
return expression
```

`let` defines or updates a variable slot. Variables must be defined before they
are read, and slots start at `0` inside the current stack frame. Block-local
names are hidden after the block, while assignments to outer visible names keep
using the outer slot. `while` repeats while its expression is non-zero. `return`
is only valid inside a function.

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
    let acc = acc * n
    let n = n - 1
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

let word = "tiny"
print len(word)
print word[0]
```

Pointer cells are created through the standard library. They model explicit
allocation, load/store, and free.

```tinyone
let ptr = alloc(41)
print load(ptr)
print store(ptr, load(ptr) + 1)
let ignored = free(ptr)
```

Using a freed pointer is a runtime error.

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

TinyOne reserves these builtin names:

```text
len(value)
array(count, fill)
buffer(size)
alloc(value)
load(ptr)
store(ptr, value)
free(ptr)
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

## Memory Model

Each function call allocates a fresh stack frame with fixed slots for that
compiled function. Heap objects are separate from stack frames and are reached
through `HeapRef` handles:

- strings: immutable text objects
- arrays: mutable indexed value vectors
- structs: mutable named-field records
- buffers: mutable zero-initialized byte vectors for raw memory operations
- cells: pointer-like single-value allocations used by `alloc/load/store/free`

Heap references and raw pointers carry a runtime generation tag, so stale
references fail after `free` even when the numeric heap address is reused. The
heap detects invalid pointers, stale derived pointers, and use-after-free at
runtime.

## Bytecode Artifacts

TinyOne can emit and run a JSON artifact containing bytecode, function chunks,
string literals, struct definitions, field metadata, and module dependency
metadata.

```sh
python3 main.py --check --emit-bytecode program.tobc.json program.to
python3 main.py --run-bytecode program.tobc.json
```

Artifacts are verified again before execution.

## JIT Backend

The JIT backend does not generate machine code. It emits and compiles a Python
function specialized for the verified TinyOne bytecode.

For branch-free main programs, the JIT maps virtual stack positions to Python
locals named `_s0`, `_s1`, and so on. Programs with functions or loops still go
through generated Python code, but use a small generated dispatch loop so branch
targets and function calls execute with the same semantics as the VM.

## VM Backend

The VM backend interprets the same optimized and verified bytecode. It is
simpler, easier to debug, and useful for checking behavior against the JIT
backend.

## Tests and Benchmarks

Run the correctness suite:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s Tests -p 'test_*.py'
```

The tests cover VM/JIT parity for straight-line code, loop dispatch, function
call/return dispatch, nested control-flow transfers, heap arrays, structs,
strings, buffers, pointer cells, raw pointers, null checks, pointer metadata,
stale pointer rejection, deterministic input, namespaced imports, export
visibility, manifest resolution, artifact round trips, line/column diagnostics,
lexical block scope, runtime errors, JIT cache reuse, and verifier rejection for
malformed bytecode.

Run the benchmark suite:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 Tests/bench_vm_jit.py
```

For a fast smoke timing run:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 Tests/bench_vm_jit.py --quick --repeats 1
```

The benchmark harness measures memory allocation/load/store/reset/snapshot,
lexer/compiler/optimizer/verifier stages, program fingerprinting, cold and hot
JIT compilation, VM execution, JIT execution, full compile-and-run APIs,
function calls, heap/struct workloads, input-backed standard-library calls, and
control-transfer opcodes. TinyOne has no OS interrupt subsystem; the
`control_interrupts` benchmarks stress the runtime's interrupt-like bytecode
transfers: `JUMP`, `JUMP_IF_ZERO`, `CALL`, `RETURN`, and `HALT`.

## Programmatic Use

`main.py` can also be imported directly:

```python
from io import StringIO
from main import compile_source, run_source

program = compile_source("fn add(a, b) { return a + b } print add(40, 2)")

stdout = StringIO()
memory = run_source(
    "fn add(a, b) { return a + b } print add(40, 2)",
    mode="jit",
    stdout=stdout,
)

assert stdout.getvalue() == "42\n"
assert memory.snapshot() == ()
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
- No garbage collector; heap ownership is manual for pointer cells and raw
  pointers

## Repository Layout

```text
.
├── Tests/
│   ├── README.md
│   ├── bench_vm_jit.py
│   └── test_vm_jit.py
├── README.md
└── main.py
```

`main.py` contains the complete implementation and CLI entrypoint. `Tests/`
contains the stdlib-only correctness tests and benchmark harness.
