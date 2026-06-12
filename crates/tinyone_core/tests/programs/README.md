# TinyOne compliance tests

These `.to` files are derived from the provided TinyOne cheatsheets.

## Layout

- `pass/`: programs expected to compile and exit `0`.
- `fail_compile/`: programs expected to fail compile/check.
- `fail_runtime/`: programs expected to compile but exit `1` at runtime.
- `modules/`: import/export fixtures.

## Suggested commands

```sh
# Passing tests, VM and JIT should agree.
for f in pass/*.to; do tinylang --mode vm "$f"; done
for f in pass/*.to; do tinylang --mode jit "$f"; done

# Input fixture: expected inputs for pass/015_input_builtins.to
# First read() should preserve int coercion for numeric input.
tinylang --mode vm --input 12 --input 34 --input hello pass/015_input_builtins.to

# Module tests.
(cd modules && tinylang --mode vm main_import_file.to)
(cd modules && tinylang --mode vm main_import_manifest.to)

# Compile-fail tests.
for f in fail_compile/*.to; do tinylang --check "$f" && echo "UNEXPECTED PASS: $f"; done

# Runtime-fail tests.
for f in fail_runtime/*.to; do tinylang --mode vm "$f" && echo "UNEXPECTED PASS: $f"; done

# Runtime-fail input-specific tests.
tinylang --mode vm fail_runtime/008_input_exhaustion.to
tinylang --mode vm --input abc fail_runtime/009_read_int_requires_numeric.to
```
