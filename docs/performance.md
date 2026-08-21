# TinyOne performance workflow

`tinylang-bench` is the performance regression and optimization-targeting
harness. It measures isolated subsystems instead of treating source-to-output
latency as one opaque number.

## Measurement model

- Build and run the optimized binary. Debug timings are not representative.
- Correctness checks run before timings and compare VM/JIT output on every
  runtime workload.
- Runtime rows reuse a `VerifiedProgram`; compiler and verifier cost is kept in
  the compiler, verifier, cache, and API rows.
- The table reports best and mean wall time plus coefficient of variation.
- Windows reports per-thread CPU time and scheduled cycles using
  `GetThreadTimes` and `QueryThreadCycleTime`. The Windows CPU-time clock is
  coarse; rows shorter than its update quantum may show `-` rather than a
  misleading zero or overcount.
- Linux reports high-resolution per-thread CPU time using
  `CLOCK_THREAD_CPUTIME_ID`. On x86/x86_64 it also reports fenced TSC cycles;
  other Linux architectures show no built-in cycle value.
- JSON baselines store wall time, CPU time, and cycle metrics. Linux TSC cycles
  include descheduling, so use `perf stat` when retired instructions, hardware
  cycles, branches, or branch misses are needed.

Use at least seven repeats for optimization decisions and rerun any row with a
coefficient of variation above 10 percent:

```text
cargo build --release --manifest-path TinyOne/Cargo.toml --bin tinylang-bench
./TinyOne/target/release/tinylang-bench --repeats 7
```

When Windows and WSL use the same mounted checkout, keep their Cargo artifacts
separate even though they share the source tree. From WSL at the repository
root, set:

```text
export CARGO_TARGET_DIR="$PWD/TinyOne/target/linux"
```

Then use `$CARGO_TARGET_DIR/release/tinylang-bench` as the benchmark binary.
This prevents one operating system from replacing the other's binaries and
Cargo fingerprints.

## Workload map

| Prefix / pair | Isolated cost |
| --- | --- |
| `allocator.*` | Ralloc allocation, release, and growth |
| `memory.*` | Ralloc-backed VM slots, reset, load/store, and snapshots |
| `frontend.*`, `compiler.*`, `optimizer.*`, `verifier.*` | Source pipeline stages |
| `compiler.file_modules_*` | Multi-file compile versus validated disk-cache hit |
| `program.*` | Fingerprints and JSON/binary artifact conversion |
| `jit.codegen_*`, `jit.cache_hit_*` | Lowering and verified/unverified cache lookup |
| `runtime.vm_*`, `runtime.jit_*` | Execution after verification |
| `runtime.*_hot_loop_4096*` | Dispatch and quickening over 4,096 controlled iterations |
| `runtime.*_vec_*`, `runtime.*_map_*` | Collection builtins and Ralloc-backed payloads |
| `runtime.*_heap_churn` | Allocation, load, explicit free, and generation reuse |
| `api.*` | End-to-end public API cost, including compilation where applicable |

## Implemented optimizations

The first optimization pass produced three material improvements on the Windows
baseline machine:

1. Repeated `JitCache::run_source` now caches the exact source's
   `VerifiedProgram`. Together with the integer runtime fast paths, the warm
   source API row fell from about 52.8 us / 137,000 cycles to 21-28 us /
   54,000-71,000 cycles across clean runs, a 47-60 percent reduction. Digest
   buckets still require exact source equality, and first use is still fully
   compiled and verified.
2. Quickened I64 arithmetic now has checked I64 fast paths with generic
   fallbacks, and Ralloc-backed VM slots update I64 immediates in place. The
   quickened 4,096-iteration JIT loop fell from about 930 us / 2.41 million
   cycles to 559 us / 1.43 million cycles, roughly 40 percent. The quickened
   tier now uses about 27 percent fewer cycles than the no-quickening tier.
3. Indexed map lookup and mutation now combine candidate lookup, stale-pointer
   collection, mutation, and allocator accounting into fewer heap-lock windows.
   The stable JIT map row fell from about 109.2 us / 279,300 cycles to 105.4 us /
   268,700 cycles, about 4 percent. Pointer-generation validation still occurs
   before key equality is accepted.

The arithmetic implementation preserves overflow errors and falls back to the generic
numeric path for floats, unsigned values, and narrower signed integers.

An Arch Linux WSL2 confirmation run from the shared `/mnt/c` worktree produced
the same CPU-side result. Across 11 repeats, the quickened hot loop used about
425.5 us / 1.10 million TSC cycles per iteration versus 516.8 us / 1.34 million
for the non-quickened tier, an 18 percent reduction. Identically configured
`perf stat` captures reported 5.48 billion retired instructions and 2.04
billion hardware cycles for quickened execution versus 7.40 billion
instructions and 2.47 billion cycles without quickening: 26 percent fewer
instructions and 17 percent fewer hardware cycles. The warm source API row was
17.2 us / 44,600 TSC cycles. These runtime rows are CPU-focused; filesystem
rows from `/mnt/c` also include WSL's mounted-filesystem behavior.

## Current optimization order

The first cycle-aware baseline on Windows identified these priorities. The
absolute values are machine-specific; the paired ratios are the useful signal.

1. **Keep verified capabilities on hot paths.** A JIT cache hit through a raw
   `Program` took about 33,100 cycles, while `compile_verified` took about 291
   cycles. Re-verification and re-fingerprinting dominate compatibility cache
   lookup. Optimize callers and internal flows to retain `VerifiedProgram`;
   do not weaken verification at untrusted boundaries.
2. **Reduce small-project disk-cache overhead.** The multi-file cache-hit row
   took about 1.12 million cycles versus 880,000 for an uncached compile. Binary
   decode, dependency hashing/canonicalization, and full verification should be
   profiled independently. A size-aware bypass may be worthwhile, but needs a
   larger-module workload before choosing a threshold.
3. **Continue reducing dispatch and operand-stack traffic.** Specialized
   arithmetic and direct I64 slot updates made quickening effective; the
   quickened JIT now takes about 1.43 million cycles versus 2.70 million for the
   VM on the controlled loop. Additional safe superinstructions and fewer
   stack moves are the next runtime candidates.
4. **Continue attacking heap/builtin common costs.** Consolidating map lock
   windows produced a measurable first gain, but JIT gains still nearly
   disappear for vector push/pop and heap churn. Profile value
   encoding/decoding and Ralloc growth before adding more bytecode dispatch
   specializations.
5. **Use `memory.*` and `allocator.*` as guardrails.** These microbenchmarks
   catch regressions in slot initialization, reset, copying, allocator scans,
   and reallocations that would otherwise be multiplied by every VM run.

Treat an optimization as credible when best wall time and the platform's
primary CPU-work metric improve across two clean runs, correctness still
passes, and variance remains below 10 percent. Prefer scheduled cycles on
Windows, thread CPU time on Linux, and `perf stat` when native instruction
counts matter. Save machine-local baselines rather than committing them,
because scheduler, CPU, power policy, and toolchain differences make absolute
numbers non-portable.
