# TinyOne performance workflow

`tinylang-bench` is the performance regression and optimization-targeting
harness. It measures isolated subsystems instead of treating source-to-output
latency as one opaque number.

## Measurement model

- Build and run the optimized binary. Debug timings are not representative.
- Correctness checks run before timings and compare VM/JIT output on every
  runtime workload. `--skip-correctness` is for exploratory diagnostics only:
  it records that fact and cannot save a decision-grade baseline.
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
coefficient of variation above 10 percent. The harness records a
decision-eligibility verdict and refuses to save a baseline when pre-timing
correctness was skipped, `--quick` was used, there are fewer than seven
repeats, or any row exceeds its CV limit:

```text
cargo build --release --manifest-path TinyOne/Cargo.toml --bin tinylang-bench
./TinyOne/target/release/tinylang-bench --repeats 7
```

Hot-loop decision rows use a stricter 5 percent coefficient-of-variation
limit. The harness marks rows above their applicable limit with `!`. A normal
`--baseline` comparison remains useful for diagnostics, but prints that it
cannot support an optimization claim if either side is not decision-grade.

Save machine-local baselines under the platform-specific
`TinyOne/target/perf/` tree with:

```text
./TinyOne/target/release/tinylang-bench --repeats 7 \
  --machine-label workstation --power-policy performance \
  --save-baseline-auto
```

Automatic baseline files use schema version 3. Each file contains the result
rows plus the timestamp, package and Rust versions, build profile, OS,
architecture, CPU model, Git commit and dirty state, filesystem context,
measurement options, and optional machine/power-policy labels. Baseline
comparison continues to accept the original result-array format, but legacy
arrays have no quality metadata and therefore cannot pass a decision gate.

Validate the Priority 3 performance criterion against a clean saved baseline
with an all-row, normalized capture (increase `--sample-scale` if a row is
noisy, and keep it identical for each side of a pair):

```text
./TinyOne/target/release/tinylang-bench --priority-3-only --repeats 7 \
  --sample-scale 16 \
  --baseline TinyOne/target/perf/<platform>/baseline-<commit>-<time>.json \
  --priority-3-gate
```

The gate requires decision-grade current and baseline runs and requires the
`runtime.jit_vec_push_pop_256`, `runtime.jit_map_set_get_256`, and
`runtime.jit_heap_churn` rows to improve by at least 10 percent in CPU time or
cycles. It deliberately fails rather than treating a near miss as a pass.

Priority 5 uses a separate guardrail gate. It selects the planned 64-byte and
4-KiB Ralloc allocation rows, 64-to-4096 resize, memory reset, and memory
snapshot; it rejects a regression above 5% in paired mean CPU time or cycles,
falling back to mean wall time only when neither primary metric is available.
The baseline must have matching platform, machine-label, power-policy, repeat,
and sample-scale metadata, and must record `priority_5_only: true`:

```text
./TinyOne/target/release/tinylang-bench --priority-5-only --repeats 7 \
  --sample-scale 4 --machine-label workstation --power-policy performance \
  --save-baseline TinyOne/target/perf/windows-native/priority-5-guardrails-baseline-final.json

./TinyOne/target/release/tinylang-bench --priority-5-only --priority-5-gate \
  --repeats 7 --sample-scale 4 --machine-label workstation --power-policy performance \
  --baseline TinyOne/target/perf/windows-native/priority-5-guardrails-baseline-final.json \
  --save-baseline TinyOne/target/perf/windows-native/priority-5-guardrails-current-final.json
```

The accepted Windows-native and Arch WSL pairs both used `--sample-scale 4`.
All ten saved rows have CV below 5%, and all 22 pre-timing parity checks passed
in every capture. Windows' largest adverse paired mean primary metric is
+0.59% (64-byte allocation cycles); every Linux primary metric improved, with
snapshot cycles down 4.74%. Earlier runs that exceeded 5% or failed the
decision-quality save were rejected and remain diagnostic only.

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
| `allocator.*` | Ralloc allocation, zero-fill, growth, arena-boundary scans, fragmentation, and multithread contention |
| `memory.*` | Ralloc-backed VM slots, reset, load/store, and snapshots |
| `frontend.*`, `compiler.*`, `optimizer.*`, `verifier.*` | Source pipeline stages |
| `compiler.file_modules_*` | Multi-file compile versus the size-aware disk-cache policy |
| `compiler.module_graph_{small,medium,large}_*` | Disk-cache bypass/hit behavior over 2, 16, and 64 imported modules, plus a medium incremental row |
| `compiler.cache_phase.*` | Metadata/input I/O, metadata-prefilter `stat`, metadata decode, hashing, canonicalization, artifact loading, verification, and fingerprint comparison |
| `program.*` | Fingerprints and JSON/binary artifact conversion |
| `jit.codegen_*`, `jit.cache_hit_*` | Lowering and verified/unverified cache lookup |
| `jit.execution_context_setup`, `jit.operand_stack_*`, `jit.chunk_dispatch`, `jit.calls`, `jit.back_edge_promotion` | JIT execution setup, transient stack allocation/reuse, dispatch, calls, and adaptive promotion |
| `runtime.vm_*`, `runtime.jit_*` | Execution after verification |
| `runtime.*_hot_loop_4096*` | Dispatch and quickening over 4,096 controlled iterations |
| `runtime.*_slot_{compare,mul,div}_immediate_4096` | Isolated slot/immediate guard, multiply, and floor-divide paths |
| `runtime.*_vec_push_pop_{16,256,4096}` | Size-scaled vector behavior |
| `runtime.*_vec_{push_in_capacity,capacity_growth,pop,clear}_*` | Individual vector mutation phases |
| `runtime.*_map_set_get_{16,256,4096}` | Size-scaled map behavior |
| `runtime.*_map_{hit,miss,update,insert_in_capacity,delete,capacity_growth,pointer_key_validation}_*` | Individual map lookup, mutation, growth, and safety phases |
| `runtime.*_heap_{allocation,lookup,load,store,free,slot_reuse}_*` | Individual heap lifecycle phases |
| `api.*` | End-to-end public API cost, including compilation where applicable |

