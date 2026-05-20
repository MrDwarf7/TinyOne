# TinyOne Examples

Runnable examples organized by feature. Each example shows the `.to` source and the expected stdout output. Run any example with:

```sh
cargo run --manifest-path Rust/Cargo.toml --bin tinyone -- example.to
```

---

## Hello World and Arithmetic

```tinyone
print "hello, world"
print 6 * 7
print (10 + 5) / 3
```

```
hello, world
42
5
```

---

## Variables, Scoping, and Block Locals

```tinyone
let x = 10
if x > 5 {
  let y = x * 2
  print y
}

let i = 0
while i < 3 {
  let inner = i + 100
  print inner
  i = i + 1
}
```

```
20
100
101
102
```

---

## Functions and Recursion

```tinyone
fn fact(n) {
  let acc = 1
  while n > 1 {
    acc = acc * n
    n = n - 1
  }
  return acc
}

fn fib(n) {
  if n < 2 { return n }
  return fib(n - 1) + fib(n - 2)
}

print fact(5)
print fact(10)
print fib(10)
```

```
120
3628800
55
```

---

## Structs and Fields

```tinyone
struct Point { x, y }

fn distance_sq(p) {
  return p.x * p.x + p.y * p.y
}

let p = Point(3, 4)
print p.x
print p.y
set p.x = 0
print distance_sq(p)
```

```
3
4
16
```

---

## Arrays and Dynamic Storage

```tinyone
let nums = [10, 20, 30, 40, 50]
print len(nums)
print nums[2]

set nums[2] = 99
print nums[2]

print push(nums, 60)
print pop(nums)
print len(nums)

let squares = []
let i = 1
while i <= 5 {
  let ignored = push(squares, i * i)
  i = i + 1
}
print squares[0]
print squares[4]
```

```
5
30
99
6
60
5
1
25
```

---

## Standard Library: Maps and Strings

Note: each `let` binding must use a unique name within the same scope — `_a`, `_b`, etc. are conventional discard names.

```tinyone
let m = map_new()
let _a = map_set(m, "name", "tinyone")
let _b = map_set(m, "version", "0.5.0")

print map_get(m, "name")
print map_has(m, "version")
print map_len(m)

let keys = map_keys(m)
print len(keys)

let s = "hello, world"
print str_char_len(s)
print str_slice(s, 0, 5)
print str_concat("foo", "bar")
```

```
tinyone
1
2
2
12
hello
foobar
```

---

## Raw Pointers and Buffers

```tinyone
let arr = [10, 20, 30]
let p = ptr(arr, 1)
print unsafe ptr_load(p)
let _a = unsafe ptr_store(p, 99)
print arr[1]

struct Pair { left, right }
let pair = Pair(4, 5)
let fp = fieldptr(pair, "right")
let _b = unsafe ptr_store(fp, 77)
print pair.right

let buf = buffer(4)
let bp = ptr(buf, 0)
let _c = unsafe write8(bp, 42)
let _d = unsafe write8(unsafe ptr_add(bp, 1), 255)
print unsafe read8(bp)
print unsafe read8(unsafe ptr_add(bp, 1))
```

```
20
99
77
42
255
```

---

## Complete Program: Stack Calculator

A simple RPN (reverse-Polish notation) calculator using a manual stack array.

```tinyone
fn calc(ops, inputs) {
  let stack = []
  let i = 0
  while i < len(ops) {
    let op = ops[i]
    if op == 0 {
      let val = inputs[i]
      let _ = push(stack, val)
    }
    if op == 1 {
      let b = pop(stack)
      let a = pop(stack)
      let _ = push(stack, a + b)
    }
    if op == 2 {
      let b = pop(stack)
      let a = pop(stack)
      let _ = push(stack, a * b)
    }
    i = i + 1
  }
  return pop(stack)
}

let ops    = [0, 0, 1, 0, 2]
let inputs = [3, 4, 0, 5, 0]
print calc(ops, inputs)

let ops2    = [0, 0, 2, 0, 0, 2, 1]
let inputs2 = [2, 3, 0, 4, 5, 0, 0]
print calc(ops2, inputs2)
```

```
35
26
```

---

## Verification Instructions

Run each example and confirm the output matches. Create temp files:

