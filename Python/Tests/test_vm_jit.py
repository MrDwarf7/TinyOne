#!/usr/bin/env python3
from __future__ import annotations

import sys
import tempfile
import unittest
from io import StringIO
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from Python.main import (  # noqa: E402
    BytecodeVerifier,
    CompileError,
    Function,
    Instr,
    JitCache,
    Op,
    Program,
    RuntimeTinyOneError,
    TinyMemory,
    VM,
    compile_file,
    compile_source,
    load_artifact,
    run_program as run_compiled_program,
    write_artifact,
)


def run_program(program: Program, mode: str) -> tuple[str, tuple[int, ...]]:
    stdout = StringIO()
    memory = TinyMemory(program.slot_count)
    if mode == "vm":
        VM(program, memory, stdout).run()
    elif mode == "jit":
        JitCache().compile(program)(memory, stdout)
    else:
        raise AssertionError(f"unknown mode {mode!r}")
    return stdout.getvalue(), memory.snapshot()


class RuntimeParityTests(unittest.TestCase):
    def assert_backends_match(self, source: str, expected_stdout: str) -> Program:
        program = compile_source(source)
        vm_result = run_program(program, "vm")
        jit_result = run_program(program, "jit")
        self.assertEqual((expected_stdout, vm_result[1]), vm_result)
        self.assertEqual(vm_result, jit_result)
        return program

    def test_straightline_jit_matches_vm(self) -> None:
        source = """
        let a = 4
        let b = a * 5 + (6 - 2)
        let c = b / 3
        print b
        print c
        print b >= 24
        print c != 8
        """

        program = self.assert_backends_match(source, "24\n8\n1\n0\n")
        generated = JitCache()._build_source(program)

        self.assertIn("_s0", generated)
        self.assertNotIn("while True:", generated)
        self.assertTrue(JitCache._can_emit_straightline(program))

    def test_dispatch_loop_jit_matches_vm(self) -> None:
        source = """
        let i = 0
        let total = 0
        while i < 32 {
          let total = total + (i * 3)
          let i = i + 1
        }
        print total
        print i == 32
        """

        program = self.assert_backends_match(source, "1488\n1\n")
        generated = JitCache()._build_source(program)

        self.assertIn("while True:", generated)
        self.assertIn("pc =", generated)
        self.assertFalse(JitCache._can_emit_straightline(program))

    def test_dispatch_call_return_jit_matches_vm(self) -> None:
        source = """
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
        while i <= 8 {
          let total = total + pair(i)
          let i = i + 1
        }
        print total
        """

        program = self.assert_backends_match(source, "204\n")
        generated = JitCache()._build_source(program)

        self.assertIn("_tinyone_call", generated)
        self.assertIn("return stack.pop()", generated)
        self.assertFalse(JitCache._can_emit_straightline(program))

    def test_nested_control_flow_transfers_match_vm(self) -> None:
        source = """
        let i = 0
        let marker = 1
        let trips = 0
        while i < 10 {
          let gate = 1
          while gate {
            let trips = trips + marker
            let gate = 0
          }
          let marker = marker + 1
          let i = i + 1
        }
        print trips
        """

        self.assert_backends_match(source, "55\n")

    def test_runtime_division_errors_match(self) -> None:
        program = compile_source(
            """
            let zero = 0
            print 12 / zero
            """
        )

        for mode in ("vm", "jit"):
            with self.subTest(mode=mode):
                with self.assertRaisesRegex(RuntimeTinyOneError, "Division by zero"):
                    run_program(program, mode)

    def test_jit_cache_reuses_compiled_function(self) -> None:
        program = compile_source("let x = 40 + 2 print x")
        cache = JitCache()

        first = cache.compile(program)
        second = cache.compile(program)

        self.assertIs(first, second)

    def test_heap_arrays_structs_strings_and_fields_match(self) -> None:
        source = """
        struct Point { x, y }
        let values = [10, 20, 30]
        set values[1] = 99
        let p = Point(values[1], len(values))
        set p.y = p.y + 1
        let msg = "hi"
        print msg
        print values
        print values[1]
        print p.x
        print p.y
        print len(msg)
        print msg[1]
        """

        self.assert_backends_match(source, "hi\n[10, 99, 30]\n99\n99\n4\n2\ni\n")

    def test_pointer_cells_and_deterministic_input(self) -> None:
        source = """
        let start = read_int()
        let ptr = alloc(start)
        print load(ptr)
        print store(ptr, load(ptr) + 5)
        print load(ptr)
        let done = free(ptr)
        """
        program = compile_source(source)

        for mode in ("vm", "jit"):
            with self.subTest(mode=mode):
                stdout = StringIO()
                memory = run_compiled_program(
                    program,
                    mode=mode,
                    stdout=stdout,
                    inputs=["37"],
                )
                self.assertEqual(stdout.getvalue(), "37\n42\n42\n")
                self.assertEqual(len(memory.snapshot()), 3)

    def test_raw_pointers_require_unsafe_and_match_backends(self) -> None:
        source = """
        struct Pair { left, right }
        let values = [10, 20, 30]
        let second = ptr(values, 1)
        print ptr_addr(second)
        print ptr_type(second)
        print unsafe ptr_load(second)
        print unsafe ptr_store(unsafe ptr_add(second, 1), 77)
        print values[2]
        let pair = Pair(4, 5)
        let field = fieldptr(pair, "right")
        print unsafe ptr_load(field)
        print unsafe ptr_store(field, 99)
        print pair.right
        let cell = alloc(12)
        let raw = ptr(cell)
        print unsafe ptr_load(raw)
        print unsafe ptr_store(raw, 13)
        print load(cell)
        let ptr_cell = alloc(second)
        print ptr_type(load(ptr_cell))
        """

        self.assert_backends_match(source, "1\narray\n20\n77\n77\n5\n99\n99\n12\n13\n13\narray\n")

        with self.assertRaisesRegex(CompileError, "requires unsafe"):
            compile_source("let values = [1] let p = ptr(values, 0) print ptr_load(p)")

    def test_raw_pointer_address_arithmetic_checks_runtime_bounds(self) -> None:
        program = compile_source(
            """
            let values = [1]
            let p = ptr(values, 0)
            print unsafe ptr_load(unsafe ptr_add(p, 2))
            """
        )

        for mode in ("vm", "jit"):
            with self.subTest(mode=mode):
                with self.assertRaisesRegex(RuntimeTinyOneError, "out of bounds"):
                    run_program(program, mode)

    def test_null_metadata_buffers_and_sized_memory_match_backends(self) -> None:
        source = """
        struct Pair { left, right }
        let nothing = null
        print is_null(nothing)

        let values = [1, 2]
        let item = ptr(values, 1)
        print ptr_kind(item)
        print ptr_offset(item)
        set values[1] = 9
        print unsafe ptr_load(item)

        let pair = Pair(4, 5)
        let field = fieldptr(pair, "right")
        print ptr_kind(field)
        print ptr_field(field)
        set pair.right = 11
        print unsafe ptr_load(field)

        let mem = buffer(16)
        let p = ptr(mem, 0)
        print is_null(p)
        print ptr_eq(p, ptr(mem, 0))
        print ptr_ne(p, nothing)
        print ptr_base(p) > 0
        print ptr_offset(unsafe ptr_add(p, 3))
        print ptr_kind(p)
        print len(ptr_field(p))
        print ptr_type(cast_ptr(p, "i32"))
        print ptr_eq(cast_ptr(p, "i32"), p)
        print unsafe read8(p)
        print unsafe write8(unsafe ptr_add(p, 1), 255)
        print unsafe read8(unsafe ptr_add(p, 1))
        print unsafe write16(unsafe ptr_add(p, 2), 4660)
        print unsafe read8(unsafe ptr_add(p, 2))
        print unsafe read8(unsafe ptr_add(p, 3))
        print unsafe read16(unsafe ptr_add(p, 2))
        print unsafe write32(unsafe ptr_add(p, 4), 305419896)
        print unsafe read32(unsafe ptr_add(p, 4))
        """

        self.assert_backends_match(
            source,
            (
                "1\narray\n1\n9\nfield\nright\n11\n0\n1\n1\n1\n3\n"
                "buffer\n0\ni32\n1\n0\n255\n255\n4660\n52\n18\n4660\n"
                "305419896\n305419896\n"
            ),
        )

    def test_raw_memory_operations_require_unsafe_and_check_bounds(self) -> None:
        with self.assertRaisesRegex(CompileError, "requires unsafe"):
            compile_source("let mem = buffer(1) let p = ptr(mem, 0) print read8(p)")

        programs = [
            compile_source("let mem = buffer(1) let p = ptr(mem, 0) print unsafe read16(p)"),
            compile_source("let mem = buffer(1) let p = ptr(mem, 0) print unsafe write8(p, 256)"),
        ]

        for program in programs:
            for mode in ("vm", "jit"):
                with self.subTest(program=program.fingerprint, mode=mode):
                    with self.assertRaisesRegex(RuntimeTinyOneError, "out of bounds|range"):
                        run_program(program, mode)

    def test_derived_pointers_fail_after_base_free_even_if_address_is_reused(self) -> None:
        programs = {
            "array": compile_source(
                """
                let values = [1, 2]
                let p = ptr(values, 1)
                let ignored = free(values)
                let replacement = [7, 8]
                print unsafe ptr_load(p)
                """
            ),
            "array_metadata": compile_source(
                """
                let values = [1, 2]
                let p = ptr(values, 1)
                let ignored = free(values)
                let replacement = [7, 8]
                print ptr_kind(p)
                """
            ),
            "field": compile_source(
                """
                struct Pair { left, right }
                let pair = Pair(1, 2)
                let p = fieldptr(pair, "right")
                let ignored = free(pair)
                let replacement = Pair(3, 4)
                print unsafe ptr_load(p)
                """
            ),
        }

        for name, program in programs.items():
            for mode in ("vm", "jit"):
                with self.subTest(name=name, mode=mode):
                    with self.assertRaisesRegex(
                        RuntimeTinyOneError,
                        "Stale heap pointer|Use after free",
                    ):
                        run_program(program, mode)

    def test_imports_and_artifact_roundtrip(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "pairs.to").write_text(
                """
                fn hidden(p) {
                  return p.left + p.right + 1000
                }

                export struct Pair { left, right }

                export fn sum_pair(p) {
                  return p.left + p.right
                }
                """,
                encoding="utf-8",
            )
            main_path = root / "main.to"
            main_path.write_text(
                """
                import "pairs.to" as pairs
                let pair = pairs.Pair(18, 24)
                print pairs.sum_pair(pair)
                """,
                encoding="utf-8",
            )

            program = compile_file(main_path)
            self.assertEqual(len(program.modules), 1)
            self.assertEqual(program.modules[0].exported_functions, ("sum_pair",))
            self.assertEqual(program.modules[0].exported_structs, ("Pair",))
            artifact_path = root / "main.tobc.json"
            write_artifact(program, artifact_path)
            loaded = load_artifact(artifact_path)

            self.assertEqual(program.fingerprint, loaded.fingerprint)
            for mode in ("vm", "jit"):
                with self.subTest(mode=mode):
                    self.assertEqual(run_program(loaded, mode)[0], "42\n")

    def test_import_manifest_namespaces_and_export_visibility(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "tinyone.json").write_text(
                '{"package": "demo", "modules": {"math": "pkg/math.to"}}',
                encoding="utf-8",
            )
            (root / "pkg").mkdir()
            (root / "pkg" / "math.to").write_text(
                """
                fn hidden(value) {
                  return value + 100
                }

                fn sum_pair(p) {
                  return p.left + p.right + hidden(0)
                }

                export struct Pair { left, right }

                export fn exported_sum(p) {
                  return sum_pair(p)
                }
                """,
                encoding="utf-8",
            )
            main_path = root / "main.to"
            main_path.write_text(
                """
                import "math" as m
                let pair = m.Pair(10, 20)
                print m.exported_sum(pair)
                """,
                encoding="utf-8",
            )

            for mode in ("vm", "jit"):
                with self.subTest(mode=mode):
                    self.assertEqual(run_program(compile_file(main_path), mode)[0], "130\n")

            bad_unqualified = root / "bad_unqualified.to"
            bad_unqualified.write_text(
                """
                import "math" as m
                let pair = Pair(10, 20)
                print m.exported_sum(pair)
                """,
                encoding="utf-8",
            )
            with self.assertRaisesRegex(CompileError, "Undefined function or constructor 'Pair'"):
                compile_file(bad_unqualified)

            bad_private = root / "bad_private.to"
            bad_private.write_text(
                """
                import "math" as m
                print m.hidden(1)
                """,
                encoding="utf-8",
            )
            with self.assertRaisesRegex(CompileError, "not exported"):
                compile_file(bad_private)

    def test_block_scope_hides_loop_locals(self) -> None:
        with self.assertRaisesRegex(CompileError, "Undefined variable 'scoped'"):
            compile_source(
                """
                let i = 0
                while i < 1 {
                  let scoped = 9
                  let i = i + 1
                }
                print scoped
                """
            )

    def test_compile_diagnostics_include_line_column_and_span(self) -> None:
        with self.assertRaises(CompileError) as caught:
            compile_source("let x = 1\nprint missing\n")

        message = str(caught.exception)
        self.assertIn("<source>:2:7", message)
        self.assertIn("Undefined variable 'missing'", message)
        self.assertIn("^", message)


