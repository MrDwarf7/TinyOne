# VM and JIT Operation

TinyOne has two execution backends that share the same verified bytecode:
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

The call-depth limit is **16** nested TinyOne function calls. Exceeding this limit returns a `TinyOneError::Runtime("call stack overflow")`.

### Error Propagation

Every operation that can fail returns `Result`. The `?` operator propagates errors up the call chain. The VM never calls `panic!`, `unwrap`, or `expect` on production paths.

---

## JIT Backend

The JIT is an **adaptive bytecode tier** — it compiles TinyOne bytecode into a lower-level internal bytecode (`JitOp`) with decoded operands, then interprets `JitOp` and quickens hot loops in-place. It does not produce native machine code.

### Compilation Phase (`JitProgram::compile`)

1. **Verify** — runs `BytecodeVerifier::verify` on the input `Program`.
2. **Decode operands** — each `Instr { op, arg, arg2 }` is translated to a `JitOp` enum variant with operands already converted to `usize` or `i64`. No conversion happens at dispatch time.
   - `LOAD 3` → `JitOp::Load(3usize)`
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

When a counter reaches **8** (`HOT_BACK_EDGE_THRESHOLD`), the chunk promotes all ops in `[branch_target, branch_instruction + 1)` to faster "hot" variants in-place:

| Cold op | Hot op | Difference |
| --- | --- | --- |
| `Add` | `AddInt` | Skips `Value` tag check; asserts integer |
| `Sub` | `SubInt` | Same |
| `Mul` | `MulInt` | Same |
| `Div` | `DivInt` | Same |
| `Compare(op)` | `CompareInt(op)` | Same |
| `Jump(t)` | `JumpHot(t)` | Skips back-edge counter increment |
| `JumpIfZero(t)` | `JumpIfZeroHot(t)` | Same |

Quickening is **in-place and permanent** for the lifetime of the `JitProgram`. A loop that mixes integers and heap values will still receive quickened jump ops even if the arithmetic ops do not qualify.

### JitCache

`JitCache` stores `JitProgram` instances keyed by their Blake2b512 program fingerprint (16 hex bytes). On a cache hit, the already-compiled and potentially already-quickened `JitProgram` is reused — hot ranges from a previous run carry over automatically.

`JitCache::compile` re-verifies the program before inserting it. The public `run_*` methods verify once then delegate to internal `*_unchecked` helpers that skip re-verification.

---

## Choosing a Backend

| Scenario | Recommendation |
| --- | --- |
| Debugging or correctness checking | `vm` — simpler dispatch, easier to correlate with bytecode |
| Production / hot loops | `jit` — superinstructions and quickening reduce dispatch overhead |
| Running the same program repeatedly | `jit` with `JitCache` — quickened state carries over across calls |
| Comparing behavior between backends | Run both; the test suites assert VM/JIT parity |
