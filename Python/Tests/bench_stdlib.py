#!/usr/bin/env python3
"""Minimal stdlib benchmark harness.

This is intentionally tiny: a flat function-by-function micro timer that
exercises each stdlib builtin under both VM and JIT backends. The output is
plain text. It is not a replacement for `bench_vm_jit.py`; it just provides
a smoke check that none of the new operations have a pathologically bad
cost on either runtime.
"""
from __future__ import annotations

import sys
import time
from io import StringIO
from pathlib import Path
from typing import Callable

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from Python.main import compile_source, run_program as run_compiled_program  # noqa: E402


def measure(label: str, source: str, *, iterations: int = 50) -> dict[str, float]:
    program = compile_source(source)
    results: dict[str, float] = {}
    for mode in ("vm", "jit"):
        start = time.perf_counter()
        for _ in range(iterations):
            out = StringIO()
            run_compiled_program(program, mode=mode, stdout=out)
        elapsed = time.perf_counter() - start
        results[mode] = elapsed
    print(
        f"{label:32}  vm={results['vm'] * 1000 / iterations:7.3f} ms/iter   "
        f"jit={results['jit'] * 1000 / iterations:7.3f} ms/iter"
    )
    return results


def main() -> int:
    measure(
        "vec_push_pop_1000",
        """
        let v = vec_new()
        let i = 0
        while i < 1000 {
          let _ = push(v, i)
          i = i + 1
        }
        while len(v) > 0 {
          let _ = pop(v)
        }
        print len(v)
        """,
    )

    measure(
        "map_set_get_100",
        """
        let m = map_new()
        let i = 0
        while i < 100 {
          let _ = map_set(m, i, i * 2)
          i = i + 1
        }
        let total = 0
        let j = 0
        while j < 100 {
          total = total + map_get(m, j)
          j = j + 1
        }
        print total
        """,
    )

    measure(
        "str_concat_100",
        """
        let s = "x"
        let i = 0
        while i < 100 {
          s = str_concat(s, "y")
          i = i + 1
        }
        print str_char_len(s)
        """,
    )

    measure(
        "typed_add_loop_1000",
        """
        let total = 0
        let i = 0
        while i < 1000 {
          total = typed_add(total, 1, "i32")
          i = i + 1
        }
        print total
        """,
    )

    measure(
        "result_unwrap_loop_500",
        """
        let i = 0
        let total = 0
        while i < 500 {
          let r = result_ok(i)
          total = total + result_unwrap(r)
          i = i + 1
        }
        print total
        """,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
