---
title: Rust API
---

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

### `compile_source(source: &str) -> Result<Arc<Program>>`

Compile a TinyLang source string through lex → compile → optimize → verify. Returns a reference-counted, verified `Program` ready for execution.

```rust
let program = tinyone::compile_source("let x = 6 * 7\nprint x")?;
```

### `compile_source_with_filename(source: &str, filename: &str) -> Result<Arc<Program>>`

Same as `compile_source` but attaches `filename` to diagnostic messages.

### Verified compilation and disk-cache APIs

The `compile_source_verified`, `compile_source_verified_with_filename`,
`compile_file_verified`, and unoptimized verified variants return
`VerifiedProgram` directly. Prefer them when compilation is followed by VM or
JIT execution; the verified token avoids repeating verification and shares a
memoized fingerprint across clones.

`compile_file_cached_verified_with_options(path, optimize)` additionally uses
the dependency-validated disk cache and returns `(VerifiedProgram,
CompileCacheStatus)`. Status is `Hit`, `Incremental`, `Miss`, or `Bypassed`.
`Bypassed` means the graph was deliberately compiled from current source
because it falls below the size-aware cache threshold or resides on a
WSL-mounted Windows drive where validation is slower; it does not mean stale
output was reused. The shorter `compile_file_cached` and
`compile_file_cached_verified` variants use optimized compilation and omit
status where appropriate.

### `compile_file(path: impl AsRef<Path>) -> Result<Arc<Program>>`

Read the file at `path` and compile it. Resolves imports relative to the file's directory.

```rust
let program = tinyone::compile_file(std::path::Path::new("example.to"))?;
```

### `compile_source_unoptimized(source: &str) -> Result<Arc<Program>>`

Compile without the peephole optimizer. Useful for testing the verifier against unoptimized bytecode.

### `compile_file_unoptimized(path: impl AsRef<Path>) -> Result<Arc<Program>>`

Read, resolve imports, compile, and verify a file without running the peephole
optimizer. This is the API used by the CLI's `-O0` mode.

### `compile_source_unoptimized_with_filename(source: &str, filename: &str) -> Result<Arc<Program>>`

Same as `compile_source_unoptimized` but attaches `filename` to diagnostic messages.

### `lex_source(source: &str) -> Result<usize>`

Lex `source` and return the token count. Does not compile.

```rust
let count = tinyone::lex_source("let x = 42")?;
```

### `optimize_program(program: Arc<Program>) -> Arc<Program>`

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

### `run_program(program: Arc<Program>, mode: &str, stdout: &mut dyn Write, inputs: Vec<String>) -> Result<TinyMemory>`

Run a pre-compiled program. Runs `BytecodeVerifier::verify` internally before execution.
Consumes the `Arc<Program>`; clone it first if you need to keep a reference.

```rust
let program = tinyone::compile_source("print 42")?;
let mut out = Vec::new();
tinyone::run_program(program, "vm", &mut out, vec![])?;
```

### `run_program_report(program: Arc<Program>, mode: &str, stdout: &mut dyn Write, inputs: Vec<String>) -> Result<TinyRunReport>`

Same as `run_program` but returns heap statistics.

### `run_program_with_env(program: Arc<Program>, mode: &str, stdout: &mut dyn Write, inputs: Vec<String>, sys_args: Vec<String>, sys_env: HashMap<String, String>) -> Result<TinyMemory>`

Run with explicit program arguments and environment variables (consumed by `sys_argc()`, `sys_argv()`, `sys_env_has()`, `sys_env_get()`).

```rust
use std::collections::HashMap;

let program = tinyone::compile_source("print sys_argc()")?;
let mut out = Vec::new();
let env = HashMap::new();
tinyone::run_program_with_env(
    program,
    "vm",
    &mut out,
    vec![],
    vec!["arg1".to_string()],
    env,
)?;
```

The corresponding `run_verified_program*` functions accept
`&VerifiedProgram` and do not re-run the verifier. Use these with the verified
compiler and loader APIs on startup-sensitive paths.

### Configured JIT runner variants

`run_program_with_jit_options` and
`run_program_with_env_and_jit_options` mirror the corresponding program
runners and accept a final `JitOptions` value. Existing runner functions use
`JitOptions::default()`.

```rust
let options = tinyone::JitOptions::new().with_hot_back_edge_threshold(2);
tinyone::run_program_with_jit_options(
    program,
    "jit",
    &mut out,
    vec![],
    options,
)?;
```

For diagnostic attribution, append `.with_execution_profile(true)` and read
`JitProgram::execution_profile()` after a run. The returned profile is opt-in
and reports lowered-opcode dispatches plus operand-stack pushes, pops, and peak
depth; normal JIT execution leaves it disabled.

---

## Artifacts

### `write_artifact(program: &Program, path: impl AsRef<Path>) -> Result<()>`

Serialize `program` to a JSON artifact file at `path`.

```rust
tinyone::write_artifact(&program, std::path::Path::new("out.tobc.json"))?;
```

### `load_artifact(path: impl AsRef<Path>) -> Result<Program>`

Deserialize and verify a JSON or compact binary artifact file. Enforces
resource limits before constructing program tables.

