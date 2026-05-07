#!/usr/bin/env python3
"""
TinyOne VM/JIT benchmark suite.

Usage:
    python bench.py                          # full run, table output
    python bench.py --quick                  # reduced iterations, smoke check
    python bench.py --filter jit             # only benchmarks containing 'jit'
    python bench.py --skip-correctness       # skip output verification
    python bench.py --correctness-only       # verify outputs, no timing
    python bench.py --save-baseline out.json # save results for later comparison
    python bench.py --baseline out.json      # compare against saved results
    python bench.py --json                   # machine-readable output
"""
from __future__ import annotations

import argparse
import io
import json
import statistics
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable, Final

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from Python.main import (  # noqa: E402
    BytecodeVerifier,
    Compiler,
    JitCache,
    Lexer,
    PeepholeOptimizer,
    Program,
    TinyMemory,
    TinyRuntimeContext,
    VM,
    compile_source,
    run_source,
)

# ---------------------------------------------------------------------------
# Source programs
# ---------------------------------------------------------------------------

STRAIGHTLINE_SOURCE: Final = """
let a = 1
let b = a + 2
let c = b * 3
let d = c - a
let e = d / 2
print e
print e >= 4
"""

LOOP_SOURCE: Final = """
let i = 0
let total = 0
while i < 128 {
  total = total + (i * 3)
  i = i + 1
}
print total
"""

FUNCTION_SOURCE: Final = """
fn mul_by_count(value, count) {
  let acc = 0
  while count > 0 {
    acc = acc + value
    count = count - 1
  }
  return acc
}

fn pair(x) {
  return mul_by_count(x, 2) + mul_by_count(x + 1, 3)
}

let i = 1
let total = 0
while i <= 32 {
  total = total + pair(i)
  i = i + 1
}
print total
"""

CONTROL_INTERRUPT_SOURCE: Final = """
let i = 0
let pulses = 0
while i < 96 {
  let gate = 1
  while gate {
    pulses = pulses + i
    gate = 0
  }
  i = i + 1
}
print pulses
"""

HEAP_SOURCE: Final = """
struct Point { x, y }
let values = [1, 2, 3, 4, 5]
let i = 0
while i < len(values) {
  set values[i] = values[i] * 3
  i = i + 1
}
let point = Point(values[1], len("tinyone"))
set point.y = point.y + values[4]
print point.x
print point.y
print values
"""

INPUT_SOURCE: Final = """
let value = read_int()
let ptr = alloc(value)
print store(ptr, load(ptr) + 1)
let ignored = unsafe free(ptr)
"""

# Exercises builtin dispatch: array(), len(), to_int(), set arr[i], arr[j].
BUILTIN_HEAVY_SOURCE: Final = """
let arr = array(16, 0)
let i = 0
while i < len(arr) {
  set arr[i] = to_int(i * 7)
  i = i + 1
}
let total = 0
let j = 0
while j < len(arr) {
  total = total + arr[j]
  j = j + 1
}
print total
"""

# ---------------------------------------------------------------------------
# Correctness cases
#
# Expected values, hand-verified:
#   STRAIGHTLINE  : e = ((1+2)*3 - 1) / 2 = 4;  e >= 4 = 1
#   LOOP          : 3 * sum(0..127) = 3 * 8128 = 24384
#   FUNCTION      : pair(x) = 5x+3;  sum(i=1..32, 5i+3) = 5*528+96 = 2736
#   CONTROL       : sum(i=0..95, i) = 4560
#   HEAP          : values=[3,6,9,12,15]; point=(6, 7+15=22)
#   INPUT("41")   : store(ptr, 41+1) prints 42
#   BUILTIN_HEAVY : 7 * sum(0..15) = 7 * 120 = 840
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class CorrectnessCase:
    name: str
    source: str
    expected: str
    inputs: list[str] = field(default_factory=list)
    mode: str = "jit"


