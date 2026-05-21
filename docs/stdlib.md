# TinyOne Standard Library Reference

TinyOne has two layers of builtin functions:

- **Phase-1 (core builtins)** — slots 0–34 in the canonical builtin table.
  These are bytecode-stable: artifacts compiled against Phase-1 builtins will
  continue to work in future versions. Reordering these slots is a breaking
  change.

- **Phase-2 (stdlib bridge builtins)** — slots 35 onward. These are also
  bytecode-stable within the Phase-2 group. Higher-level TinyOne-language
  modules in `stdlib/` wrap these for ergonomic use via `import`.

## Using the Stdlib Modules

The stdlib modules live in `stdlib/` and are loadable via `import` with the
`stdlib/tinyone.json` package manifest:

```tinyone
import "vec"    as vec
import "map"    as map
import "io"     as io
import "string" as str
import "sync"   as sync
import "result" as result
import "option" as option
import "sys"    as sys
import "path"   as path
import "fs"     as fs
import "math"   as math
import "logic"  as logic
import "typing" as typing
```

When running from the repo root you can point to the stdlib manifest:

```sh
# If tinyone.json is in your source directory's ancestor, it resolves automatically.
# Otherwise, copy or symlink stdlib/tinyone.json alongside your source.
cargo run --manifest-path Rust/Cargo.toml --bin tinyone -- your_program.to
```

---

## Phase-1 Core Builtins

### Array and collection

#### `len(value) → int`
Returns the number of elements in an array, the byte length of a string, or the
byte length of a buffer.

#### `array(count, fill) → array`
Allocates a new heap array of `count` elements each initialized to `fill`.
`count` must be a non-negative integer and must not exceed 65,536.

#### `push(array, value) → int`
Appends `value` to `array` in place. Returns the new length. Runtime error if
the array would exceed 65,536 elements.

#### `pop(array) → value`
Removes and returns the last element of `array`. Runtime error on an empty array.

---

### Memory: pointer cells

#### `alloc(value) → cell`
Allocates a heap cell initialized to `value`. Returns a cell reference.

#### `load(cell) → value`
Reads the current value of `cell`.

#### `store(cell, value) → value`
Writes `value` to `cell`. Returns `value`.

#### `unsafe free(value) → null`
Releases a heap object. The object must be a live array, struct, string, buffer,
or cell. Any subsequent access to the freed reference, or to any raw pointer
derived from it, is a runtime error. Freeing is shallow — referenced objects
inside a freed aggregate are not freed.

---

### Input queue

#### `read() → int | string`
Consumes the next item from the deterministic input queue. Returns an integer if
the item parses as a decimal integer, otherwise a heap string. Runtime error
when the queue is empty.

#### `read_int() → int`
Consumes the next item, requiring it to be a valid decimal integer. Runtime
error if the item is not numeric or the queue is empty.

#### `read_str() → string`
Consumes the next item as a heap string regardless of content. Runtime error
when the queue is empty.

#### `to_int(value) → int`
Converts a string to its integer representation. Runtime error if the string is
not a valid decimal integer.

---

### Raw buffers

#### `buffer(size) → buffer`
Allocates a zero-initialized byte buffer of `size` bytes. `size` must not exceed
1,048,576 (1 MiB).

#### `unsafe read8(ptr) → int`
Reads one unsigned byte from buffer pointer `ptr`. Little-endian. Runtime error
on out-of-bounds access or non-buffer pointer.

#### `unsafe write8(ptr, value) → int`
Writes `value` as one unsigned byte. Returns the stored value.

#### `unsafe read16(ptr) → int` / `unsafe write16(ptr, value) → int`
Two-byte little-endian unsigned load/store.

#### `unsafe read32(ptr) → int` / `unsafe write32(ptr, value) → int`
Four-byte little-endian unsigned load/store.

---

### Raw pointers

#### `ptr(value) → pointer`
Creates an object pointer to `value` (must be a heap object).

