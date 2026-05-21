# Statements

Statements are separated by newlines or whitespace; there is no semicolon.
Calls and other expressions may be used as statements when only their side
effect matters; the expression result is discarded.

---

## `let` — Variable Declaration

```
let name = expression
```

Declares `name` in the current block scope and initializes it to the
value of `expression`. The name is visible from this line until the end
of the enclosing block. You cannot redeclare a name with `let` in the
same scope.

```tinyone
let x = 10
let y = x + 5
print y   # 15
```

---

## Assignment — Variable Update

```
name = expression
```

Updates the value of an existing visible variable `name`. The variable
must have been declared with `let` before it is assigned. Assignment does
not introduce a new scope entry.

```tinyone
let n = 0
n = n + 1
print n   # 1
```

---

## `print` — Output

```
print expression
```

Evaluates `expression` and writes its string representation to stdout,
followed by a newline. Works on integers, strings, arrays, structs,
buffers, cells, and raw pointers.

```tinyone
print 42
print "hello"
print [1, 2, 3]
```

---

## Expression Statement

```
expression
```

Evaluates `expression` and discards its result. This is the normal form for
calling a function or builtin for its side effect.

```tinyone
let arr = []
push(arr, 1)
unsafe write8(ptr(buffer(4), 0), 255)
```

---

## `unsafe` Block

```
unsafe { statements }
```

Runs the block with unsafe builtins enabled. Use this for clustered pointer,
buffer, raw heap, or filesystem operations.

```tinyone
let buf = buffer(4)
let base = ptr(buf, 0)
unsafe {
  write8(base, u8(255))
  write8(ptr_add(base, 1), u8(1))
}
```

---

## `set` — Aggregate Mutation

```
set name[index] = expression
set name.field  = expression
```

Mutates an array element or struct field. `name` must be a visible
variable holding an array (for index form) or a struct (for field form).

```tinyone
let arr = [10, 20, 30]
set arr[1] = 99
print arr[1]   # 99

struct Point { x, y }
let p = Point(1, 2)
set p.y = 99
print p.y   # 99
```

---

## `if` / `else` — Conditional

```
if expression { statements }
if expression { statements } else { statements }
if expression { statements } else if expression { statements }
```

Evaluates `expression`. If non-zero, executes the first block; if zero
and an `else` clause is present, executes the else block. `else if` chains
are parsed as cascaded conditionals without requiring an extra nested block.
Braces are required around every branch body.

```tinyone
let x = 5
if x > 3 {
  print 1
} else {
  print 0
}
```

---

## `while` — Loop

```
while expression { statements }
```

Evaluates `expression` before each iteration. Repeats while non-zero.
Braces are required.

```tinyone
let i = 0
while i < 5 {
  print i
  i = i + 1
}
```

---

## `break` — Exit Loop

```
break
```

Exits the innermost enclosing `while` loop immediately. Only valid
inside a loop body.

```tinyone
let i = 0
while 1 {
  if i == 3 { break }
  i = i + 1
}
print i   # 3
```

---

## `continue` — Next Iteration

```
continue
```

Jumps to the condition check of the innermost enclosing `while` loop.
Only valid inside a loop body.

```tinyone
let i = 0
while i < 5 {
  i = i + 1
  if i == 3 { continue }
  print i   # prints 1 2 4 5 (skips 3)
}
```

---

## `return` — Function Return

```
return expression
```

Returns `expression` from the current function. Only valid inside a
function body. Not valid at the top level.

```tinyone
fn double(n) {
  return n * 2
}
print double(21)   # 42
```

---

## `struct` — Struct Declaration

```
struct Name { field1, field2, ... }
```

Declares a struct type at the top level. Field names are identifiers
separated by commas. Structs must be declared before they are used as
constructors. Only valid at top level; not valid inside a function body
or loop.

```tinyone
struct Point { x, y }
let p = Point(3, 4)
print p.x   # 3
```

---

## `fn` — Function Declaration

```
fn name(param1, param2, ...) {
  statements
  return expression
}
```

Declares a named function at the top level. A function may call itself
recursively (its name is reserved before the body is compiled). Functions
cannot be nested. Parameters are local slots initialized from the call
arguments. Functions may read top-level variables declared before the function,
but direct assignment to those top-level slots is rejected.

Only valid at top level. Must be declared before it is called.

```tinyone
fn fact(n) {
  let acc = 1
  while n > 1 {
    acc = acc * n
    n = n - 1
  }
  return acc
}
print fact(5)   # 120
```

---

## `export` — Export Modifier

```
export fn name(params) { ... }
export struct Name { fields }
```

Marks a function or struct declaration as publicly visible to importing
files. Only valid in module source files (files that are `import`ed by
another file). Non-exported declarations are private to the module.

```tinyone
# math.to
fn helper(x) { return x }        # private
export fn double(x) { return helper(x) * 2 }  # public
```

---

## `import` — Module Import

See [`modules.md`](modules.md) for the full import system reference.

```
import "path/to/module.to" as alias
```

Must appear before any executable statements or non-import declarations
in the file.

---
