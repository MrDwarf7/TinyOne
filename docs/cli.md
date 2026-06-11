# CLI Reference

```
usage: tinylang [OPTIONS] [path]
```

TinyOne exits with status `0` on success, status `1` on any error.

Build the CLI executable before using these commands:

```sh
cargo build --manifest-path TinyOne/Cargo.toml
```

Cargo writes the debug executable to `TinyOne/target/debug/tinylang`
(`TinyOne/target/debug/tinylang.exe` on Windows). For optimized local use,
build with `cargo build --release --manifest-path TinyOne/Cargo.toml`; the
release executable is `TinyOne/target/release/tinylang` or
`TinyOne/target/release/tinylang.exe`. The examples below assume the executable
is available on `PATH` as `tinylang`.

---

## Flags

### `path`

The `.to` source file to compile and execute. If `--run-bytecode` is specified, this argument is not required.

```sh
tinylang example.to
```

---

### `--mode {jit,vm}`

Select the execution backend. Default is `jit`.

- `jit` — compile verified bytecode into the adaptive JIT tier and run it. Superinstructions and hot-loop quickening reduce dispatch overhead on repeated or long-running programs.
- `vm` — interpret bytecode with the portable VM. Simpler and easier to debug; useful for comparing behavior against the JIT.

```sh
# JIT mode (default)
tinylang example.to

# VM mode
tinylang --mode vm example.to
```

---

### `--check`

Compile and verify the source file without executing it. Exits `0` if the program compiles and passes verification, `1` otherwise.

```sh
tinylang --check example.to
```

---

### `--emit-bytecode PATH`

After compilation, write the JSON bytecode artifact to `PATH`. The artifact can be executed later with `--run-bytecode`.

```sh
tinylang \
  --check --emit-bytecode out.tobc.json example.to
```

See [`bytecode.md`](bytecode.md) for the artifact format.

---

### `--emit-jit PATH`

Write the human-readable JIT assembly listing to `PATH` after compilation. Shows the decoded `JitOp` sequence and superinstruction fusions.

```sh
tinylang \
  --emit-jit listing.txt example.to
```

---

### `--run-bytecode PATH`

Execute a pre-compiled JSON artifact from `PATH` without recompiling from source. The artifact is verified again before execution.

```sh
# Compile once
tinylang \
  --check --emit-bytecode out.tobc.json example.to

# Run the artifact
tinylang \
  --run-bytecode out.tobc.json
```

---

### `--input VALUE`

Append one string item to the deterministic input queue. Repeat to supply multiple items. Items are consumed in order by `read()`, `read_int()`, and `read_str()`.

```sh
tinylang \
  --input 10 --input 20 example.to
```

```tinyone
# example.to
let a = read_int()
let b = read_int()
print a + b   # 30
```

---

### `--stdin`

Read stdin line-by-line and append each line to the deterministic input queue.

```sh
printf "10\n20\n" | tinylang --stdin example.to
# Output: 30
```

---

### `--verbose`

Print a compiler/runtime summary to stderr after execution. Includes execution metadata, slot counts, function count, and other program statistics.

```sh
tinylang \
  --verbose example.to
```

---

## Common Workflows

### Compile-once / run-many

```sh
tinylang \
  --check --emit-bytecode program.tobc.json program.to

tinylang \
  --run-bytecode program.tobc.json --input 7
tinylang \
  --run-bytecode program.tobc.json --input 12
```

### Inspect the JIT listing

```sh
tinylang \
  --emit-jit /dev/stdout --check program.to 2>/dev/null
```

### Validate without running (CI)

```sh
tinylang --check program.to && echo "OK"
```

---