CORRECTNESS_CASES: Final[tuple[CorrectnessCase, ...]] = (
    CorrectnessCase("straightline/jit", STRAIGHTLINE_SOURCE, "4\n1\n"),
    CorrectnessCase("straightline/vm",  STRAIGHTLINE_SOURCE, "4\n1\n",  mode="vm"),
    CorrectnessCase("loop/jit",         LOOP_SOURCE,         "24384\n"),
    CorrectnessCase("loop/vm",          LOOP_SOURCE,         "24384\n", mode="vm"),
    CorrectnessCase("functions/jit",    FUNCTION_SOURCE,     "2736\n"),
    CorrectnessCase("functions/vm",     FUNCTION_SOURCE,     "2736\n",  mode="vm"),
    CorrectnessCase("interrupts/jit",   CONTROL_INTERRUPT_SOURCE, "4560\n"),
    CorrectnessCase("interrupts/vm",    CONTROL_INTERRUPT_SOURCE, "4560\n", mode="vm"),
    CorrectnessCase("heap/jit",         HEAP_SOURCE,         "6\n22\n[3, 6, 9, 12, 15]\n"),
    CorrectnessCase("heap/vm",          HEAP_SOURCE,         "6\n22\n[3, 6, 9, 12, 15]\n", mode="vm"),
    CorrectnessCase("input/jit",        INPUT_SOURCE,        "42\n", inputs=["41"]),
    CorrectnessCase("input/vm",         INPUT_SOURCE,        "42\n", inputs=["41"], mode="vm"),
    CorrectnessCase("builtins/jit",     BUILTIN_HEAVY_SOURCE, "840\n"),
    CorrectnessCase("builtins/vm",      BUILTIN_HEAVY_SOURCE, "840\n", mode="vm"),
)


def run_correctness_checks(cases: tuple[CorrectnessCase, ...]) -> int:
    """Runs all cases.  Returns failure count; prints pass/FAIL per case."""
    failures = 0
    width = max(len(c.name) for c in cases)
    for case in cases:
        buf = io.StringIO()
        try:
            run_source(case.source, mode=case.mode, stdout=buf, inputs=case.inputs)
            actual = buf.getvalue()
        except Exception as exc:
            print(f"  FAIL  {case.name:<{width}}  raised {type(exc).__name__}: {exc}")
            failures += 1
            continue

        if actual == case.expected:
            print(f"  pass  {case.name}")
        else:
            exp = case.expected.replace("\n", "\\n")
            got = actual.replace("\n", "\\n")
            print(f"  FAIL  {case.name:<{width}}  expected {exp!r}  got {got!r}")
            failures += 1

    return failures


# ---------------------------------------------------------------------------
# Benchmark infrastructure
# ---------------------------------------------------------------------------

class NullWriter:
    """Minimal stdout sink accepted by VM/JIT."""

    def write(self, text: str) -> int:
        return len(text)

    def flush(self) -> None:
        pass

    def writelines(self, lines: object) -> None:
        pass


NULL_WRITER: Final = NullWriter()

# Reused across api.run_source_jit_warm calls to demonstrate the correct
# pattern (vs run_source() which creates a fresh JitCache every call).
_WARM_CACHE: Final = JitCache()


@dataclass(frozen=True)
class Fixture:
    source: str
    raw: Program      # pre-optimisation
    program: Program  # optimised + verified
    jit_fn: Callable[[TinyMemory, NullWriter, TinyRuntimeContext | None], None]


@dataclass(frozen=True)
class Benchmark:
    name: str
    iterations: int
    fn: Callable[[], None]


@dataclass(frozen=True)
class BenchmarkResult:
    name: str
    iterations: int
    best_ns: float   # best total time across repeats
    mean_ns: float   # mean total time across repeats
    stdev_ns: float  # stdev of total time across repeats

    @property
    def best_per_iter_ns(self) -> float:
        return self.best_ns / self.iterations

    @property
    def mean_per_iter_ns(self) -> float:
        return self.mean_ns / self.iterations

    @property
    def cv_pct(self) -> float:
        """Coefficient of variation (%).  >10% flags a noisy measurement."""
        if self.mean_ns == 0:
            return 0.0
        return (self.stdev_ns / self.mean_ns) * 100.0

    def to_dict(self) -> dict[str, object]:
        return {
            "name": self.name,
            "iterations": self.iterations,
            "best_per_iter_ns": self.best_per_iter_ns,
            "mean_per_iter_ns": self.mean_per_iter_ns,
            "cv_pct": self.cv_pct,
        }


# ---------------------------------------------------------------------------
# Fixture and runner helpers
# ---------------------------------------------------------------------------

def make_fixture(source: str) -> Fixture:
    raw = Compiler(source).compile()
    program = compile_source(source)
    jit_fn = JitCache().compile(program)
    return Fixture(source, raw, program, jit_fn)