Build with `--features testing-hooks` to use
`tinyone::testing::{reset_runtime_cost_counters, runtime_cost_counters}`. The
process-wide diagnostic counters report heap-lock acquisitions, encoded-value
encodes/decodes, successful Ralloc buffer growth events, and bytes copied by
allocator moves. Reset them only around an exclusive measurement interval;
they deliberately aggregate work from spawned runtime threads.

`compiler.cache_phase.binary_decode_verify` retains the public artifact
loader's mandatory verification. Compare it with the separate
`compiler.cache_phase.verification` row to attribute verifier cost without
introducing an unverified artifact-loading API solely for benchmarking.

File fixtures default to the platform temporary directory. Set
`TINYONE_BENCH_FIXTURE_ROOT` to measure a specific filesystem. Benchmark JSON
records both the process working-directory filesystem and the fixture
filesystem, which distinguishes WSL `/mnt/c` runs from WSL native `/tmp` runs.

## Implemented optimizations

The first optimization pass produced three material improvements on the Windows
baseline machine:

1. Repeated `JitCache::run_source` now caches the exact source's
   `VerifiedProgram`. Together with the integer runtime fast paths, the warm
   source API row fell from about 52.8 us / 137,000 cycles to 21-28 us /
   54,000-71,000 cycles across clean runs, a 47-60 percent reduction. Digest
   buckets still require exact source equality, and first use is still fully
   compiled and verified. Retention is bounded by a 128-entry/8-MiB
   deterministic LRU policy, with hit, miss, compilation, eviction, bypass,
   and retained-byte statistics available through `JitCacheStats`.
2. Quickened I64 arithmetic now has checked I64 fast paths with generic
   fallbacks, and Ralloc-backed VM slots update I64 immediates in place. Fused
   slot/immediate multiply, floor-divide, compare/jump, zero-jump, and
   slot-to-slot move operations remove
   additional dispatches, decoding, and operand-stack moves while preserving
   branch-target safety and generic numeric behavior. The
   quickened 4,096-iteration JIT loop fell from about 930 us / 2.41 million
   cycles to 559 us / 1.43 million cycles, roughly 40 percent. The quickened
   tier now uses about 27 percent fewer cycles than the no-quickening tier.
   The later branch-safe slot superinstruction pass reduced the same Windows
   row from 598 us / 1.55 million scheduled cycles to 355 us / 0.92 million
   (about 41 percent). On Arch WSL it moved from 461 us / 1.19 million TSC
   cycles to 276 us / 0.71 million (about 40 percent). Identical Linux `perf`
   workloads fell from 5.734 billion to 3.454 billion retired instructions
   (40 percent) and from 2.511 billion to 1.810 billion hardware cycles
   (28 percent). Packed verifier-bounded operands keep `JitOp` at 24 bytes, so
   the new guard does not inflate every cold lowered operation.
3. The completed collection/heap pass removes several shared fixed costs.
   Indexed map operations use one heap-lock window, canonical hits decode only
   the selected value, and pointer-key validation reads encoded bases under
   that lock. Direct JIT builtins move operands without generic name dispatch;
   I64 map set/get, `len(slot) > 0`, map set with a slot/immediate product, and
   `total + map_get(slot, slot)` have branch-safe lowered forms with generic
   fallbacks. Vector pop avoids a temporary host byte vector, collection growth
   is amortized fourfold, maps reserve their small host index, and cell churn
   reuses one fixed payload. The VM heap no longer shadow-tracks each payload
   through TinyAllocator's diagnostics, while its standalone diagnostics stay
   intact. Two decision-grade Windows captures reduced CPU/cycles by 16-18% /
   19% for vector, 23-24% / 23-24% for map, and 41% / 40-42% for heap churn.
   Two Arch WSL captures reduced them by 13-17% / 13-17%, 22-23% / 22-23%, and
   35-37% / 35-37%, respectively. All four runs executed 22
   pre-timing parity checks and stayed below 10% CV. The earlier 9.05%,
   skipped-correctness Linux vector artifact and high-CV heap artifacts remain
   diagnostic only. Pointer generations, byte budgets, and allocation totals
   remain exact.