class MemoryTests(unittest.TestCase):
    def test_memory_allocation_reset_and_bounds(self) -> None:
        memory = TinyMemory(3)
        self.assertEqual(memory.snapshot(), (0, 0, 0))

        memory.store(1, 99)
        self.assertEqual(memory.load(1), 99)
        self.assertEqual(memory.snapshot(), (0, 99, 0))

        memory.reset()
        self.assertEqual(memory.snapshot(), (0, 0, 0))

        with self.assertRaisesRegex(RuntimeTinyOneError, "Invalid memory slot"):
            memory.load(3)
        with self.assertRaisesRegex(RuntimeTinyOneError, "Invalid memory slot"):
            memory.store(-1, 1)
        with self.assertRaisesRegex(ValueError, "slot_count"):
            TinyMemory(-1)


class VerifierTests(unittest.TestCase):
    def assert_verify_error(self, program: Program, message: str) -> None:
        with self.assertRaisesRegex(CompileError, message):
            BytecodeVerifier.verify(program)

    def test_verifier_rejects_stack_underflow_before_runtime(self) -> None:
        program = Program(
            code=(Instr(Op.PRINT), Instr(Op.HALT)),
            slot_count=0,
            names=(),
        )

        self.assert_verify_error(program, "stack underflow")

    def test_verifier_rejects_invalid_jump_target(self) -> None:
        program = Program(
            code=(
                Instr(Op.PUSH_INT, 1),
                Instr(Op.JUMP_IF_ZERO, 99),
                Instr(Op.HALT),
            ),
            slot_count=0,
            names=(),
        )

        self.assert_verify_error(program, "targets 99")

    def test_verifier_rejects_call_arity_mismatch(self) -> None:
        function = Function(
            name="id",
            param_count=1,
            code=(Instr(Op.LOAD, 0), Instr(Op.RETURN)),
            slot_count=1,
            names=("value",),
        )
        program = Program(
            code=(
                Instr(Op.PUSH_INT, 7),
                Instr(Op.CALL, 0, 0),
                Instr(Op.PRINT),
                Instr(Op.HALT),
            ),
            slot_count=0,
            names=(),
            functions=(function,),
        )

        self.assert_verify_error(program, "expects 1 argument")

    def test_verifier_rejects_invalid_slot(self) -> None:
        program = Program(
            code=(Instr(Op.LOAD, 2), Instr(Op.PRINT), Instr(Op.HALT)),
            slot_count=1,
            names=("only",),
        )

        self.assert_verify_error(program, "invalid slot 2")


if __name__ == "__main__":
    unittest.main()