```rust
let program = tinyone::load_artifact(std::path::Path::new("out.tobc.json"))?;
```

`write_binary_artifact(program, path)` writes the compact versioned `.tob`
representation. `load_verified_artifact(path)` auto-detects binary magic or
JSON and preserves the `VerifiedProgram` capability for direct execution.

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

Use `JitCache::with_options(options)` to configure the cache. A hot-back-edge
threshold of zero disables adaptive quickening while keeping lowering,
superinstructions, caching, and JIT execution enabled.

Exact-source verified compilations use a separate bounded LRU cache. It
defaults to 128 entries and 8 MiB of retained source text. Configure it while
constructing the cache:

```rust
let cache = tinyone::JitCache::with_options(options)
    .with_source_cache_limits(64, 4 * 1024 * 1024);
```

Either limit may be zero to disable exact-source retention without disabling
the fingerprint-keyed compiled JIT cache.

Key methods on `JitCache`:

- `JitCache::with_options(options) -> JitCache` — create a configured cache
- `cache.options() -> JitOptions` — inspect its immutable JIT configuration

- `JitCache::new() -> JitCache` — create an empty cache
- `cache.len() -> usize` — number of cached programs
- `cache.source_cache_len() -> usize` — number of exact sources with a cached verified compilation
- `cache.source_cache_bytes() -> usize` — retained exact-source text bytes
- `cache.source_cache_max_entries() -> usize` — configured entry limit
- `cache.source_cache_byte_limit() -> usize` — configured retained-source byte limit
- `cache.is_empty() -> bool` — true when the cache holds no programs
- `cache.compile(program: &Program) -> Result<&JitProgram>` — compile and cache without running; verifies the program first
- `cache.compile_verified(program: &VerifiedProgram) -> Result<&JitProgram>` — compile without duplicate verification or hashing
- `cache.run_program(program, stdout, inputs) -> Result<TinyMemory>` — compile (if not cached) and run
- `cache.run_verified_program(program, stdout, inputs) -> Result<TinyMemory>` — verified-token run path
- `cache.run_program_report(program, stdout, inputs) -> Result<TinyRunReport>` — same, but includes heap statistics
- `cache.run_program_with_env(program, stdout, inputs, sys_args, sys_env) -> Result<TinyMemory>` — run with explicit args and environment
- `cache.run_source(source, stdout, inputs) -> Result<TinyMemory>` — compile and verify new source, then reuse its verified compilation and JIT state on exact-source hits
- `cache.run_source_report(source, stdout, inputs) -> Result<TinyRunReport>` — same, with heap statistics
- `cache.stats() -> JitCacheStats` — aggregate stats across all cached programs

Source-cache statistics include `source_bytes`, `source_hits`,
`source_misses`, `source_compilations`, `source_evictions`, and
`source_bypasses`.

### `write_jit_listing(program: &Program, path: impl AsRef<Path>) -> Result<()>`

Compile `program` through the JIT and write the human-readable assembly listing to `path`.

```rust
tinyone::write_jit_listing(&program, std::path::Path::new("listing.txt"))?;
```

Use `write_verified_jit_listing(&verified, path)` when a trusted compiler or
artifact loader already returned `VerifiedProgram`; it reuses the capability
and memoized fingerprint instead of verifying again. The raw-program function
continues to verify before lowering.

---

## Verification

### `BytecodeVerifier::verify(program: &Program) -> Result<()>`

Run the BFS stack-depth verifier over all chunks. Enforces resource limits (max functions, ops, slots, strings, structs, modules) before walking any bytecode. Returns `Ok(())` on success.

```rust
tinyone::BytecodeVerifier::verify(&program)?;
```

### `VerifiedProgram`

A capability wrapper that records that verification has already run. It owns
an `Arc<Program>` and a shared, lazily initialized fingerprint. Construct it via
`VerifiedProgram::verify(program)` or obtain it from a verified compiler or
artifact-loader API.

```rust
let verified = tinyone::compile_source_verified("print 1")?;
// Borrow the inner Program without consuming:
let _inner: &tinyone::Program = verified.program();
let fingerprint: &str = verified.fingerprint(); // memoized and clone-shared
// Consume and recover an owned Program (the type-system guarantee is lost):
let program = verified.into_program();
```

Key methods on `VerifiedProgram`:

- `VerifiedProgram::verify(program: Program) -> Result<VerifiedProgram>` — verify and wrap; takes an owned `Program`, not `Arc<Program>`
- `verified.program() -> &Program` — borrow the inner program
- `verified.into_program() -> Program` — consume and return the inner program

### Feature-gated runtime cost counters

With the `testing-hooks` feature enabled, external performance harnesses can
measure heap-lock acquisitions, value encodes/decodes, Ralloc growth events,
and allocator bytes copied:

```rust
use tinyone::testing::{reset_runtime_cost_counters, runtime_cost_counters};

reset_runtime_cost_counters();
// Run one isolated workload here.
let costs = runtime_cost_counters();
println!("{costs:?}");
```

These counters are process-wide and include spawned runtime threads. Reset
them only when the harness owns an exclusive measurement interval; they are a
diagnostic testing surface, not part of the production runtime API.
