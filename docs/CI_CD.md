---
title: CI/CD
---

# CI/CD

> Native preference: version pins live in native files (`rust-toolchain.toml`, `.python-version`), not
> duplicated in shell. Cargo already exports `CARGO_TARGET_DIR` -- use it directly; do not redefine
> `TARGET_DIR` as a second source of truth.

## Toolchain pinning

| Tool        | Pin file                                                             | Composite input                                      | Bump procedure                                                                                                    |
| ----------- | -------------------------------------------------------------------- | ---------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| Rust        | `rust-toolchain.toml` (`channel = "stable"` + `components`)          | `setup-rust` `inputs.toolchain` (default reads file) | Edit `rust-toolchain.toml`, update `docs/CI_CD.md` table, verify `cargo make ci-gate` locally                     |
| Python      | `.python-version` (`3.12`)                                           | `setup-python` `inputs.python-version`               | Edit `.python-version` + `build/ci-tasks.toml` `[env] PYTHON_VERSION`, verify `uv run`                            |
| uv          | `setup-python` `inputs.uv-version` (`latest`)                        | `setup-python`                                       | Bump in `setup-python/action.yml` default                                                                         |
| GCC/Clang   | FFI workflow leaves `setup-gcc` inputs empty (runner defaults)       | `setup-gcc`                                          | Set explicit workflow inputs and update this doc only when a compiler pin is required                             |
| mold        | `setup-rust` `MOLD_VERSION='2.41.0'` (Linux)                         | `setup-rust` `inputs.use-mold`                       | Bump in `setup-rust/action.yml`                                                                                   |
| cargo tools | `setup-rust` `inputs.tools` (`cargo-binstall,cargo-make,just,taplo`) | `setup-rust`                                         | Edit `setup-rust` `inputs.tools` default                                                                          |

**Rules for agents:**

- Do not add `env:` version lists to composite actions -- use `inputs` with defaults (tested: `env` is disallowed in action files).
- `nightly.yml` must `rm -f rust-toolchain.toml` before `setup-rust` so `inputs.toolchain = nightly` takes precedence.
- `cranelift.yml` is isolated for the same reason (codegen backend affects language performance).
- C compiler versions affect ABI consumer artifacts. The current FFI matrix records
  runner-default versions; if a pin is required, set explicit workflow inputs and
  document the approved version.

## Paths

- `Makefile.toml` defines `ROOT` (shorthand for `CARGO_MAKE_WORKSPACE_WORKING_DIRECTORY` -- 15+ chars, comment explains), `SCRIPT_DIR = "${ROOT}/scripts"`, `TOOL_DIR = "${ROOT}/tools"`.
- `CARGO_TARGET_DIR` is cargo-native; extend files must not re-define it.
- `build/ci-tasks.toml` owns ci gates; `build/ffi-tasks.toml` owns ffi (strict split, no bleed).
- Root `Makefile.toml` is alias-only: `ma f` -> `format`, etc. Extend files declare local `env` only if strictly file-scoped.

## Scripts

- `scripts/consumer_compile.sh`, `consumer_smoke.sh`, `consumer_smoke_msvc.sh`, and `check_abi_drift.sh` are `#!/usr/bin/env bash` helpers that use `CARGO_TARGET_DIR` and fail hard for their validation work; diagnostic probes may tolerate missing output while reporting it.
- Legacy one-line wrappers are not part of this workspace; call `cargo make` directly.
- Workflow Cargo and consumer steps use `shell: bash` on all runners, including
  `windows-latest`. The Windows MSVC setup action uses `pwsh` only to adjust the
  Windows `PATH` before its Bash consumer steps.

## Workflows

- `format.yml` -- fmt+taplo check (matrix ubuntu/windows), auto-fix push on `push` to `main`/`master`.
- `build.yml` -- `check` + `clippy -D warnings` matrix.
- `test.yml` -- `cargo test --workspace` + `language_suite` + python tool tests + abi drift.
- `ffi.yml` -- cdylib build + consumer_compile + consumer_smoke (3 OS) + recorded runner-default C/C++ versions.
- `nightly.yml` -- isolated nightly validation.
- `cranelift.yml` -- installs nightly plus `rustc-codegen-cranelift` explicitly
  and runs a direct `-Zcodegen-backend=cranelift` probe before building and
  testing with that backend on Windows and Linux.

## CODEOWNERS

- `* @ConnerAdamsMaine @mrdwarf7` (owner first)
- `/docs/` and `*.md` -> `@ConnerAdamsMaine`
- `/build/` and `/Makefile.toml` -> `@mrdwarf7` (intentional friction, owner can still override but review required)
- See `.github/CODEOWNERS` for full patterns (last match wins).

## Local

```bash
ma fmt          # format
ma ci-gate      # full gate (no script wrapper)
ma ci-gate-fast # check+test
ma ffi-consumer-compile
```
