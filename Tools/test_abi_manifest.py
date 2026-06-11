import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
TOOL = ROOT / "Tools" / "abi_manifest.py"


class AbiManifestToolTests(unittest.TestCase):
    def run_tool(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(TOOL), *args],
            cwd=ROOT,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )

    def test_manifest_lists_symbols_from_rust_and_header(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            ffi = Path(tmp) / "ffi.rs"
            header = Path(tmp) / "tinyone.h"
            ffi.write_text(
                textwrap.dedent(
                    """
                    #[unsafe(no_mangle)]
                    pub unsafe extern "C" fn tinyone_beta(value: *const c_char) -> *mut c_char {
                        todo!()
                    }

                    #[unsafe(no_mangle)]
                    pub extern "C" fn tinyone_alpha() {
                        todo!()
                    }
                    """
                ),
                encoding="utf-8",
            )
            header.write_text(
                textwrap.dedent(
                    """
                    #ifndef TINYONE_H
                    #define TINYONE_H
                    void tinyone_alpha(void);
                    char *tinyone_beta(const char *value);
                    #endif
                    """
                ),
                encoding="utf-8",
            )

            result = self.run_tool(
                "manifest",
                "--ffi",
                str(ffi),
                "--header",
                str(header),
            )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("rust_symbols:", result.stdout)
        self.assertIn("  - tinyone_alpha", result.stdout)
        self.assertIn("  - tinyone_beta", result.stdout)
        self.assertIn("header_symbols:", result.stdout)

    def test_check_reports_header_drift(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            ffi = Path(tmp) / "ffi.rs"
            header = Path(tmp) / "tinyone.h"
            ffi.write_text(
                textwrap.dedent(
                    """
                    #[unsafe(no_mangle)]
                    pub unsafe extern "C" fn tinyone_only_in_rust() {
                        todo!()
                    }
                    """
                ),
                encoding="utf-8",
            )
            header.write_text("void tinyone_only_in_header(void);\n", encoding="utf-8")

            result = self.run_tool(
                "check",
                "--ffi",
                str(ffi),
                "--header",
                str(header),
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing from header", result.stdout)
        self.assertIn("tinyone_only_in_rust", result.stdout)
        self.assertIn("missing from Rust exports", result.stdout)
        self.assertIn("tinyone_only_in_header", result.stdout)

    def test_generate_header_requires_tinylang_crate_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            crate = Path(tmp) / "crate"
            crate.mkdir()
            (crate / "Cargo.toml").write_text(
                textwrap.dedent(
                    """
                    [package]
                    name = "wrong-name"
                    version = "0.1.0"
                    edition = "2024"
                    """
                ),
                encoding="utf-8",
            )

            result = self.run_tool(
                "generate-header",
                "--crate-dir",
                str(crate),
                "--cbindgen",
                sys.executable,
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Cargo.toml", result.stderr)
        self.assertIn("tinylang", result.stderr)

    def test_generate_header_checks_crate_and_passes_ffi_source_to_cbindgen(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            crate = base / "TinyOne"
            source_dir = crate / "src"
            source_dir.mkdir(parents=True)
            ffi_source = source_dir / "ffi.rs"
            output = base / "tinylang.h"
            argv_file = base / "argv.txt"
            fake_cbindgen = base / "fake-cbindgen.py"
            (crate / "Cargo.toml").write_text(
                textwrap.dedent(
                    """
                    [package]
                    name = "tinylang"
                    version = "0.6.0"
                    edition = "2024"
                    """
                ),
                encoding="utf-8",
            )
            ffi_source.write_text(
                '#[unsafe(no_mangle)]\npub extern "C" fn tinyone_smoke() {}\n',
                encoding="utf-8",
            )
            fake_cbindgen.write_text(
                f"#!{sys.executable}\n"
                "import sys\n"
                "from pathlib import Path\n"
                f"Path({str(argv_file)!r}).write_text("
                '"\\n".join(sys.argv[1:]), encoding="utf-8")\n',
                encoding="utf-8",
            )
            fake_cbindgen.chmod(0o755)

            result = self.run_tool(
                "generate-header",
                "--crate-dir",
                str(crate),
                "--ffi-source",
                str(ffi_source),
                "--output",
                str(output),
                "--cbindgen",
                str(fake_cbindgen),
            )
            argv = argv_file.read_text(encoding="utf-8").splitlines()

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("--crate", argv)
        self.assertIn("--config", argv)
        self.assertIn(str(ROOT / "cbindgen.toml"), argv)
        self.assertNotIn(str(crate), argv)
        self.assertIn(str(ffi_source), argv)
        self.assertIn("--output", argv)
        self.assertIn(str(output), argv)

    def test_generate_header_reports_missing_cbindgen_config(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            crate = base / "TinyOne"
            crate.mkdir()
            missing_config = base / "missing-cbindgen.toml"
            (crate / "Cargo.toml").write_text(
                textwrap.dedent(
                    """
                    [package]
                    name = "tinylang"
                    version = "0.6.0"
                    edition = "2024"
                    """
                ),
                encoding="utf-8",
            )

            result = self.run_tool(
                "generate-header",
                "--crate-dir",
                str(crate),
                "--config",
                str(missing_config),
                "--cbindgen",
                sys.executable,
            )

        self.assertEqual(result.returncode, 2)
        self.assertIn("cbindgen config does not exist", result.stderr)

    def test_generate_header_reports_missing_ffi_source(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            crate = base / "TinyOne"
            crate.mkdir()
            missing_source = base / "missing-ffi.rs"
            (crate / "Cargo.toml").write_text(
                textwrap.dedent(
                    """
                    [package]
                    name = "tinylang"
                    version = "0.6.0"
                    edition = "2024"
                    """
                ),
                encoding="utf-8",
            )

            result = self.run_tool(
                "generate-header",
                "--crate-dir",
                str(crate),
                "--ffi-source",
                str(missing_source),
                "--cbindgen",
                sys.executable,
            )

        self.assertEqual(result.returncode, 2)
        self.assertIn("cbindgen source does not exist", result.stderr)


if __name__ == "__main__":
    unittest.main()
