# TinyOne

TinyOne is a tiny integer language implemented as a single Python file. It
includes a lexer, recursive-descent compiler, bytecode optimizer, verifier,
portable VM, and a locals-based Python JIT backend.

The project is intentionally small and dependency-free. It is useful as a
readable example of how a minimal language runtime can move from source text to
verified bytecode and then execute through more than one backend.

## Features

- Single-file implementation in `main.py`
- Python standard library only
- Integer arithmetic with precedence and parentheses
- `let` bindings and `print` statements
- Stack-machine bytecode compiler
- Peephole constant folding before execution
- Static bytecode stack-depth verification
- Arena-style runtime storage for variable slots
- Two execution backends:
  - `jit`, the default, emits a Python function using local variables for the
    virtual stack
  - `vm`, a portable bytecode interpreter

## Requirements

- Python 3.10 or newer
- No package installation step

## Quick Start

Create a TinyOne source file:

```tinyone
let x = 1 + 2 * 3
let y = (x - 4) / 2
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
1
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

TinyOne currently supports statements and expressions only. Whitespace separates
tokens, but newlines are not significant.

### Statements

```tinyone
let name = expression
print expression
```

`let` defines or updates a variable slot. Variables must be defined before they
are read.

### Expressions

Supported expression forms:

```tinyone
123
name
-expression
(expression)
left + right
left - right
left * right
left / right
```

Operator precedence is:

1. Parentheses and literals
2. Unary minus
3. Multiplication and integer division
4. Addition and subtraction

Division uses Python-style integer floor division through `//`. Division by zero
is reported as a TinyOne runtime error.

### Identifiers

Identifiers may contain letters, digits, and underscores. The first character
must be a letter or underscore.

## Runtime Pipeline

TinyOne executes source code through this pipeline:

```text
source -> lexer -> compiler -> bytecode -> optimizer -> verifier -> VM/JIT
```

The compiler emits stack-machine bytecode. The peephole optimizer folds constant
arithmetic patterns such as:

```text
PUSH_INT 2, PUSH_INT 3, MUL -> PUSH_INT 6
```

The verifier then checks stack depth in one pass before execution. Invalid
bytecode fails before either backend runs.

## JIT Backend

The JIT backend does not generate machine code. It emits and compiles a Python
function specialized for the verified TinyOne bytecode.

Instead of using a Python list as the operand stack, the JIT maps virtual stack
positions to Python locals named `_s0`, `_s1`, and so on. That keeps stack
operations in the generated function on `LOAD_FAST` and `STORE_FAST` paths.

## VM Backend

The VM backend interprets the same optimized and verified bytecode. It is
simpler, easier to debug, and useful for checking behavior against the JIT
backend.

## Programmatic Use

`main.py` can also be imported directly:

```python
from io import StringIO
from main import compile_source, run_source

program = compile_source("let x = 40 + 2 print x")

stdout = StringIO()
memory = run_source("let x = 40 + 2 print x", mode="jit", stdout=stdout)

assert stdout.getvalue() == "42\n"
assert memory.snapshot() == (42,)
```

## Current Limitations

- Integers only
- No functions
- No control flow
- No strings
- No comments
- No module system
- No persistent compiled artifact format

## Repository Layout

```text
.
├── README.md
└── main.py
```

`main.py` contains the complete implementation and CLI entrypoint.
