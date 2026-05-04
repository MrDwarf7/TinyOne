# TinyOne

TinyOne is a tiny integer language implemented as a single Python file. It
includes a lexer, recursive-descent compiler, bytecode optimizer, verifier,
portable VM, and a generated-Python JIT backend.

The project is intentionally small and dependency-free. It is useful as a
readable example of how a minimal language runtime can move from source text to
verified bytecode and then execute through more than one backend.

## Features

- Single-file implementation in `main.py`
- Python standard library only
- Integer arithmetic with precedence and parentheses
- `let` bindings and `print` statements
- Top-level `fn` functions with parameters, calls, and `return`
- `while` loops with brace-delimited bodies
- Comparison expressions that produce `0` or `1`
- Stack-machine bytecode compiler
- Peephole constant folding before execution
- Static control-flow-aware bytecode stack-depth verification
- Arena-style runtime storage for variable slots
- Two execution backends:
  - `jit`, the default, emits generated Python functions
  - `vm`, a portable bytecode interpreter

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
usage: main.py [-h] [--mode {jit,vm}] [--verbose] path
```

Arguments:

- `path`: TinyOne source file to execute
- `--mode jit`: compile bytecode into a generated Python function
- `--mode vm`: execute bytecode with the portable VM
- `--verbose`: enable debug logging

TinyOne exits with status `0` on success and status `1` for file, compile, or
runtime errors.

## Language

TinyOne is an integer-only language with top-level statements, top-level
functions, brace-delimited loop/function bodies, and expressions. Whitespace
separates tokens, but newlines are not significant.

### Statements

```tinyone
let name = expression
print expression
while expression { statements }
return expression
```

`let` defines or updates a variable slot. Variables must be defined before they
are read. `while` repeats while its expression is non-zero. `return` is only
valid inside a function.

### Functions

Functions are declared at top level and return one integer value. Function
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
let answer = add(40, 2)
print answer
```

Calls may appear before the function declaration, but every called function must
be defined exactly once before compilation completes.

### Expressions

Supported expression forms:

```tinyone
123
name
-expression
(expression)
name(expression, ...)
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
2. Function calls and variable reads
3. Unary minus
4. Multiplication and integer division
5. Addition and subtraction
6. Comparisons and equality

Division uses Python-style integer floor division through `//`. Division by zero
is reported as a TinyOne runtime error. Comparisons evaluate to integer `1` for
true and `0` for false, which makes them usable as bounded loop conditions.

### Identifiers

Identifiers may contain letters, digits, and underscores. The first character
must be a letter or underscore.

## Runtime Pipeline

TinyOne executes source code through this pipeline:

```text
source -> lexer -> compiler -> bytecode -> optimizer -> verifier -> VM/JIT
```

The compiler emits stack-machine bytecode. Function bodies are stored as
separate bytecode chunks, and `while` emits branch opcodes. The peephole
optimizer folds constant arithmetic and comparison patterns in branch-free
chunks, such as:

```text
PUSH_INT 2, PUSH_INT 3, MUL -> PUSH_INT 6
```

The verifier checks stack depth across reachable control-flow paths before
execution. Invalid bytecode, mismatched function arity, invalid branch targets,
and stack imbalance fail before either backend runs.

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

- Integers only
- No strings
- No comments
- No module system
- No nested functions or closures
- No direct global-variable access from functions
- No persistent compiled artifact format

## Repository Layout

```text
.
├── README.md
└── main.py
```

`main.py` contains the complete implementation and CLI entrypoint.
