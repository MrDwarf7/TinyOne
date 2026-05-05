# TinyOne Tests

Run correctness tests:

```sh
python3 -m unittest discover -s Tests -p 'test_*.py'
```

Run the benchmark suite:

```sh
python3 Tests/bench_vm_jit.py
```

For a fast smoke run:

```sh
python3 Tests/bench_vm_jit.py --quick --repeats 1
```

The correctness suite pins VM/JIT parity across straight-line code, loop
dispatch, function call/return dispatch, nested control-flow transfers, runtime
errors, memory slot behavior, heap arrays, structs, strings, buffers, pointer
cells, raw pointers, null checks, pointer metadata, stale pointer rejection,
deterministic input, namespaced imports, export visibility, package manifest
resolution, import/artifact round trips, diagnostics, lexical scopes, JIT
caching, and verifier failures.

The benchmark runner measures memory allocation/load/store/reset/snapshot,
lexer/compiler/optimizer/verifier stages, program fingerprinting, cold and hot
JIT compilation, VM execution, JIT execution, full compile-and-run APIs,
function calls, heap/struct workloads, input-backed standard-library calls, and
control-transfer opcodes. TinyOne has no external OS-style interrupt subsystem;
the `control_interrupts` benchmarks stress the runtime's interrupt-like
bytecode transfers: `JUMP`, `JUMP_IF_ZERO`, `CALL`, `RETURN`, and `HALT`.
