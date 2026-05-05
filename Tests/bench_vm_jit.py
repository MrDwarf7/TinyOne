#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Callable

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from main import (  # noqa: E402
    BytecodeVerifier,
    Compiler,
    JitCache,
    Lexer,
    PeepholeOptimizer,
    Program,
    TinyMemory,
    VM,
    compile_source,
    run_source,
)


STRAIGHTLINE_SOURCE = """
let a = 1
let b = a + 2
let c = b * 3
let d = c - a
let e = d / 2
print e
print e >= 4
"""

LOOP_SOURCE = """
let i = 0
let total = 0
while i < 128 {
  let total = total + (i * 3)
  let i = i + 1
}
print total
"""

FUNCTION_SOURCE = """
fn mul_by_count(value, count) {
  let acc = 0
  while count > 0 {
    let acc = acc + value
    let count = count - 1
  }
  return acc
}

fn pair(x) {
  return mul_by_count(x, 2) + mul_by_count(x + 1, 3)
}

let i = 1
let total = 0
while i <= 32 {
  let total = total + pair(i)
  let i = i + 1
}
print total
"""

CONTROL_INTERRUPT_SOURCE = """
let i = 0
let pulses = 0
while i < 96 {
  let gate = 1
  while gate {
    let pulses = pulses + i
    let gate = 0
  }
  let i = i + 1
}
print pulses
"""

HEAP_SOURCE = """
struct Point { x, y }
let values = [1, 2, 3, 4, 5]
let i = 0
while i < len(values) {
  set values[i] = values[i] * 3
  let i = i + 1
}
let point = Point(values[1], len("tinyone"))
set point.y = point.y + values[4]
print point.x
print point.y
print values
"""

INPUT_SOURCE = """
let value = read_int()
let ptr = alloc(value)
print store(ptr, load(ptr) + 1)
let ignored = free(ptr)
"""


class NullWriter:
    def write(self, text: str) -> int:
        return len(text)

    def flush(self) -> None:
        return None


NULL_WRITER = NullWriter()


@dataclass(frozen=True)
class Fixture:
    source: str
    raw: Program
    program: Program
    jit_fn: Callable[[TinyMemory, NullWriter], None]


@dataclass(frozen=True)
class Benchmark:
    name: str
    iterations: int
    fn: Callable[[], None]


def make_fixture(source: str) -> Fixture:
    raw = Compiler(source).compile()
    program = compile_source(source)
    jit_fn = JitCache().compile(program)
    return Fixture(source, raw, program, jit_fn)


def run_vm(program: Program) -> None:
    VM(program, TinyMemory(program.slot_count), NULL_WRITER).run()


def run_jit(fixture: Fixture) -> None:
    fixture.jit_fn(TinyMemory(fixture.program.slot_count), NULL_WRITER)


