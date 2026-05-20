# Expressions

Expressions produce a value and appear on the right side of `let` and
assignment, as `print` arguments, as `if`/`while` conditions, and as
function call arguments.

---

## Precedence

Higher number = tighter binding (evaluated first).

| Level | Forms |
| --- | --- |
| 6 (highest) | Integer literals, string literals, `null`, identifiers, parentheses `(expr)` |
| 5 | Function calls `f(...)`, qualified calls `ns.f(...)`, struct constructors `Name(...)`, array literals `[...]`, postfix index `a[i]`, postfix field `a.field`, `unsafe expr` |
| 4 | Unary minus `-expr` |
| 3 | `*`, `/` |
| 2 | `+`, `-` |
| 1 (lowest) | `<`, `<=`, `>`, `>=`, `==`, `!=` |

All binary operators are left-associative. Use parentheses to override.

---

## Literals

```tinyone
42          # integer literal
-7          # unary minus applied to 7
"hello"     # string literal (UTF-8, heap-allocated)
null        # null raw pointer
```

---

## Identifiers

```tinyone
x           # read variable x
```

A variable must be declared with `let` before it is read. Variables are
resolved lexically by scope.

---

## Arithmetic

```tinyone
a + b       # integer addition
a - b       # integer subtraction
a * b       # integer multiplication
a / b       # floor (truncating toward -∞) division; runtime error if b == 0
```

Both operands must be integers. Mixing integers with heap objects is a
runtime error.

---

## Comparisons

```tinyone
a < b       # 1 if a < b, else 0
a <= b      # 1 if a ≤ b, else 0
a > b       # 1 if a > b, else 0
a >= b      # 1 if a ≥ b, else 0
a == b      # 1 if a == b, else 0  (integers only)
a != b      # 1 if a != b, else 0  (integers only)
```

All comparison expressions produce integer `0` or `1`. Comparisons
require integer operands. For pointer equality, use `ptr_eq(a, b)` and
`ptr_ne(a, b)`.

---

## Unary Minus

```tinyone
-expr       # negate integer expr
```

---

## Parentheses

```tinyone
(a + b) * c   # override default precedence
```

---

## Function Calls

```tinyone
f(arg1, arg2)             # call top-level function f
namespace.f(arg1, arg2)   # call exported function f from imported module
```

Arguments are evaluated left-to-right. The call pushes arguments onto
the stack and invokes the function chunk.

---

## Struct Constructors

```tinyone
Point(3, 4)   # construct a Point with fields x=3, y=4
```

Fields are assigned positionally in declaration order. The struct must
be declared with `struct` before its constructor is called.

---

## Array Literals

```tinyone
[10, 20, 30]   # heap array of three integers
[]             # empty heap array
```

---

## Postfix Index

```tinyone
arr[i]    # read element i of array arr, or byte i of string arr
```

Strings are byte-indexed (returns the integer byte value at position
`i`). For Unicode-aware access use `str_char_at(s, i)`.

---

## Postfix Field Access

```tinyone
value.field_name   # read named field from struct value
```

---

## `unsafe` Expression

```tinyone
unsafe expr
```

Permits a single expression that is otherwise gated by the `unsafe`
keyword. Required for: `free`, `ptr_at`, `ptr_add`, `ptr_load`,
`ptr_store`, `read8/16/32`, `write8/16/32`, `fs_read`, `fs_write`,
`fs_list_dir`.

`unsafe` gates a single expression, not a block. To chain multiple
unsafe operations, apply `unsafe` to each one individually.

```tinyone
let byte = unsafe read8(ptr(buf, 0))
let _    = unsafe write8(unsafe ptr_add(ptr(buf, 0), 1), 255)
```

---
