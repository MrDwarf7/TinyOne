---
title: Syntax - Modules
---

# Modules

TinyLang's module system separates source files into independently
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

The imported file is compiled into qualified program functions and structs
with module ownership and an export table. Importing a module does not run hidden top-level
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

2. **Manifest lookup:** if the path does not end in `.to`, the compiler
   searches for a `tinyone.json` package manifest in the importing
   file's directory and then in each ancestor directory up to the entry
   file's directory. The first manifest that maps the module name wins.
   ```
   import "math" as math   # resolved via tinyone.json
   ```

3. **Stem alias:** if `as alias` is omitted, the filename stem (without
   `.to`) is used as the namespace.
   ```
   import "lib/math.to"   # accessible as math.add(...)
   ```

The entry file's directory is the module sandbox root. Imports and manifest
targets must be relative `.to` files inside that root. Absolute paths,
`..` traversal, and symlinks that resolve outside the root are rejected before
source text is read. This keeps an imported dependency from silently reaching
into a parent project, user home directory, or machine-wide manifest.

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

For a module that needs a host-facing facility, use the object form and grant
only that facility:

```json
{
  "package": "myproject",
  "modules": {
    "cache": {
      "path": "lib/cache.to",
      "capabilities": ["filesystem"]
    },
    "renderer": {
      "path": "lib/renderer.to",
      "capabilities": ["graphics"]
    }
  }
}
```

The legacy string form remains valid but grants **no** host capabilities. An
imported module starts with no host authority; the root program retains the
authority supplied by its embedding application. Grants belong to the module
itself, not its caller, so a privileged root cannot accidentally lend its
authority to an unprivileged dependency.

| Capability | Current protected surface |
| --- | --- |
| `filesystem` | `fs_read`, `fs_write`, `fs_exists`, `fs_list_dir` |
| `environment` | `sys_env_has`, `sys_env_get` |
| `threads` | `thread_spawn`, `thread_join` |
| `unsafe_memory` | unsafe pointer, buffer, allocation, and free operations |
| `network` | Reserved for future socket/network bridge builtins |
| `graphics` | Reserved for future GPU/graphics bridge builtins |

`unsafe` syntax is still required for unsafe builtins, but it is not a
capability grant. A module needs both the syntax and `unsafe_memory`. The VM
and JIT enforce the same policy at every builtin dispatch, and capability
metadata is retained in JSON and binary artifacts. Artifacts that omit module
capabilities decode with an empty grant set.

Artifact capability metadata is a deployment request, not a signature or an
OS security boundary: a host that executes an artifact from an untrusted party
must still use an external process/container/OS sandbox. In particular, the
artifact's root code has the embedding application's authority, just as source
entrypoint code does.

String-selected calls (`closure_new` and `thread_spawn`) also follow the
caller's import/export boundary. An imported module cannot name a root function
to turn that function into a privileged deputy; it may select its own functions
or exported functions from modules it imports.

TinyOne does not yet expose socket or GPU builtins. The reserved `network` and
`graphics` grants establish the stable contract those bridges must use when
they arrive; they do not by themselves provide access to a device or network.

Within one compilation session, TinyOne caches canonical resolutions, parsed
manifests (including missing-manifest probes), and source text. Repeated or
diamond-shaped imports do not reread the same file. CLI source runs also use a
dependency-validated `.tinyone-cache/` artifact by default; `--no-cache`
disables it. A single changed module with stable declaration topology can be
relocated into the cached program and the complete result is re-verified.

Manifest targets are TinyLang source files. Native shared libraries (`.dll`,
`.so`, versioned `.so.*`, and `.dylib`) are rejected explicitly, including
when hidden behind a manifest module name. Loading arbitrary native code in
the VM/JIT process would bypass bytecode verification and could corrupt the
runtime. The planned native-module boundary requires a versioned C ABI,
validated value marshalling, capability declarations, and generation-checked
foreign handles; untrusted libraries additionally require process isolation.
Capability checks are language-runtime checks, not a replacement for an OS
sandbox around arbitrary native code.

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

Visibility is enforced twice: by the source compiler and by the bytecode
verifier. This matters for JSON artifacts, which are untrusted input. A forged
artifact cannot call or take a function value for a private module function,
construct a private module struct, access root globals from module code, forge
an export for a missing declaration, or add an invalid/cyclic dependency.
String-based runtime entry points such as `closure_new` and `thread_spawn`
also enforce the invoking function's module/import/export boundary.

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
transitively), the compiler reports a compile error. Circular imports are
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
tinylang /tmp/tinytest/main.to
# Expected output: 2
```

Fix the example if it doesn't produce `2`.
