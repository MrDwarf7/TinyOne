# VM and JIT Operation

The TinyOne implementation has two execution backends that share the same verified bytecode:
the **VM** (portable interpreter) and the **JIT** (adaptive bytecode
tier). Both are selected via `--mode vm` or `--mode jit` on the CLI, or
`"vm"` / `"jit"` in the Rust and C APIs.

For bytecode format and opcode semantics, see [`bytecode.md`](bytecode.md).
For heap and memory operation, see [`memory-model.md`](memory-model.md).

---

## VM Backend

### Dispatch Loop

`VM::run` iterates over the main chunk's instruction stream. Each `Instr { op, arg, arg2 }` dispatches to a match arm in `run_chunk`. The match arm reads operands from `arg`/`arg2`, manipulates the operand stack and stack-frame memory, and may call into the heap or runtime helpers.

All operations return `Result`. There are no `unwrap` or `panic` calls on production paths. A runtime error immediately unwinds through `?` and surfaces as a `TinyOneError::Runtime` to the caller.

### Operand Stack

A `Vec<Value>` inside the VM holds the operand stack. Instructions push and pop values. At any point during execution, the stack depth matches what the verifier computed at compile time for that instruction offset.

### Stack-Frame Memory

`TinyMemory` is a flat `Vec<Value>` representing all stack-frame slots for the current call chain. Each function call allocates a contiguous slice of the memory vector for its frame. Slots are zero-initialized at frame entry. Frame memory is not freed between loop iterations — slot values persist across iterations until explicitly overwritten.

### Function Calls

`CALL arg arg2` pops `arg2` arguments from the operand stack, pushes a new frame onto `TinyMemory`, initializes the first `arg2` slots from the arguments, and recurses into `run_chunk` for the callee's bytecode.

The call-depth limit is **16** nested TinyLang function calls. Exceeding this limit returns a `TinyOneError::Runtime("call stack overflow")`.

### Module Capabilities

Imported functions carry a capability set from their package-manifest entry.
Before dispatching a host-facing builtin, the VM selects the grant for the
currently running function's owning module and rejects a missing grant. The
root chunk and root functions retain the authority given to the embedding
application; authority is never inherited from a caller into an imported
module. This check is performed at runtime, including dynamically invoked
module functions. Artifact metadata preserves grants but is not a signature:
untrusted artifacts still need an external OS/process sandbox because their
root chunk is privileged.

The JIT repeats the same check for generic builtin dispatch and for its direct
`free` superinstruction. See [modules](syntax/modules.md) for the manifest
format and the capability-to-builtin map.

### Error Propagation

Every operation that can fail returns `Result`. The `?` operator propagates errors up the call chain. The VM never calls `panic!`, `unwrap`, or `expect` on production paths.

---

## JIT Backend

Startup lowering is lazy: the main chunk is decoded immediately, while each
function chunk is decoded and fused only on its first call. The JIT retains a
single `VerifiedProgram` for all strings and declaration metadata rather than
cloning those tables. Listing generation is the deliberate exception and
lowers every chunk so the output is complete.

The JIT is an **adaptive bytecode tier** — it compiles TinyLang bytecode into a lower-level internal bytecode (`JitOp`) with decoded operands, then interprets `JitOp` and quickens hot loops in-place. It does not produce native machine code.

### Compilation Phase (`JitProgram::compile`)

1. **Verify or accept capability** — raw `Program` entry points verify once;
   `VerifiedProgram` entry points preserve an earlier proof.
2. **Decode operands lazily** — each reached chunk's `Instr { op, arg, arg2 }` is translated to a `JitOp` enum variant with operands already converted to `usize` or `i64`. No conversion happens at dispatch time.
   - `LOAD 3` → `JitOp::Load(3usize)`
   - `LOAD_GLOBAL 1` → `JitOp::LoadGlobal(1usize)`
   - `PUSH_INT 42` → `JitOp::PushInt(42i64)`
   - `CALL 1 2` → `JitOp::Call(1usize, 2usize)`