def build_benchmarks() -> list[Benchmark]:
    straightline = make_fixture(STRAIGHTLINE_SOURCE)
    loop = make_fixture(LOOP_SOURCE)
    functions = make_fixture(FUNCTION_SOURCE)
    control_interrupts = make_fixture(CONTROL_INTERRUPT_SOURCE)
    heap = make_fixture(HEAP_SOURCE)
    hot_cache = JitCache()
    hot_cache.compile(functions.program)
    memory = TinyMemory(1024)

    def memory_load_store() -> None:
        for slot in range(64):
            memory.store(slot, slot * 3)
            memory.load(slot)

    def memory_reset() -> None:
        memory.store(511, 7)
        memory.reset()

    def memory_snapshot() -> None:
        memory.snapshot()

    return [
        Benchmark("memory.allocate_8", 100_000, lambda: TinyMemory(8)),
        Benchmark("memory.allocate_1024", 30_000, lambda: TinyMemory(1024)),
        Benchmark("memory.load_store_64", 15_000, memory_load_store),
        Benchmark("memory.reset_1024", 30_000, memory_reset),
        Benchmark("memory.snapshot_1024", 30_000, memory_snapshot),
        Benchmark("frontend.lex", 10_000, lambda: Lexer(FUNCTION_SOURCE).tokenize()),
        Benchmark("compiler.emit_bytecode", 3_000, lambda: Compiler(FUNCTION_SOURCE).compile()),
        Benchmark(
            "optimizer.straightline",
            20_000,
            lambda: PeepholeOptimizer.optimize(straightline.raw),
        ),
        Benchmark(
            "optimizer.control_flow_passthrough",
            20_000,
            lambda: PeepholeOptimizer.optimize(loop.raw),
        ),
        Benchmark(
            "verifier.loop_cfg",
            30_000,
            lambda: BytecodeVerifier.verify(loop.program),
        ),
        Benchmark(
            "verifier.function_cfg",
            20_000,
            lambda: BytecodeVerifier.verify(functions.program),
        ),
        Benchmark(
            "verifier.heap_structs",
            20_000,
            lambda: BytecodeVerifier.verify(heap.program),
        ),
        Benchmark("compile.full_pipeline", 2_000, lambda: compile_source(FUNCTION_SOURCE)),
        Benchmark("program.fingerprint", 50_000, lambda: functions.program.fingerprint),
        Benchmark(
            "jit.codegen_straightline_cold",
            5_000,
            lambda: JitCache().compile(straightline.program),
        ),
        Benchmark(
            "jit.codegen_dispatch_cold",
            1_000,
            lambda: JitCache().compile(functions.program),
        ),
        Benchmark(
            "jit.codegen_heap_cold",
            1_000,
            lambda: JitCache().compile(heap.program),
        ),
        Benchmark("jit.cache_hit", 100_000, lambda: hot_cache.compile(functions.program)),
        Benchmark("runtime.vm_straightline", 10_000, lambda: run_vm(straightline.program)),
        Benchmark("runtime.jit_straightline", 10_000, lambda: run_jit(straightline)),
        Benchmark("runtime.vm_loop_control", 2_000, lambda: run_vm(loop.program)),
        Benchmark("runtime.jit_loop_control", 2_000, lambda: run_jit(loop)),
        Benchmark("runtime.vm_function_calls", 600, lambda: run_vm(functions.program)),
        Benchmark("runtime.jit_function_calls", 600, lambda: run_jit(functions)),
        Benchmark(
            "runtime.vm_control_interrupts",
            2_000,
            lambda: run_vm(control_interrupts.program),
        ),
        Benchmark(
            "runtime.jit_control_interrupts",
            2_000,
            lambda: run_jit(control_interrupts),
        ),
        Benchmark("runtime.vm_heap_structs", 1_000, lambda: run_vm(heap.program)),
        Benchmark("runtime.jit_heap_structs", 1_000, lambda: run_jit(heap)),
        Benchmark(
            "api.run_source_vm_compile_and_run",
            500,
            lambda: run_source(LOOP_SOURCE, mode="vm", stdout=NULL_WRITER),
        ),
        Benchmark(
            "api.run_source_jit_compile_and_run",
            500,
            lambda: run_source(LOOP_SOURCE, mode="jit", stdout=NULL_WRITER),
        ),
        Benchmark(
            "api.run_source_input_heap",
            500,
            lambda: run_source(INPUT_SOURCE, mode="jit", stdout=NULL_WRITER, inputs=["41"]),
        ),
    ]


def format_duration(ns: float) -> str:
    if ns < 1_000:
        return f"{ns:.1f} ns"
    if ns < 1_000_000:
        return f"{ns / 1_000:.2f} us"
    if ns < 1_000_000_000:
        return f"{ns / 1_000_000:.2f} ms"
    return f"{ns / 1_000_000_000:.2f} s"


def run_benchmark(benchmark: Benchmark, repeats: int, quick: bool) -> dict[str, object]:
    iterations = benchmark.iterations
    if quick:
        iterations = max(1, iterations // 20)

    benchmark.fn()
    samples: list[int] = []
    for _ in range(repeats):
        start = time.perf_counter_ns()
        for _ in range(iterations):
            benchmark.fn()
        samples.append(time.perf_counter_ns() - start)

    best_ns = min(samples)
    mean_ns = sum(samples) / len(samples)
    return {
        "name": benchmark.name,
        "iterations": iterations,
        "best_total_ns": best_ns,
        "mean_total_ns": mean_ns,
        "best_per_iter_ns": best_ns / iterations,
        "mean_per_iter_ns": mean_ns / iterations,
    }


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Benchmark TinyOne VM/JIT internals")
    parser.add_argument("--quick", action="store_true", help="run shorter smoke timings")
    parser.add_argument("--json", action="store_true", help="print JSON instead of a table")
    parser.add_argument("--filter", default="", help="only run benchmarks containing text")
    parser.add_argument("--repeats", type=int, default=5, help="timing repeats per benchmark")
    return parser.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv[1:])
    if args.repeats < 1:
        raise SystemExit("--repeats must be at least 1")

    benchmarks = [
        benchmark
        for benchmark in build_benchmarks()
        if not args.filter or args.filter in benchmark.name
    ]
    if not benchmarks:
        raise SystemExit("no benchmarks matched")

    results = [run_benchmark(benchmark, args.repeats, args.quick) for benchmark in benchmarks]

    if args.json:
        print(json.dumps(results, indent=2, sort_keys=True))
        return 0

    print("TinyOne VM/JIT benchmark suite")
    print(f"benchmarks={len(results)} repeats={args.repeats} quick={args.quick}")
    print()
    print(f"{'benchmark':38} {'iters':>9} {'best/iter':>12} {'mean/iter':>12}")
    print("-" * 76)
    for result in results:
        print(
            f"{result['name']:<38} "
            f"{result['iterations']:>9} "
            f"{format_duration(result['best_per_iter_ns']):>12} "
            f"{format_duration(result['mean_per_iter_ns']):>12}"
        )

    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
