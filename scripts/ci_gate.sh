#!/usr/bin/env bash
set -uo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

MANIFEST="crates/tinyone_core/Cargo.toml"
TIMEOUT_SECONDS="${CI_GATE_TIMEOUT_SECONDS:-300}"

failures=()
skips=()

usage() {
  cat <<'USAGE'
usage: scripts/ci_gate.sh

Runs the repo-local TinyOne CI/release gate from the repository root.

Default behavior is honest: supported gates run and failures are reported.
Skip expensive or currently-known-broken gates explicitly with environment flags:

  CI_GATE_SKIP_CARGO_CHECK=1
  CI_GATE_SKIP_CARGO_TEST=1
  CI_GATE_SKIP_CARGO_FMT=1
  CI_GATE_SKIP_CLIPPY=1
  CI_GATE_SKIP_TESTING_HOOKS=1
  CI_GATE_SKIP_PYTHON_TOOLS=1
  CI_GATE_SKIP_ABI_DRIFT=1
  CI_GATE_SKIP_HASH_LOC_SMOKE=1
  CI_GATE_SKIP_RELEASE_BUILD=1
  CI_GATE_SKIP_BENCH_SMOKE=1

Optional timeout:

  CI_GATE_TIMEOUT_SECONDS=600   default: 300, set 0 to disable

This gate intentionally does not restore or require the removed root stdlib.
If tests still reference stdlib/tinyone.json, that is reported as a real failure.
USAGE
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

die() {
  printf 'ci_gate: error: %s\n' "$*" >&2
  exit 2
}

should_skip() {
  local var_name="$1"
  [[ "${!var_name:-}" == "1" || "${!var_name:-}" == "true" || "${!var_name:-}" == "yes" ]]
}

run_command() {
  if [[ "$TIMEOUT_SECONDS" == "0" ]]; then
    "$@"
    return $?
  fi

  if command -v timeout >/dev/null 2>&1; then
    timeout "$TIMEOUT_SECONDS" "$@"
    return $?
  fi

  "$@"
}

run_gate() {
  local name="$1"
  local skip_var="$2"
  shift 2

  if should_skip "$skip_var"; then
    printf '\n==> SKIP %s (%s)\n' "$name" "$skip_var"
    skips+=("$name skipped by $skip_var")
    return 0
  fi

  printf '\n==> RUN %s\n' "$name"
  printf '+'
  printf ' %q' "$@"
  printf '\n'

  run_command "$@"
  local status=$?
  if [[ $status -eq 0 ]]; then
    printf '==> PASS %s\n' "$name"
  else
    printf '==> FAIL %s (exit %s)\n' "$name" "$status" >&2
    failures+=("$name exited $status")
  fi
}

require_file() {
  [[ -f "$1" ]] || die "required file missing: $1"
}

require_file "$MANIFEST"
require_file "tools/hash.py"
require_file "tools/loc.py"
require_file "tools/abi_manifest.py"

run_gate "cargo check" "CI_GATE_SKIP_CARGO_CHECK" \
  cargo check --manifest-path "$MANIFEST"

run_gate "cargo test" "CI_GATE_SKIP_CARGO_TEST" \
  cargo test --manifest-path "$MANIFEST"

run_gate "cargo fmt --check" "CI_GATE_SKIP_CARGO_FMT" \
  cargo fmt --manifest-path "$MANIFEST" -- --check

run_gate "cargo clippy" "CI_GATE_SKIP_CLIPPY" \
  cargo clippy --manifest-path "$MANIFEST" --all-targets -- -D warnings

if should_skip "CI_GATE_SKIP_TESTING_HOOKS"; then
  printf '\n==> SKIP testing-hooks (CI_GATE_SKIP_TESTING_HOOKS)\n'
  skips+=("testing-hooks skipped by CI_GATE_SKIP_TESTING_HOOKS")
else
  printf '\n==> PROBE testing-hooks compile support\n'
  printf '+ cargo check --manifest-path %q --features testing-hooks\n' "$MANIFEST"
  if run_command cargo check --manifest-path "$MANIFEST" --features testing-hooks; then
    printf '==> PASS testing-hooks compile probe\n'
    run_gate "testing-hooks language suite" "CI_GATE_SKIP_TESTING_HOOKS" \
      cargo test --manifest-path "$MANIFEST" --features testing-hooks --test language_suite
  else
    status=$?
    printf '==> FAIL testing-hooks compile probe (exit %s)\n' "$status" >&2
    failures+=("testing-hooks compile probe exited $status")
  fi
fi

run_gate "Python tool tests" "CI_GATE_SKIP_PYTHON_TOOLS" \
  python3 -m unittest discover -s tools -p 'test_*.py'

run_gate "ABI header drift" "CI_GATE_SKIP_ABI_DRIFT" \
  scripts/check_abi_drift.sh

if should_skip "CI_GATE_SKIP_HASH_LOC_SMOKE"; then
  printf '\n==> SKIP hash/loc smoke (CI_GATE_SKIP_HASH_LOC_SMOKE)\n'
  skips+=("hash/loc smoke skipped by CI_GATE_SKIP_HASH_LOC_SMOKE")
else
  run_gate "hash tree smoke" "CI_GATE_SKIP_HASH_LOC_SMOKE" \
    python3 tools/hash.py --tree TinyOne --include .rs --format json
  run_gate "loc smoke" "CI_GATE_SKIP_HASH_LOC_SMOKE" \
    python3 tools/loc.py --json
fi

release_built=0
if should_skip "CI_GATE_SKIP_RELEASE_BUILD"; then
  printf '\n==> SKIP release bench build (CI_GATE_SKIP_RELEASE_BUILD)\n'
  skips+=("release bench build skipped by CI_GATE_SKIP_RELEASE_BUILD")
else
  printf '\n==> RUN release bench build\n'
  printf '+ cargo build --manifest-path %q --release --bin tinylang_bench\n' "$MANIFEST"
  if run_command cargo build --manifest-path "$MANIFEST" --release --bin tinylang_bench; then
    printf '==> PASS release bench build\n'
    release_built=1
  else
    status=$?
    printf '==> FAIL release bench build (exit %s)\n' "$status" >&2
    failures+=("release bench build exited $status")
  fi
fi

if should_skip "CI_GATE_SKIP_BENCH_SMOKE"; then
  printf '\n==> SKIP bench smoke (CI_GATE_SKIP_BENCH_SMOKE)\n'
  skips+=("bench smoke skipped by CI_GATE_SKIP_BENCH_SMOKE")
elif [[ "$release_built" == "1" ]]; then
  run_gate "bench smoke" "CI_GATE_SKIP_BENCH_SMOKE" \
    crates/tinyone_core/target/release/tinylang_bench --quick --repeats 1 --filter runtime.vm_straightline
else
  printf '\n==> SKIP bench smoke (release binary did not build)\n'
  skips+=("bench smoke skipped because release binary did not build")
fi

printf '\n==> CI gate summary\n'
if ((${#skips[@]})); then
  printf 'Skipped gates:\n'
  for item in "${skips[@]}"; do
    printf '  - %s\n' "$item"
  done
fi

if ((${#failures[@]})); then
  printf 'Failed gates:\n' >&2
  for item in "${failures[@]}"; do
    printf '  - %s\n' "$item" >&2
  done
  exit 1
fi

printf 'All enabled gates passed.\n'
