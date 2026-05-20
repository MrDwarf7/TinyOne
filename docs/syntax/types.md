# Types

TinyOne is dynamically typed. Every value at runtime has one of the
following types. Use `type_of(value)` to inspect the type at runtime.

---

## `int`

64-bit signed integer (`i64`). The only numeric type in TinyOne.

**Created by:** integer literals (`42`, `-7`), arithmetic expressions,
comparison expressions (produce `0` or `1`), and most builtin return values.

**Mutated:** integers are values, not references. Assigning to a variable
replaces the slot.

**Overflow:** wraps silently in the current implementation (Rust default).
This behavior is not yet formally specified — see
[v1-roadmap.md](../v1-roadmap.md) item 6.

**Runtime errors:** divide-by-zero (`/` with a zero right operand).

---

## `string`

Heap-allocated, immutable, UTF-8 byte sequence.

**Created by:** string literals (`"hello"`), `str_concat(a, b)`,
`str_slice(s, start, end)`, `str_from_buffer(buf)`, `read_str()`.

**Read:** `len(s)` returns byte length; `s[i]` returns the byte at index
`i`; `str_char_at(s, i)` returns the Unicode codepoint at character
position `i`.

**Mutated:** strings are immutable. To build a new string, use
`str_concat` or `str_slice`.

**Runtime errors:** out-of-bounds index access; `str_char_at` out of range.

**Ownership:** copying a string variable aliases the same heap object.
Strings are not freed by user code (no `unsafe free` on strings —
attempting to free a string is a runtime error).

---

## `array`

Heap-allocated, mutable, zero-indexed, heterogeneous sequence.

**Created by:** array literals (`[10, 20, 30]`), `array(count, fill)`,
`vec_new()`.

**Read:** `arr[i]` returns the element at index `i`; `len(arr)` returns
the element count.

**Mutated:** `set arr[i] = value` writes element `i`; `push(arr, value)`
appends; `pop(arr)` removes and returns the last element.

**Runtime errors:** out-of-bounds index; `pop` on empty array; exceeding
65,536 elements.

**Ownership:** copying an array variable aliases the same heap array.
Free with `unsafe free(arr)` when done. Freeing is shallow — elements
that are themselves heap objects are not freed.

---

## `struct`

Heap-allocated, mutable, named-field record.

**Created by:** struct constructor expression — `StructName(field1, field2, ...)`.
Struct definitions are top-level declarations.

**Read:** `value.field_name` reads a named field.

**Mutated:** `set value.field_name = expr` writes a named field.

**Runtime errors:** accessing a field that does not exist on the struct
definition.

**Ownership:** same as arrays — copying aliases. Free with
`unsafe free(struct_value)`.

---

## `buffer`

Heap-allocated, mutable, zero-initialized byte array. Used for raw
memory operations.

**Created by:** `buffer(size)` — allocates `size` zero bytes. Maximum
size is 1,048,576 (1 MiB).

**Read / mutated:** use raw pointer builtins — `ptr(buf, offset)` to
create a byte pointer, then `unsafe read8/16/32` and
`unsafe write8/16/32` to load and store. All accesses are
little-endian unsigned integers.

**Runtime errors:** out-of-bounds pointer access; buffer exceeds 1 MiB.

**Ownership:** copying a buffer variable aliases the same buffer.
Free with `unsafe free(buf)`.

---

## `cell`

A heap-allocated single-value box. Models an explicit pointer cell.

**Created by:** `alloc(value)` — allocates a cell initialized to `value`.

**Read:** `load(cell)` reads the current value.

**Mutated:** `store(cell, value)` writes `value` and returns it.

**Runtime errors:** using a freed cell.

**Ownership:** copying a cell variable aliases the same cell.
Free with `unsafe free(cell)`.

---

## `pointer` (raw pointer)

A derived alias into a heap object. Can point at an object, an array
element, a struct field, a buffer byte offset, or a cell. `null` is the
null raw pointer literal.

**Created by:**
- `ptr(value)` — object pointer
- `ptr(array, index)` — array element pointer
- `ptr(buffer, offset)` — buffer byte pointer
- `fieldptr(struct_value, "field_name")` — struct field pointer
- `unsafe ptr_at(address)` — reconstruct from numeric address
- `unsafe ptr_add(ptr, offset)` — advance array/buffer pointer
- `null` literal — the null raw pointer

**Read / mutated:** `unsafe ptr_load(ptr)` and `unsafe ptr_store(ptr, value)`.

**Equality:** use `ptr_eq(a, b)` and `ptr_ne(a, b)`. The `==` and `!=`
operators work only on integers.

**Null check:** `is_null(ptr)` returns `1` for null, `0` otherwise.

**Metadata:** `ptr_kind(ptr)`, `ptr_base(ptr)`, `ptr_offset(ptr)`,
`ptr_field(ptr)`, `ptr_type(ptr)`.

**Runtime errors:** stale base object (freed after pointer was created);
wrong kind; out-of-bounds index or offset; null dereference.

**Ownership:** raw pointers are derived aliases. They do not own the base
object. When the base object is freed, all derived pointers to it become
stale and any access to them is a runtime error.

---

## `null`

The null raw pointer sentinel. Not a type of its own — `null` is a
`pointer` value with a null kind.

**Created by:** the `null` literal keyword.

**Use:** `is_null(ptr)` checks for null. `unsafe ptr_load(null)` is a
runtime error.

---
