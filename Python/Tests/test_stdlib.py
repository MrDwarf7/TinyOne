#!/usr/bin/env python3
"""Parity tests for the Phase 2 stdlib bridge layer.

Each test runs the same TinyOne source through the Python VM and JIT
backends and asserts identical stdout. Where Rust and Python disagree, this
suite must catch it before runtime artifacts diverge.
"""
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
    BUILTINS_PUBLIC,
    CompileError,
    JitCache,
    Program,
    RuntimeTinyOneError,
    TinyMemory,
    TinyRuntimeContext,
    VM,
    compile_file,
    compile_source,
    run_program as run_compiled_program,
    typing_promote,
    typing_smallest_fit,
)


def run_modes(
    source: str,
    *,
    inputs=None,
    sys_args=None,
    sys_env=None,
) -> tuple[str, str]:
    program = compile_source(source)
    vm_out = StringIO()
    run_compiled_program(
        program,
        mode="vm",
        stdout=vm_out,
        inputs=inputs,
        sys_args=sys_args,
        sys_env=sys_env,
    )
    jit_out = StringIO()
    run_compiled_program(
        program,
        mode="jit",
        stdout=jit_out,
        inputs=inputs,
        sys_args=sys_args,
        sys_env=sys_env,
    )
    return vm_out.getvalue(), jit_out.getvalue()


def assert_parity(self, source: str, expected: str, **kwargs) -> None:
    vm, jit = run_modes(source, **kwargs)
    self.assertEqual(vm, expected, "VM output mismatch")
    self.assertEqual(jit, expected, "JIT output mismatch")


class BuiltinParityTests(unittest.TestCase):
    """The Phase-1 builtin slots (push/pop included) must match Rust order."""

    def test_phase1_builtin_names_match_rust(self) -> None:
        # Authoritative names from Rust/src/builtins.rs::BUILTINS, slots 0..34.
        expected = [
            "len", "array", "alloc", "load", "store", "free",
            "read", "read_int", "read_str", "to_int",
            "ptr", "fieldptr", "ptr_addr", "ptr_at", "ptr_add",
            "ptr_load", "ptr_store", "ptr_type", "buffer",
            "is_null", "ptr_eq", "ptr_ne", "ptr_base", "ptr_offset",
            "ptr_kind", "ptr_field",
            "read8", "write8", "read16", "write16", "read32", "write32",
            "cast_ptr", "push", "pop",
        ]
        names = [b.name for b in BUILTINS_PUBLIC[:35]]
        self.assertEqual(names, expected)

    def test_push_pop_match_rust_behavior(self) -> None:
        source = """
        let v = []
        let ignored = push(v, 1)
        let ignored2 = push(v, 2)
        let ignored3 = push(v, 3)
        print len(v)
        print v[0]
        print pop(v)
        print len(v)
        """
        assert_parity(self, source, "3\n1\n3\n2\n")


class VecAndMapTests(unittest.TestCase):
    def test_vec_basic(self) -> None:
        source = """
        let v = vec_new()
        let _1 = push(v, 10)
        let _2 = push(v, 20)
        print len(v)
        print v[0]
        print pop(v)
        print vec_clear(v)
        print len(v)
        """
        assert_parity(self, source, "2\n10\n20\n0\n0\n")

    def test_map_basic(self) -> None:
        source = """
        let m = map_new()
        let _1 = map_set(m, 1, 100)
        let _2 = map_set(m, 1, 111)
        let _3 = map_set(m, 2, 200)
        print map_get(m, 1)
        print map_get(m, 2)
        print map_has(m, 1)
        print map_has(m, 9)
        print map_len(m)
        print map_del(m, 2)
        print map_len(m)
        """
        assert_parity(self, source, "111\n200\n1\n0\n2\n1\n1\n")

    def test_map_string_keys_use_content_equality(self) -> None:
        source = """
        let m = map_new()
        let _1 = map_set(m, "alpha", 1)
        let _2 = map_set(m, "alpha", 2)
        print map_len(m)
        print map_get(m, "alpha")
        """
        assert_parity(self, source, "1\n2\n")

    def test_map_get_missing_key_errors(self) -> None:
        program = compile_source(
            """
            let m = map_new()
            print map_get(m, 1)
            """
        )
        for mode in ("vm", "jit"):
            with self.subTest(mode=mode):
                with self.assertRaisesRegex(RuntimeTinyOneError, "missing key"):
                    out = StringIO()
                    run_compiled_program(program, mode=mode, stdout=out)