#### `ptr(array, index) → pointer`
Creates a pointer to element `index` of `array`.

#### `ptr(buffer, offset) → pointer`
Creates a byte pointer at byte offset `offset` of `buffer`.

#### `fieldptr(struct, field_name) → pointer`
Creates a pointer to the named field of `struct`. `field_name` must be a string
literal matching a declared field name.

#### `ptr_addr(value) → int`
Returns the numeric heap address of a heap object. Not stable across runs.

#### `unsafe ptr_at(address) → pointer`
Reconstructs an object pointer from a raw numeric address. The address must have
been returned by `ptr_addr` on a live object in the same run.

#### `unsafe ptr_add(ptr, offset) → pointer`
Advances an array-element or buffer pointer by `offset` positions or bytes.
Runtime error on object or field pointers.

#### `unsafe ptr_load(ptr) → value`
Dereferences a raw pointer. Validates kind, generation, and bounds before access.

#### `unsafe ptr_store(ptr, value) → value`
Stores `value` through a raw pointer. Returns `value`.

#### `ptr_type(ptr) → string`
Returns the explicit cast type tag if set (`cast_ptr` was called), otherwise the
pointer kind string.

#### `is_null(ptr) → int`
Returns `1` if `ptr` is the null raw pointer, `0` otherwise.

#### `ptr_eq(left, right) → int` / `ptr_ne(left, right) → int`
Raw-pointer equality/inequality. Use these for pointer comparisons; the ordinary
`==` and `!=` operators work only on integers.

#### `ptr_base(ptr) → int`
Returns the numeric heap address of the base object.

#### `ptr_offset(ptr) → int`
Returns the element or byte offset within the base object.

#### `ptr_kind(ptr) → string`
Returns the pointer kind: `"object"`, `"array"`, `"buffer"`, `"struct"`,
`"cell"`, or `"null"`.

#### `ptr_field(ptr) → string | null`
Returns the field name for a field pointer, or `null` for other kinds.

#### `cast_ptr(ptr, type_name) → pointer`
Records a type annotation tag on `ptr`. Does not change runtime behavior; used
for debugging and tools. `type_name` must be a string literal.

---

## Phase-2 Stdlib Bridge Builtins

These are called directly or through the stdlib modules.

### Dynamic arrays (`vec`)

#### `vec_new() → array`
Allocates an empty dynamic array (equivalent to `[]`).

#### `vec_clear(v) → int`
Removes all elements from `v` without freeing the array object itself. Returns 0.
Releases the heap byte budget for the element values.

---

### Hash maps (`map`)

Maps are heap-allocated association lists. Keys may be integers, strings, or
raw pointers (pointers are checked for staleness at map access time).

#### `map_new() → map`
Allocates an empty hash map.

#### `map_set(m, key, value) → int`
Insert or update `key → value`. Returns 0. Runtime error if the map's heap byte
budget would be exceeded.

#### `map_get(m, key) → value`
Returns the value for `key`. Runtime error if `key` is not present.

#### `map_has(m, key) → int`
Returns `1` if `key` is in `m`, `0` otherwise.

#### `map_del(m, key) → int`
Removes `key` from `m`. Returns 1 if the key was present, 0 otherwise.

#### `map_len(m) → int`
Returns the number of key-value pairs.

#### `map_keys(m) → array`
Returns a new array containing all keys.

#### `map_values(m) → array`
Returns a new array containing all values in key-insertion order.

---

### I/O abstractions (`io`)

TinyOne I/O operates on deterministic file-descriptor handles. `io_stdout()`,
`io_stderr()`, and `io_stdin()` return the standard handles.

#### `io_write(fd, text) → int`
Writes `text` to `fd` without a trailing newline. Returns 0.

#### `io_writeln(fd, text) → int`
Writes `text` followed by a newline. Returns 0.

#### `io_read_line() → string`
Reads one line from stdin (the deterministic input queue). Strips the trailing
newline. Runtime error when the queue is empty.