3. **Fuse superinstructions** — common two- and three-instruction sequences are collapsed into single `JitOp` variants:

   | Bytecode sequence | JIT superinstruction |
   | --- | --- |
   | `PUSH_INT n, STORE s` | `StoreInt(s, n)` |
   | `LOAD s, PUSH_INT n, ADD, STORE s` | `AddSlotInt(s, n)` |
   | `LOAD s, PUSH_INT n, SUB, STORE s` | `SubSlotInt(s, n)` |

### Dispatch Loop (`JitVm`)

`JitVm` iterates over `JitOp` slices. Operand decoding is already done at compile time, so each arm is a direct Rust operation with no conversion overhead.

### Hot-Loop Quickening

Every compiled chunk carries an `edge_counts: Vec<u16>` parallel to its ops. At every backward branch, `JitVm` increments the counter for that branch.

When a counter reaches the configured threshold (**8** by default), the chunk promotes all ops in `[branch_target, branch_instruction + 1)` to faster "hot" variants in-place:

| Cold op | Hot op | Difference |
| --- | --- | --- |
| `Add` | `AddInt` | Checked direct I64 path; generic fallback otherwise |
| `Sub` | `SubInt` | Same |
| `Mul` | `MulInt` | Same |
| `Div` | `DivInt` | Same |
| `Compare(op)` | `CompareInt(op)` | Direct I64 comparison; generic fallback otherwise |
| `Jump(t)` | `JumpHot(t)` | Skips back-edge counter increment |
| `JumpIfZero(t)` | `JumpIfZeroHot(t)` | Same |

Quickening is **in-place and permanent** for the lifetime of the `JitProgram`.
Mixed numeric loops remain correct because quickened arithmetic falls back to
the generic implementation unless both operands are I64 values.

`JitOptions::with_hot_back_edge_threshold` changes the threshold for direct JIT
programs, caches, and configured runner calls. Threshold `0` disables
quickening without disabling JIT lowering or superinstructions. The CLI exposes
the same controls as `--jit-threshold N` and `--no-jit-quickening`.

### JitCache

`JitCache` stores `JitProgram` instances keyed by their Blake2b512 program fingerprint (16 hex bytes). On a cache hit, the already-compiled and potentially already-quickened `JitProgram` is reused — hot ranges from a previous run carry over automatically.

The source entry points keep a second cache of verified compilations. Source
content is bucketed by a 16-byte Blake2 digest and compared byte-for-byte on a
hit, so digest collisions cannot substitute a different program. This removes
lexing, parsing, optimization, and verification from repeated
`JitCache::run_source` calls without weakening first-run validation. Retention
is bounded by deterministic LRU eviction: the defaults are 128 exact sources
and 8 MiB of source text. Configure both limits with
`JitCache::with_source_cache_limits`; zero disables exact-source retention.
`JitCacheStats` exposes source bytes, hits, misses, compilations, evictions, and
bypasses in addition to compiled-program statistics.

The lowered JIT also recognizes branch-safe slot-to-slot moves, in-place
slot/immediate multiply and floor-divide, compare-then-jump, and direct
zero-jump sequences. Hot variants inspect encoded I64 slots directly and avoid
the intermediate stack values; non-I64 tags take the same generic numeric or
truthiness path as ordinary bytecode. Lowering leaves a sequence unfused when
any branch targets its interior. For opt-in attribution,
`JitOptions::with_execution_profile(true)` records opcode dispatches and
operand-stack traffic, while completed runs reuse a bounded pool of cleared
operand vectors.

Compatibility methods accepting raw `Program` values verify before insertion.
`compile_verified` and the `run_verified_program*` methods preserve an earlier
verification capability and its memoized fingerprint.

---

## Choosing a Backend

| Scenario | Recommendation |
| --- | --- |
| Debugging or correctness checking | `vm` — simpler dispatch, easier to correlate with bytecode |
| Production / hot loops | `jit` — superinstructions and quickening reduce dispatch overhead |
| Running the same program repeatedly | `jit` with `JitCache` — quickened state carries over across calls |
| Comparing behavior between backends | Run both; the test suites assert VM/JIT parity |