class IoTests(unittest.TestCase):
    def test_io_writeln_to_captured_stdout(self) -> None:
        source = """
        let _ = io_writeln(io_stdout(), "hello")
        let _2 = io_writeln(io_stdout(), "world")
        let s = io_capture_stdout()
        print s
        print str_byte_len(s)
        """
        assert_parity(self, source, "hello\nworld\n\n12\n")

    def test_io_write_to_stdin_errors(self) -> None:
        program = compile_source(
            """
            let _ = io_write(io_stdin(), "x")
            """
        )
        for mode in ("vm", "jit"):
            with self.subTest(mode=mode):
                with self.assertRaisesRegex(RuntimeTinyOneError, "cannot write to stdin"):
                    out = StringIO()
                    run_compiled_program(program, mode=mode, stdout=out)


class StringTests(unittest.TestCase):
    def test_byte_vs_char_indexing(self) -> None:
        source = """
        let text = "héllo"
        print str_byte_len(text)
        print str_char_len(text)
        print str_byte_at(text, 0)
        print str_char_at(text, 1)
        print str_slice(text, 1, 4)
        print str_is_utf8(text)
        """
        assert_parity(self, source, "6\n5\n104\né\néll\n1\n")

    def test_invalid_utf8_buffer_detected(self) -> None:
        source = """
        let mem = buffer(2)
        let p = ptr(mem, 0)
        let _ = unsafe write8(p, 255)
        let _2 = unsafe write8(unsafe ptr_add(p, 1), 254)
        print str_is_utf8(mem)
        """
        assert_parity(self, source, "0\n")

    def test_str_from_buffer_round_trips(self) -> None:
        source = """
        let mem = buffer(3)
        let p = ptr(mem, 0)
        let _ = unsafe write8(p, 65)
        let _2 = unsafe write8(unsafe ptr_add(p, 1), 66)
        let _3 = unsafe write8(unsafe ptr_add(p, 2), 67)
        print str_from_buffer(mem)
        """
        assert_parity(self, source, "ABC\n")


class SyncTests(unittest.TestCase):
    def test_mutex_lock_unlock(self) -> None:
        source = """
        let m = mutex_new()
        print mutex_lock(m)
        print mutex_unlock(m)
        print mutex_lock(m)
        print mutex_unlock(m)
        """
        assert_parity(self, source, "1\n0\n1\n0\n")

    def test_mutex_double_lock_errors(self) -> None:
        program = compile_source(
            """
            let m = mutex_new()
            let _ = mutex_lock(m)
            let _2 = mutex_lock(m)
            """
        )
        for mode in ("vm", "jit"):
            with self.subTest(mode=mode):
                with self.assertRaisesRegex(RuntimeTinyOneError, "deadlock"):
                    out = StringIO()
                    run_compiled_program(program, mode=mode, stdout=out)

    def test_atomic_add_overflows(self) -> None:
        program = compile_source(
            """
            let a = atomic_new(9223372036854775807)
            let _ = atomic_add(a, 1)
            """
        )
        for mode in ("vm", "jit"):
            with self.subTest(mode=mode):
                with self.assertRaisesRegex(RuntimeTinyOneError, "Memory_Overflow"):
                    out = StringIO()
                    run_compiled_program(program, mode=mode, stdout=out)