#### `io_flush(fd) → int`
Flushes the output buffer for `fd`. Returns 0.

#### `io_capture_stdout() → string` / `io_capture_stderr() → string`
Returns and clears the captured output buffer. Useful for testing.

---

### String / Unicode (`string`)

All character operations use Unicode scalar values (UTF-8 codepoints). Byte
operations use raw byte offsets.

#### `str_byte_len(s) → int`
UTF-8 byte length of string `s`.

#### `str_char_len(s) → int`
Unicode scalar count of `s`.

#### `str_byte_at(s, byte_index) → int`
Returns the byte value at `byte_index`.

#### `str_char_at(s, char_index) → int`
Returns the Unicode codepoint at `char_index`. Runtime error if
`char_index >= str_char_len(s)`.

#### `str_slice(s, start_char, end_char) → string`
Returns the substring from `start_char` (inclusive) to `end_char` (exclusive),
measured in Unicode scalar positions.

#### `str_concat(a, b) → string`
Returns a new string that is the concatenation of `a` and `b`.

#### `str_is_utf8(value) → int`
Returns `1` if `value` is a valid UTF-8 string, `0` otherwise.

#### `str_from_buffer(buf) → string`
Interprets `buf`'s bytes as UTF-8 and returns a new string. Runtime error if
the bytes are not valid UTF-8.

---

### Threading and sync (`sync`)

TinyOne supports real OS multithreading. Threads share the same heap; mutex and
atomic operations use blocking OS primitives.

#### `thread_spawn(fn_name, arg...) → thread`
Spawns an OS thread running the named function with the given arguments. The
thread shares the heap with the spawning program. Returns a thread handle. Up
to 63 arguments may be forwarded. Runtime error if `fn_name` does not name a
defined function or if the argument count does not match the function's arity.

#### `thread_join(handle) → value`
Blocks until the thread finishes, then returns the thread's return value.
Thread stdout is collected and printed before the next `print` statement in
the calling program. Calling `thread_join` on an already-joined handle is a
runtime error.

#### `mutex_new() → mutex`
Allocates an unlocked mutex backed by a real OS condvar.

#### `mutex_lock(m) → int`
Blocks until the mutex is acquired. Runtime error on same-thread deadlock
(re-locking a mutex already held by the calling thread).

#### `mutex_unlock(m) → int`
Releases the mutex. Runtime error if the mutex is not locked by the calling
thread.

#### `atomic_new(init) → atomic`
Allocates an `AtomicI64` initialized to `init`.

#### `atomic_load(a) → int`
Reads the current value with sequential-consistency ordering.

#### `atomic_store(a, value) → int`
Stores `value` with sequential-consistency ordering. Returns `value`.

#### `atomic_add(a, delta) → int`
Atomically adds `delta` with sequential-consistency ordering and returns the
new value. Runtime error on overflow.

---

### Result and Option (`result`, `option`)

Results and options are heap structs with a tag field.

#### Result

```tinyone
let r = result_ok(42)
let e = result_err("not found")
print result_is_ok(r)       # 1
print result_unwrap(r)      # 42
print result_unwrap_err(e)  # "not found"
```

| Function | Description |
| --- | --- |
| `result_ok(v)` | Construct Ok(v) |
| `result_err(v)` | Construct Err(v) |
| `result_is_ok(r)` | 1 if Ok |
| `result_is_err(r)` | 1 if Err |
| `result_unwrap(r)` | Extract Ok value; runtime error on Err |
| `result_unwrap_err(r)` | Extract Err value; runtime error on Ok |

#### Option

```tinyone
let some = option_some(99)
let none = option_none()
print option_is_some(some)   # 1
print option_unwrap(some)    # 99
```

| Function | Description |
| --- | --- |
| `option_some(v)` | Construct Some(v) |
| `option_none()` | Construct None |
| `option_is_some(o)` | 1 if Some |
| `option_is_none(o)` | 1 if None |
| `option_unwrap(o)` | Extract Some value; runtime error on None |

