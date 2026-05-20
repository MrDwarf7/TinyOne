# Rust Crate API Reference

Add `tinyone` to your `Cargo.toml` dependencies (path or crate source),
then import the public API:

```rust
use tinyone::{compile_source, run_source, JitCache, TinyOneError};
```

All fallible functions return `tinyone::Result<T>`, an alias for
`Result<T, TinyOneError>`. `TinyOneError` has two variants:
- `TinyOneError::Compile(msg)` — failure during lexing, parsing, or verification
- `TinyOneError::Runtime(msg)` — failure during execution

---

## Compilation

### `compile_source(source: &str) -> Result<Program>`

Compile a TinyOne source string through lex → compile → optimize → verify. Returns a verified `Program` ready for execution.

```rust
let program = tinyone::compile_source("let x = 6 * 7\nprint x")?;
```

### `compile_source_with_filename(source: &str, filename: &str) -> Result<Program>`

Same as `compile_source` but attaches `filename` to diagnostic messages.

### `compile_file(path: impl AsRef<Path>) -> Result<Program>`

Read the file at `path` and compile it. Resolves imports relative to the file's directory.

```rust
let program = tinyone::compile_file(std::path::Path::new("example.to"))?;
```

### `compile_source_unoptimized(source: &str) -> Result<Program>`

Compile without the peephole optimizer. Useful for testing the verifier against unoptimized bytecode.

### `compile_source_unoptimized_with_filename(source: &str, filename: &str) -> Result<Program>`

Same as `compile_source_unoptimized` but attaches `filename` to diagnostic messages.

### `lex_source(source: &str) -> Result<usize>`

Lex `source` and return the token count. Does not compile.

```rust
let count = tinyone::lex_source("let x = 42")?;
```

### `optimize_program(program: Program) -> Program`

Run the peephole optimizer over an already-compiled program. This function is infallible.

---

## Execution

All execution functions take a `mode: &str` that must be `"vm"` or `"jit"`.
`stdout` is any `&mut dyn Write`.

### `run_source(source: &str, mode: &str, stdout: &mut dyn Write, inputs: Vec<String>) -> Result<TinyMemory>`

Compile and run a source string. Writes program output to `stdout`. `inputs` pre-populates the deterministic input queue consumed by `read()`, `read_int()`, and `read_str()`. Returns the final heap state as `TinyMemory`.

```rust
let mut out = Vec::new();
tinyone::run_source("print 6 * 7", "jit", &mut out, vec![])?;
assert_eq!(String::from_utf8(out).unwrap(), "42\n");
```

### `run_source_report(source: &str, mode: &str, stdout: &mut dyn Write, inputs: Vec<String>) -> Result<TinyRunReport>`

Same as `run_source` but returns a `TinyRunReport` containing the final `TinyMemory` plus heap statistics (live objects/bytes, peak, total allocations/frees, shutdown frees).

### `run_program(program: &Program, mode: &str, stdout: &mut dyn Write, inputs: Vec<String>) -> Result<TinyMemory>`

Run a pre-compiled program. Runs `BytecodeVerifier::verify` internally before execution.

```rust
let program = tinyone::compile_source("print 42")?;
let mut out = Vec::new();
tinyone::run_program(&program, "vm", &mut out, vec![])?;
```

### `run_program_report(program: &Program, mode: &str, stdout: &mut dyn Write, inputs: Vec<String>) -> Result<TinyRunReport>`

Same as `run_program` but returns heap statistics.

### `run_program_with_env(program: &Program, mode: &str, stdout: &mut dyn Write, inputs: Vec<String>, sys_args: Vec<String>, sys_env: HashMap<String, String>) -> Result<TinyMemory>`

Run with explicit program arguments and environment variables (consumed by `sys_argc()`, `sys_argv()`, `sys_env_has()`, `sys_env_get()`).

```rust
use std::collections::HashMap;

let program = tinyone::compile_source("print sys_argc()")?;
let mut out = Vec::new();
let env = HashMap::new();
tinyone::run_program_with_env(
    &program,
    "vm",
    &mut out,
    vec![],
    vec!["arg1".to_string()],
    env,
)?;
```

---

## Artifacts

### `write_artifact(program: &Program, path: impl AsRef<Path>) -> Result<()>`

Serialize `program` to a JSON artifact file at `path`.

```rust
tinyone::write_artifact(&program, std::path::Path::new("out.tobc.json"))?;
```

### `load_artifact(path: impl AsRef<Path>) -> Result<Program>`

Deserialize and verify an artifact file. Enforces all resource limits before any allocation.

```rust
let program = tinyone::load_artifact(std::path::Path::new("out.tobc.json"))?;
```

---

## JIT

### `JitCache`

A fingerprint-keyed cache of compiled `JitProgram` instances. Cache hits reuse already-quickened programs across calls.

```rust
let program = tinyone::compile_source("let i = 0\nwhile i < 1000 { i = i + 1 }\nprint i")?;
let mut cache = tinyone::JitCache::new();
let mut out = Vec::new();
cache.run_program(&program, &mut out, vec![])?;
// Second call reuses the compiled and potentially quickened JitProgram:
cache.run_program(&program, &mut out, vec![])?;
```

Key methods on `JitCache`:

- `JitCache::new() -> JitCache` — create an empty cache
- `cache.len() -> usize` — number of cached programs
- `cache.is_empty() -> bool` — true when the cache holds no programs
- `cache.compile(program: &Program) -> Result<&JitProgram>` — compile and cache without running; verifies the program first
- `cache.run_program(program, stdout, inputs) -> Result<TinyMemory>` — compile (if not cached) and run
- `cache.run_program_report(program, stdout, inputs) -> Result<TinyRunReport>` — same, but includes heap statistics
- `cache.run_program_with_env(program, stdout, inputs, sys_args, sys_env) -> Result<TinyMemory>` — run with explicit args and environment
- `cache.run_source(source, stdout, inputs) -> Result<TinyMemory>` — compile source, then run via the cache
- `cache.run_source_report(source, stdout, inputs) -> Result<TinyRunReport>` — same, with heap statistics
- `cache.stats() -> JitCacheStats` — aggregate stats across all cached programs

### `write_jit_listing(program: &Program, path: impl AsRef<Path>) -> Result<()>`

Compile `program` through the JIT and write the human-readable assembly listing to `path`.

```rust
tinyone::write_jit_listing(&program, std::path::Path::new("listing.txt"))?;
```

---

## Verification

### `BytecodeVerifier::verify(program: &Program) -> Result<()>`

Run the BFS stack-depth verifier over all chunks. Enforces resource limits (max functions, ops, slots, strings, structs, modules) before walking any bytecode. Returns `Ok(())` on success.

```rust
tinyone::BytecodeVerifier::verify(&program)?;
```

### `VerifiedProgram`

A newtype wrapper that records that verification has already run. Constructed via `VerifiedProgram::verify(program)`, which runs `BytecodeVerifier::verify` internally.

```rust
let program = tinyone::compile_source("print 1")?;
let verified = tinyone::VerifiedProgram::verify(program)?;
// Borrow the inner Program without consuming:
let _inner: &tinyone::Program = verified.program();
// Consume and recover the inner Program (type-system guarantee is lost):
let program = verified.into_program();
```

Key methods on `VerifiedProgram`:

- `VerifiedProgram::verify(program: Program) -> Result<VerifiedProgram>` — verify and wrap
- `verified.program() -> &Program` — borrow the inner program
- `verified.into_program() -> Program` — consume and return the inner program