class ResultOptionTests(unittest.TestCase):
    def test_result_round_trip(self) -> None:
        source = """
        let r = result_ok(42)
        print result_is_ok(r)
        print result_is_err(r)
        print result_unwrap(r)
        let e = result_err(99)
        print result_unwrap_err(e)
        """
        assert_parity(self, source, "1\n0\n42\n99\n")

    def test_option_unwrap_on_none_errors(self) -> None:
        program = compile_source(
            """
            let o = option_none()
            print option_unwrap(o)
            """
        )
        for mode in ("vm", "jit"):
            with self.subTest(mode=mode):
                with self.assertRaisesRegex(RuntimeTinyOneError, "None"):
                    out = StringIO()
                    run_compiled_program(program, mode=mode, stdout=out)


class SysIntrospectionTests(unittest.TestCase):
    def test_sys_args_and_env_are_deterministic(self) -> None:
        source = """
        print sys_argc()
        print sys_argv(0)
        print sys_argv(1)
        print sys_env_has("FOO")
        print sys_env_get("FOO")
        print sys_env_has("MISSING")
        """
        vm_out, jit_out = run_modes(
            source,
            sys_args=["program", "alpha"],
            sys_env={"FOO": "bar"},
        )
        self.assertEqual(vm_out, "2\nprogram\nalpha\n1\nbar\n0\n")
        self.assertEqual(vm_out, jit_out)