def run_vm(program: Program) -> None:
    VM(program, TinyMemory(program.slot_count), NULL_WRITER).run()


def run_jit(fixture: Fixture) -> None:
    # Explicit None for context so the JIT function creates a fresh
    # TinyRuntimeContext; keeps each iteration isolated.
    fixture.jit_fn(TinyMemory(fixture.program.slot_count), NULL_WRITER, None)


def _warm_run_source_jit(source: str) -> None:
    """Demonstrates the fixed API pattern: compile once, cache forever."""
    program = compile_source(source)
    _WARM_CACHE.compile(program)(TinyMemory(program.slot_count), NULL_WRITER, TinyRuntimeContext())


# ---------------------------------------------------------------------------
# Benchmark definitions
# ---------------------------------------------------------------------------

_WARMUP_ITERS: Final = 3


def build_benchmarks() -> list[Benchmark]:
    straightline       = make_fixture(STRAIGHTLINE_SOURCE)
    loop               = make_fixture(LOOP_SOURCE)
    functions          = make_fixture(FUNCTION_SOURCE)
    control_interrupts = make_fixture(CONTROL_INTERRUPT_SOURCE)
    heap               = make_fixture(HEAP_SOURCE)
    builtins           = make_fixture(BUILTIN_HEAVY_SOURCE)

    # Pre-warm _WARM_CACHE so api.warm benchmarks never pay first-compile cost.
    for fx in (straightline, loop, functions, control_interrupts, heap, builtins):
        _WARM_CACHE.compile(fx.program)

    # Dedicated hot cache for jit.cache_hit_* family.
    hot_cache = JitCache()
    for fx in (straightline, functions, heap):
        hot_cache.compile(fx.program)

    # Shared memory object for memory.* — avoids allocation inside timed loop.
    shared_mem = TinyMemory(1024)

    def memory_load_store() -> None:
        for slot in range(64):
            shared_mem.store(slot, slot * 3)
            shared_mem.load(slot)

    def memory_reset() -> None:
        shared_mem.store(511, 7)
        shared_mem.reset()

    # Artifact round-trip: in-memory, no disk I/O.
    functions_artifact = functions.program.to_artifact()

    return [
        # -- memory ------------------------------------------------------------
        Benchmark("memory.allocate_8",                   100_000, lambda: TinyMemory(8)),
        Benchmark("memory.allocate_1024",                 30_000, lambda: TinyMemory(1024)),
        Benchmark("memory.load_store_64",                 15_000, memory_load_store),
        Benchmark("memory.reset_1024",                    30_000, memory_reset),
        Benchmark("memory.snapshot_1024",                 30_000, shared_mem.snapshot),

        # -- frontend ----------------------------------------------------------
        Benchmark("frontend.lex",                         10_000, lambda: Lexer(FUNCTION_SOURCE).tokenize()),
        Benchmark("compiler.emit_bytecode",                3_000, lambda: Compiler(FUNCTION_SOURCE).compile()),
        Benchmark("optimizer.straightline",               20_000, lambda: PeepholeOptimizer.optimize(straightline.raw)),
        Benchmark("optimizer.control_flow_passthrough",   20_000, lambda: PeepholeOptimizer.optimize(loop.raw)),

        # -- verifier ----------------------------------------------------------
        Benchmark("verifier.loop_cfg",                    30_000, lambda: BytecodeVerifier.verify(loop.program)),
        Benchmark("verifier.function_cfg",                20_000, lambda: BytecodeVerifier.verify(functions.program)),
        Benchmark("verifier.heap_structs",                20_000, lambda: BytecodeVerifier.verify(heap.program)),

        # -- pipeline ----------------------------------------------------------
        Benchmark("compile.full_pipeline",                 2_000, lambda: compile_source(FUNCTION_SOURCE)),

        # -- program metadata --------------------------------------------------
        # fingerprint: after caching fix this should be sub-microsecond.
        # High value here means the fix isn't in effect.
        Benchmark("program.fingerprint",                  50_000, lambda: functions.program.fingerprint),
        Benchmark("program.to_artifact",                   5_000, lambda: functions.program.to_artifact()),
        Benchmark("program.from_artifact",                 2_000, lambda: Program.from_artifact(functions_artifact)),

        # -- JIT codegen: cold -------------------------------------------------
        Benchmark("jit.codegen_straightline_cold",         5_000, lambda: JitCache().compile(straightline.program)),
        Benchmark("jit.codegen_dispatch_cold",             1_000, lambda: JitCache().compile(functions.program)),
        Benchmark("jit.codegen_heap_cold",                 1_000, lambda: JitCache().compile(heap.program)),
        Benchmark("jit.codegen_builtin_cold",              1_000, lambda: JitCache().compile(builtins.program)),

        # -- JIT codegen: warm (cache hit) ------------------------------------
        Benchmark("jit.cache_hit_dispatch",              100_000, lambda: hot_cache.compile(functions.program)),
        Benchmark("jit.cache_hit_straightline",          100_000, lambda: hot_cache.compile(straightline.program)),
        Benchmark("jit.cache_hit_heap",                  100_000, lambda: hot_cache.compile(heap.program)),

        # -- runtime: VM -------------------------------------------------------
        Benchmark("runtime.vm_straightline",              10_000, lambda: run_vm(straightline.program)),
        Benchmark("runtime.vm_loop_control",               2_000, lambda: run_vm(loop.program)),
        Benchmark("runtime.vm_function_calls",               600, lambda: run_vm(functions.program)),
        Benchmark("runtime.vm_control_interrupts",         2_000, lambda: run_vm(control_interrupts.program)),
        Benchmark("runtime.vm_heap_structs",               1_000, lambda: run_vm(heap.program)),
        Benchmark("runtime.vm_builtin_heavy",              2_000, lambda: run_vm(builtins.program)),

        # -- runtime: JIT ------------------------------------------------------
        Benchmark("runtime.jit_straightline",             10_000, lambda: run_jit(straightline)),
        Benchmark("runtime.jit_loop_control",              2_000, lambda: run_jit(loop)),
        Benchmark("runtime.jit_function_calls",              600, lambda: run_jit(functions)),
        Benchmark("runtime.jit_control_interrupts",        2_000, lambda: run_jit(control_interrupts)),
        Benchmark("runtime.jit_heap_structs",              1_000, lambda: run_jit(heap)),
        Benchmark("runtime.jit_builtin_heavy",             2_000, lambda: run_jit(builtins)),

        # -- API end-to-end ----------------------------------------------------
        # cold: run_source() creates a fresh JitCache on every call — regression
        # warm: _WARM_CACHE reused — shows the gap the fix closes
        Benchmark("api.run_source_vm",                       500, lambda: run_source(LOOP_SOURCE, mode="vm", stdout=NULL_WRITER)),
        Benchmark("api.run_source_jit_cold",                 500, lambda: run_source(LOOP_SOURCE, mode="jit", stdout=NULL_WRITER)),
        Benchmark("api.run_source_jit_warm",                 500, lambda: _warm_run_source_jit(LOOP_SOURCE)),
        Benchmark("api.run_source_input_heap",               500, lambda: run_source(INPUT_SOURCE, mode="jit", stdout=NULL_WRITER, inputs=["41"])),
    ]