4. The disk-cache pass combines hit and incremental probing into one metadata,
   input, resolution, and artifact-validation flow. An incremental rebuild
   reuses the changed source bytes and digest from that probe, and repeated
   aliases reuse canonical resolutions within a compile session. Cache format
   v4 adds a path/size/modification-time prefilter that rejects known-stale
   roots, newly present inputs, and multi-input edits before digest/artifact
   work; matching metadata still falls through to all content digests,
   canonical resolution checks, binary verification, and fingerprint matching.
   A same-size edit with a spoofed stored identity is parity-tested to ensure
   metadata never becomes cache trust. Tiny graphs (<=2 imports and <=4 KiB)
   bypass everywhere. Windows-native medium graphs (3-16 imports and <=64 KiB)
   now also bypass because the two prior paired captures showed only 8-11%
   cache-hit improvement, below the 20% threshold; 64-import graphs remain
   cacheable and the phase suite measures `input_metadata_prefilter` there.
   A bounded 64-entry process LRU remembers compiled bypass decisions without
   retaining or trusting output. WSL source trees on mounted Windows drives
   bypass at every graph size. The former Windows small hit was about 19%
   slower than uncached compilation; bypass was within about 1% across two full
   release runs. Windows large hits were 25% faster in both runs. On WSL's
   native `/tmp` filesystem, medium hits were 44-46% faster and two clean
   11-repeat large-graph confirmations measured 59-60%, while the small bypass
   was effectively neutral. On WSL `/mnt/c`, pre-policy medium and large hits
   were about 24 and 19% slower than recompilation, which is why that filesystem
   bypasses. The v4 decision capture ran all 22 pre-timing parity checks for
   each invocation: Windows medium bypass averaged 3.68 ms versus 3.70 ms
   uncached (CV 0.5% / 1.6%), the 65-source-path prefilter averaged 1.80 ms
   (8.2% CV), and native Arch WSL `/tmp` medium cache hit averaged 357.09 us
   versus 571.36 us uncached (37.5% wall-time reduction; CV 6.3% / 7.6%). The
   WSL `/mnt/c` medium bypass assertion also completed. Any threshold expansion
   still needs a paired capture on every affected filesystem.
5. Allocator coverage now separates 4-KiB zero fill from allocation and adds
   near-arena-capacity, fragmented-arena-cycle, and persistent four-thread
   contention rows. Existing 64-byte, 4-KiB, resize, memory reset, and snapshot
   rows remain unchanged. Runtime profiles did not identify snapshots as a
   production hot path, so snapshot decoding was deliberately left alone.

The arithmetic implementation preserves overflow errors and falls back to the generic
numeric path for floats, unsigned values, and narrower signed integers.

For targeted JIT attribution, compile with
`JitOptions::with_execution_profile(true)` and read
`JitProgram::execution_profile()`. The opt-in profile reports lowered-opcode
dispatches and operand-stack pushes/pops per opcode, plus the observed maximum
stack depth. It is intentionally disabled for normal execution measurements.
Completed JIT runs retain at most eight cleared operand vectors; recursive runs
borrow distinct vectors while active and return them afterward, so reuse never
aliases an active caller's stack.

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
2. **Preserve the measured disk-cache policy.** Tiny graphs bypass everywhere;
   Windows-native medium graphs (3-16 imports, <=64 KiB) bypass because their
   recorded hit rate missed the threshold; large graphs retain verified binary
   cache hits; and changed modules use the single-pass incremental probe.
   Revisit thresholds only with paired Windows, mounted-WSL, and native-
   filesystem measurements; mounted WSL drive trees currently bypass all sizes.
   Never replace content validation with metadata-only trust.
3. **Measure the new dispatch/stack workstream selectively.** Branch-safe
   slot moves, direct zero jumps, in-place slot multiply/divide, and bounded
   operand-stack reuse are now available alongside the earlier compare/jump
   fusions. Use the opt-in opcode/stack profile and the
   `jit.operand_stack_reuse_32` row to retain only changes that improve the
   isolated instruction/cycle evidence without a cold-code regression.
4. **Keep collection paths measured.** Priority 3 now meets its paired CPU
   work target on Windows and WSL. Retain the encoded-slot, stale-pointer, byte
   budget, and lock-acquisition tests as guardrails before pursuing bulk
   `map_keys`/`map_values` work or any larger heap architecture change.
5. **Use `memory.*` and `allocator.*` as guardrails.** These microbenchmarks
   catch regressions in slot initialization, reset, copying, arena-boundary and
   fragmented scans, contention, and reallocations that would otherwise be
   multiplied by every VM run.

Treat an optimization as credible when best wall time and the platform's
primary CPU-work metric improve across two clean runs, correctness still
passes, and variance remains below 10 percent. Prefer scheduled cycles on
Windows, thread CPU time on Linux, and `perf stat` when native instruction
counts matter. Save machine-local baselines rather than committing them,
because scheduler, CPU, power policy, and toolchain differences make absolute
numbers non-portable.