class PathFsTests(unittest.TestCase):
    def test_path_helpers(self) -> None:
        source = """
        print path_join("/tmp", "x")
        print path_join("/tmp/", "x")
        print path_join("", "x")
        print path_join("/tmp", "/abs")
        print path_basename("/a/b.txt")
        print path_dirname("/a/b.txt")
        """
        assert_parity(
            self,
            source,
            "/tmp/x\n/tmp/x\nx\n/abs\nb.txt\n/a\n",
        )

    def test_fs_round_trip(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            target = Path(tmp) / "hello.txt"
            target_lit = f'"{target}"'
            source = (
                'let mem = buffer(4)\n'
                'let p = ptr(mem, 0)\n'
                'let _ = unsafe write8(p, 104)\n'
                'let _1 = unsafe write8(unsafe ptr_add(p, 1), 105)\n'
                'let _2 = unsafe write8(unsafe ptr_add(p, 2), 33)\n'
                'let _3 = unsafe write8(unsafe ptr_add(p, 3), 10)\n'
                f'let _4 = unsafe fs_write({target_lit}, mem)\n'
                f'let body = unsafe fs_read({target_lit})\n'
                'print str_from_buffer(body)\n'
                f'print fs_exists({target_lit})\n'
            )
            expected = "hi!\n\n1\n"
            assert_parity(self, source, expected)


class MathLogicTests(unittest.TestCase):
    def test_math_constants_and_helpers(self) -> None:
        source = """
        print math_const("PI_THOUSANDTHS")
        print math_const("E_THOUSANDTHS")
        print math_const("TAU_THOUSANDTHS")
        print math_const("MAX_I64")
        print math_const("MIN_I64")
        print math_abs(-42)
        print math_min(1, 2)
        print math_max(1, 2)
        """
        assert_parity(
            self,
            source,
            "3142\n2718\n6283\n9223372036854775807\n-9223372036854775808\n42\n1\n2\n",
        )

    def test_logic_helpers(self) -> None:
        source = """
        print logic_and(1, 1)
        print logic_and(0, 1)
        print logic_or(0, 1)
        print logic_or(0, 0)
        print logic_not(0)
        print logic_not(1)
        print logic_xor(1, 0)
        print logic_xor(0, 0)
        """
        assert_parity(self, source, "1\n0\n1\n0\n1\n0\n1\n0\n")


class TypingSystemTests(unittest.TestCase):
    def test_smallest_fit_matches_rust(self) -> None:
        cases = {
            0: "u8",
            16: "u8",
            255: "u8",
            256: "u16",
            65_535: "u16",
            65_536: "u32",
            -1: "i8",
            -128: "i8",
            -129: "i16",
        }
        for value, expected in cases.items():
            with self.subTest(value=value):
                self.assertEqual(typing_smallest_fit(value), expected)

    def test_promotion_matches_spec(self) -> None:
        # From typing_system.md and phase_2.md examples.
        self.assertEqual(typing_promote("i8", "u8"), "i16")
        self.assertEqual(typing_promote("u8", "u16"), "u32")
        self.assertEqual(typing_promote("i32", "i64"), "i64")
        self.assertEqual(typing_promote("u8", "u8"), "u8")

    def test_typed_arithmetic_runtime(self) -> None:
        source = """
        print typed_add(100, 28, "u8")
        print typed_add(1, 2, "i32")
        print typed_sub(0, 1, "i8")
        print typed_mul(10, 20, "u16")
        print typed_div(100, 3, "i32")
        print typed_neg(7, "i8")
        """
        assert_parity(self, source, "128\n3\n-1\n200\n33\n-7\n")

    def test_typed_add_overflow_errors(self) -> None:
        program = compile_source('print typed_add(200, 100, "u8")')
        for mode in ("vm", "jit"):
            with self.subTest(mode=mode):
                with self.assertRaisesRegex(RuntimeTinyOneError, "Memory_Overflow"):
                    out = StringIO()
                    run_compiled_program(program, mode=mode, stdout=out)

    def test_type_of_recognizes_runtime_shapes(self) -> None:
        source = """
        print type_of(0)
        print type_of("abc")
        print type_of([1, 2])
        print type_of(map_new())
        print type_of(result_ok(1))
        print type_of(option_none())
        print type_of(mutex_new())
        print type_of(atomic_new(0))
        print type_of(null)
        """
        assert_parity(
            self,
            source,
            "i64\nString\nVec\nMap\nResult\nOption\nMutex\nAtomic\nNull\n",
        )

    def test_type_id_matches_documented_values(self) -> None:
        source = """
        print type_id("i8")
        print type_id("i16")
        print type_id("i32")
        print type_id("i64")
        print type_id("u8")
        print type_id("u16")
        print type_id("u32")
        print type_id("u64")
        print type_id("bool")
        print type_id("String")
        print type_id("Vec")
        """
        assert_parity(self, source, "2\n3\n4\n5\n6\n7\n8\n9\n1\n15\n18\n")


class StdlibModuleImportTests(unittest.TestCase):
    def test_compile_and_run_via_manifest_import(self) -> None:
        stdlib_root = ROOT / "stdlib"
        self.assertTrue((stdlib_root / "tinyone.json").exists())
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            manifest = {
                "package": "app",
                "modules": {
                    "vec": str(stdlib_root / "vec.to"),
                    "map": str(stdlib_root / "map.to"),
                    "math": str(stdlib_root / "math.to"),
                    "logic": str(stdlib_root / "logic.to"),
                    "result": str(stdlib_root / "result.to"),
                    "option": str(stdlib_root / "option.to"),
                    "typing": str(stdlib_root / "typing.to"),
                },
            }
            import json as _json
            (tmp_path / "tinyone.json").write_text(_json.dumps(manifest), encoding="utf-8")
            main = tmp_path / "main.to"
            main.write_text(
                """
                import "vec" as v
                import "map" as m
                import "math" as math
                import "logic" as l
                import "result" as r
                import "option" as o
                import "typing" as t

                let xs = v.new()
                let _ = v.append(xs, 7)
                let _2 = v.append(xs, 8)
                print v.size(xs)

                let d = m.new()
                let _3 = m.put(d, "k", 41)
                print m.get(d, "k")

                print math.abs(-9)
                print l.xor(1, 0)
                print t.add(1, 2, "u8")
                print r.unwrap(r.ok(11))
                print o.unwrap(o.some(22))
                """,
                encoding="utf-8",
            )
            program = compile_file(main)
            stdout = StringIO()
            run_compiled_program(program, mode="vm", stdout=stdout)
            self.assertEqual(stdout.getvalue(), "2\n41\n9\n1\n3\n11\n22\n")


if __name__ == "__main__":
    unittest.main()