# ---------------------------------------------------------------------------
# Timing
# ---------------------------------------------------------------------------

def run_benchmark(benchmark: Benchmark, repeats: int, quick: bool) -> BenchmarkResult:
    iterations = max(1, benchmark.iterations // 20) if quick else benchmark.iterations

    # Warmup stabilises instruction caches, branch predictors, OS page faults.
    for _ in range(_WARMUP_ITERS):
        benchmark.fn()

    samples: list[float] = []
    for _ in range(repeats):
        start = time.perf_counter_ns()
        for _ in range(iterations):
            benchmark.fn()
        samples.append(float(time.perf_counter_ns() - start))

    return BenchmarkResult(
        name=benchmark.name,
        iterations=iterations,
        best_ns=min(samples),
        mean_ns=sum(samples) / len(samples),
        stdev_ns=statistics.stdev(samples) if len(samples) > 1 else 0.0,
    )


# ---------------------------------------------------------------------------
# Output
# ---------------------------------------------------------------------------

def format_duration(ns: float) -> str:
    if ns < 1_000:
        return f"{ns:.1f} ns"
    if ns < 1_000_000:
        return f"{ns / 1_000:.2f} us"
    if ns < 1_000_000_000:
        return f"{ns / 1_000_000:.2f} ms"
    return f"{ns / 1_000_000_000:.2f} s"


_CV_WARN: Final = 10.0  # percent


def print_table(results: list[BenchmarkResult]) -> None:
    print(f"\n{'benchmark':<44} {'iters':>7} {'best/iter':>12} {'mean/iter':>12} {'cv%':>6}")
    print("-" * 86)
    for r in results:
        flag = " !" if r.cv_pct > _CV_WARN else "  "
        print(
            f"{r.name:<44} "
            f"{r.iterations:>7} "
            f"{format_duration(r.best_per_iter_ns):>12} "
            f"{format_duration(r.mean_per_iter_ns):>12} "
            f"{r.cv_pct:>5.1f}%{flag}"
        )
    noisy = [r for r in results if r.cv_pct > _CV_WARN]
    if noisy:
        print(f"\n  ! = cv > {_CV_WARN}% — high jitter; try --repeats or close background processes")


def compare_to_baseline(results: list[BenchmarkResult], baseline_path: Path) -> None:
    try:
        raw = json.loads(baseline_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        print(f"\nWarning: could not load baseline {baseline_path}: {exc}", file=sys.stderr)
        return

    baseline: dict[str, float] = {item["name"]: float(item["best_per_iter_ns"]) for item in raw}

    print(f"\nBaseline comparison  ({baseline_path.name})")
    print(f"{'benchmark':<44} {'baseline':>12} {'current':>12} {'delta':>9}")
    print("-" * 82)

    for r in results:
        old = baseline.get(r.name)
        if old is None:
            print(f"  {r.name:<42} {'(new)':>12}")
            continue
        new = r.best_per_iter_ns
        delta = ((new - old) / old) * 100.0
        sign = "+" if delta >= 0 else ""
        marker = " ▲" if delta > 5 else (" ▼" if delta < -5 else "  ")
        print(
            f"  {r.name:<42} "
            f"{format_duration(old):>12} "
            f"{format_duration(new):>12} "
            f"{sign}{delta:>6.1f}%{marker}"
        )

    removed = [name for name in baseline if not any(r.name == name for r in results)]
    for name in removed:
        print(f"  {name:<42} {'(removed)':>12}")


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def parse_args(argv: list[str]) -> argparse.Namespace:
    p = argparse.ArgumentParser(description="TinyOne VM/JIT benchmark suite")
    p.add_argument("--quick",            action="store_true", help="reduce iterations for smoke runs")
    p.add_argument("--json",             action="store_true", help="emit JSON instead of a table")
    p.add_argument("--filter",           default="",          metavar="TEXT", help="only run benchmarks whose name contains TEXT")
    p.add_argument("--repeats",          type=int, default=5, help="timing repeats per benchmark (default: 5)")
    p.add_argument("--skip-correctness", action="store_true", help="skip output verification")
    p.add_argument("--correctness-only", action="store_true", help="run correctness checks and exit")
    p.add_argument("--baseline",         metavar="PATH",      help="compare results against a saved JSON baseline")
    p.add_argument("--save-baseline",    metavar="PATH",      help="save results as a JSON baseline")
    return p.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv[1:])

    if args.repeats < 1:
        print("--repeats must be >= 1", file=sys.stderr)
        return 1

    # -- correctness -----------------------------------------------------------
    if not args.skip_correctness or args.correctness_only:
        print("Correctness checks")
        print("-" * 40)
        failures = run_correctness_checks(CORRECTNESS_CASES)
        if failures:
            print(f"\n{failures} check(s) failed — aborting.")
            return 1
        print(f"\nAll {len(CORRECTNESS_CASES)} checks passed.\n")

    if args.correctness_only:
        return 0

    # -- benchmarks ------------------------------------------------------------
    all_benchmarks = build_benchmarks()
    selected = [b for b in all_benchmarks if not args.filter or args.filter in b.name]
    if not selected:
        print(f"No benchmarks matched {args.filter!r}", file=sys.stderr)
        return 1

    print(f"TinyOne VM/JIT benchmark suite")
    print(f"benchmarks={len(selected)}  repeats={args.repeats}  quick={args.quick}\n")

    results = [run_benchmark(b, args.repeats, args.quick) for b in selected]

    if args.json:
        print(json.dumps([r.to_dict() for r in results], indent=2, sort_keys=True))
    else:
        print_table(results)

    if args.baseline:
        compare_to_baseline(results, Path(args.baseline))

    if args.save_baseline:
        path = Path(args.save_baseline)
        path.write_text(
            json.dumps([r.to_dict() for r in results], indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        print(f"\nBaseline saved → {path}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