---

### System introspection (`sys`)

Args and environment are injected by the runtime at startup and are
deterministic.

#### `sys_argc() → int`
Number of program arguments.

#### `sys_argv(index) → string`
Argument at zero-based `index`. Runtime error if out of range.

#### `sys_env_has(name) → int`
Returns `1` if environment variable `name` is set.

#### `sys_env_get(name) → string`
Returns the value of environment variable `name`. Runtime error if not set.

---

### Paths (`path`)

#### `path_join(left, right) → string`
Joins `left` and `right` as path components. If `right` is absolute, returns
`right`. Otherwise joins with `/`.

#### `path_basename(p) → string`
Returns the final component of path `p`.

#### `path_dirname(p) → string`
Returns the parent directory of path `p`.

---

### Filesystem (`fs`)

All FS operations that modify or read host state require `unsafe`.

#### `unsafe fs_read(path) → buffer`
Reads the file at `path` as a byte buffer. Runtime error if the file does not
exist, cannot be read, or exceeds 1 MiB.

#### `unsafe fs_write(path, buffer) → int`
Writes `buffer` bytes to `path`, creating or truncating the file. Returns 0.

#### `fs_exists(path) → int`
Returns `1` if `path` exists (file or directory), `0` otherwise.

#### `unsafe fs_list_dir(path) → array`
Returns an array of entry name strings for the directory at `path`. Sorted
lexicographically. Runtime error if:
- The path is not a directory.
- Entry count exceeds 65,536.
- Total bytes of all entry names exceed 1 MiB.

---

### Math (`math`)

#### `math_const(name) → int`
Returns a named mathematical constant as an integer approximation.

#### `math_abs(v) → int`
Absolute value.

#### `math_min(a, b) → int` / `math_max(a, b) → int`
Minimum or maximum of two integers.

---

### Logic (`logic`)

Core `&&`, `||`, and `!` operators are preferred inside conditions because
they short-circuit. These functions remain available for explicit stdlib-style
calls and wrappers.

#### `logic_and(a, b) → int`
Returns `1` if both `a` and `b` are non-zero.

#### `logic_or(a, b) → int`
Returns `1` if either `a` or `b` is non-zero.

#### `logic_not(v) → int`
Returns `1` if `v` is zero, `0` otherwise.

#### `logic_xor(a, b) → int`
Returns `1` if exactly one of `a` or `b` is non-zero.

---

### Typing system (`typing`)

The typing system provides runtime type introspection, fixed-width integer
constructors, and legacy checked integer helpers.

#### `type_of(value) → string`
Returns the runtime type name: `"i64"`, `"u8"`, `"u16"`, `"u32"`, `"String"`,
`"Vec"`, `"Struct"`, `"Buffer"`, `"Alloc"`, `"Map"`, `"Pointer"`, or `"Null"`.

#### `type_id(name) → int`
Returns the integer ID for a type name string.

#### `i64(value)`, `u8(value)`, `u16(value)`, `u32(value) → int`
Converts an integer into the requested runtime width. Unsigned conversions trap
with `Runtime.Memory_Overflow` when the value is negative or too large.

#### `smallest_fit(value) → string`
Returns the smallest integer type name that can represent `value`; non-negative
values prefer unsigned widths.

#### `promote(lhs, rhs) → string`
Returns the name of the smallest type that can represent both `lhs` and `rhs`.

#### `check_int_range(value, type_name) → int`
Returns `value` as a real runtime integer value if it fits in `type_name`.

#### `typed_add(lhs, rhs, type_name) → int`
Adds `lhs + rhs` with overflow checking for `type_name`. Runtime error on
overflow.

#### `typed_sub`, `typed_mul`, `typed_div`, `typed_neg`
Same pattern as `typed_add` for subtraction, multiplication, division, and
negation.

#### `assert(condition)` / `assert(condition, message)`
Runtime error if `condition` is zero. The two-argument form includes `message`
in the error text.