```bash
# Hello world
printf 'print "hello, world"\nprint 6 * 7\nprint (10 + 5) / 3\n' > /tmp/ex1.to
cargo run --manifest-path Rust/Cargo.toml --bin tinyone -- /tmp/ex1.to 2>/dev/null
# Expected: hello, world / 42 / 5

# Functions
cat > /tmp/ex3.to << 'EOF'
fn fact(n) {
  let acc = 1
  while n > 1 { acc = acc * n  n = n - 1 }
  return acc
}
fn fib(n) {
  if n < 2 { return n }
  return fib(n - 1) + fib(n - 2)
}
print fact(5)
print fact(10)
print fib(10)
EOF
cargo run --manifest-path Rust/Cargo.toml --bin tinyone -- /tmp/ex3.to 2>/dev/null
# Expected: 120 / 3628800 / 55

# Structs
cat > /tmp/ex4.to << 'EOF'
struct Point { x, y }
fn distance_sq(p) { return p.x * p.x + p.y * p.y }
let p = Point(3, 4)
print p.x
print p.y
set p.x = 0
print distance_sq(p)
EOF
cargo run --manifest-path Rust/Cargo.toml --bin tinyone -- /tmp/ex4.to 2>/dev/null
# Expected: 3 / 4 / 16

# Arrays
cat > /tmp/ex5.to << 'EOF'
let nums = [10, 20, 30, 40, 50]
print len(nums)
print nums[2]
set nums[2] = 99
print nums[2]
print push(nums, 60)
print pop(nums)
print len(nums)
let squares = []
let i = 1
while i <= 5 {
  let ignored = push(squares, i * i)
  i = i + 1
}
print squares[0]
print squares[4]
EOF
cargo run --manifest-path Rust/Cargo.toml --bin tinyone -- /tmp/ex5.to 2>/dev/null
# Expected: 5/30/99/6/60/5/1/25

# Maps and strings
cat > /tmp/ex6.to << 'EOF'
let m = map_new()
let _a = map_set(m, "name", "tinyone")
let _b = map_set(m, "version", "0.5.0")
print map_get(m, "name")
print map_has(m, "version")
print map_len(m)
let keys = map_keys(m)
print len(keys)
let s = "hello, world"
print str_char_len(s)
print str_slice(s, 0, 5)
print str_concat("foo", "bar")
EOF
cargo run --manifest-path Rust/Cargo.toml --bin tinyone -- /tmp/ex6.to 2>/dev/null
# Expected: tinyone/1/2/2/12/hello/foobar

# Raw pointers
cat > /tmp/ex7.to << 'EOF'
let arr = [10, 20, 30]
let p = ptr(arr, 1)
print unsafe ptr_load(p)
let _a = unsafe ptr_store(p, 99)
print arr[1]
struct Pair { left, right }
let pair = Pair(4, 5)
let fp = fieldptr(pair, "right")
let _b = unsafe ptr_store(fp, 77)
print pair.right
let buf = buffer(4)
let bp = ptr(buf, 0)
let _c = unsafe write8(bp, 42)
let _d = unsafe write8(unsafe ptr_add(bp, 1), 255)
print unsafe read8(bp)
print unsafe read8(unsafe ptr_add(bp, 1))
EOF
cargo run --manifest-path Rust/Cargo.toml --bin tinyone -- /tmp/ex7.to 2>/dev/null
# Expected: 20/99/77/42/255

# Calculator
cat > /tmp/ex8.to << 'EOF'
fn calc(ops, inputs) {
  let stack = []
  let i = 0
  while i < len(ops) {
    let op = ops[i]
    if op == 0 { let val = inputs[i]  let _ = push(stack, val) }
    if op == 1 { let b = pop(stack)  let a = pop(stack)  let _ = push(stack, a + b) }
    if op == 2 { let b = pop(stack)  let a = pop(stack)  let _ = push(stack, a * b) }
    i = i + 1
  }
  return pop(stack)
}
let ops    = [0, 0, 1, 0, 2]
let inputs = [3, 4, 0, 5, 0]
print calc(ops, inputs)
let ops2    = [0, 0, 2, 0, 0, 2, 1]
let inputs2 = [2, 3, 0, 4, 5, 0, 0]
print calc(ops2, inputs2)
EOF
cargo run --manifest-path Rust/Cargo.toml --bin tinyone -- /tmp/ex8.to 2>/dev/null
# Expected: 35 / 26
```
