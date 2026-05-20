# Modules

TinyOne's module system separates source files into independently
compiled namespaces. An `import` declaration compiles and links another
source file and makes its exported declarations available under an alias.

---

## `import` Syntax

```
import "path/to/module.to" as alias
import "module-name" as alias
```

Import declarations must appear before any executable statements or
non-import declarations in the file. Placing an `import` after a `let`,
`fn`, or `struct` is a compile error.

The imported file is compiled into a separate function chunk with its
own export table. Importing a module does not run hidden top-level
code — module files may only contain `import`, `struct`, `fn`, and
`export` declarations; top-level executable statements in a module are
a compile error.

---

## Path Resolution

Import paths are resolved in this order:

1. **Relative path:** if the path ends in `.to`, it is resolved relative
   to the importing file's directory.
   ```
   import "lib/math.to" as math
   ```

2. **Manifest lookup:** if the path does not end in `.to`, TinyOne
   searches for a `tinyone.json` package manifest in the importing
   file's directory and then in each ancestor directory up to the
   filesystem root. The first manifest that maps the module name wins.
   ```
   import "math" as math   # resolved via tinyone.json
   ```

3. **Stem alias:** if `as alias` is omitted, the filename stem (without
   `.to`) is used as the namespace.
   ```
   import "lib/math.to"   # accessible as math.add(...)
   ```

---

## `tinyone.json` Package Manifest

A `tinyone.json` file in a directory maps module names to source paths:

```json
{
  "package": "myproject",
  "modules": {
    "math": "lib/math.to",
    "utils": "lib/utils.to"
  }
}
```

With this manifest, `import "math" as m` resolves `lib/math.to` relative
to the manifest file. The `"package"` key is optional metadata.

---

## Export Visibility

In a module source file, declarations without `export` are private —
they cannot be accessed by importing files. Declarations prefixed with
`export` are public.

```tinyone
# math.to
fn normalize(x) { return x }          # private helper

export fn add(a, b) {                  # visible to importers
  return normalize(a) + normalize(b)
}

export struct Vec2 { x, y }            # visible to importers
```

Only `fn` and `struct` declarations can be exported. Variables declared
with `let` in a module are not visible to importers.

---

## Using Imported Declarations

```tinyone
import "math.to" as math

let result = math.add(40, 2)      # call exported function
let v = math.Vec2(1, 2)           # construct exported struct
print v.x                          # access struct field
```

Qualified calls use `namespace.name(...)` syntax. There is no wildcard
import; all accesses require the namespace prefix.

---

## Circular Import Detection

If module A imports module B and module B imports module A (directly or
transitively), TinyOne reports a compile error. Circular imports are
detected via a seen-set in the compiler's shared state.

---

## Worked Example: Two-File Project with a Manifest

**Directory layout:**

```
project/
├── tinyone.json
├── main.to
└── lib/
    └── counter.to
```

**`tinyone.json`:**

```json
{
  "package": "project",
  "modules": {
    "counter": "lib/counter.to"
  }
}
```

**`lib/counter.to`:**

```tinyone
export struct Counter { value }

export fn new_counter() {
  return Counter(0)
}

export fn increment(c) {
  set c.value = c.value + 1
  return c
}
```

**`main.to`:**

```tinyone
import "counter" as ctr

let c = ctr.new_counter()
c = ctr.increment(c)
c = ctr.increment(c)
print c.value   # 2
```

**Verify the example compiles and runs:**

```bash
mkdir -p /tmp/tinytest/lib
cat > /tmp/tinytest/tinyone.json << 'EOF'
{"package": "project", "modules": {"counter": "lib/counter.to"}}
EOF
cat > /tmp/tinytest/lib/counter.to << 'EOF'
export struct Counter { value }
export fn new_counter() { return Counter(0) }
export fn increment(c) { set c.value = c.value + 1  return c }
EOF
cat > /tmp/tinytest/main.to << 'EOF'
import "counter" as ctr
let c = ctr.new_counter()
c = ctr.increment(c)
c = ctr.increment(c)
print c.value
EOF
cargo run --manifest-path Rust/Cargo.toml --bin tinyone -- /tmp/tinytest/main.to
# Expected output: 2
```

Fix the example if it doesn't produce `2`.
