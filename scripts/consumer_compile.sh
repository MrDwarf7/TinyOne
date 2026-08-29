#!/usr/bin/env bash
set -euo pipefail

# consumer_compile.sh -- dispatcher for per-lang leaves
# Delegates to _compile_*.sh which each handle toolchain dispatch internally.

export t=$'\t'

_usage() {
  local _self
  _self="$(basename "${BASH_SOURCE[0]:-$0}")"
  printf "%s\n" "$(cat <<EOF
Usage: ${_self} <c|cpp|rust|all> [help|-h|--help]
  or: bash ${BASH_SOURCE[0]} <c|cpp|rust|all>

Dispatcher for per-lang consumer fixtures. No fallback -- arg required.

Args:
${t}c       Compile C fixture  (tests/consumers/tinylang_consumer.c)
${t}cpp     Compile C++ fixture (tests/consumers/tinylang_consumer.cpp)
${t}rust    Compile Rust fixture (tests/consumers/tinylang_consumer.rs)
${t}all     Compile all three (c + cpp + rust)

Options:
${t}-h, --help, help   Show this help and exit

Examples:
${t}bash ${BASH_SOURCE[0]} c
${t}bash ${BASH_SOURCE[0]} cpp
${t}bash ${BASH_SOURCE[0]} all

For LLM: explicit arg required. No default fallback. Unknown arg = error.
Make tasks: bash ${BASH_SOURCE[0]} c  (or cpp/rust/all)
EOF
)"
}

case "${1:-}" in
  -h|--help|help) _usage; exit 0 ;;
esac

if [[ $# -eq 0 ]]; then
  _self="$(basename "${BASH_SOURCE[0]:-$0}")"
  printf "%s: missing required argument (expected: c|cpp|rust|all)\n" "${_self}" >&2
  _usage >&2
  exit 2
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

case "${1}" in
  c)    bash "${SCRIPT_DIR}/_compile_c.sh" ;;
  cpp)  bash "${SCRIPT_DIR}/_compile_cpp.sh" ;;
  rust) bash "${SCRIPT_DIR}/_compile_rust.sh" ;;
  all)
    bash "${SCRIPT_DIR}/_compile_c.sh"
    bash "${SCRIPT_DIR}/_compile_cpp.sh"
    bash "${SCRIPT_DIR}/_compile_rust.sh"
    ;;
  *)
    _self="$(basename "${BASH_SOURCE[0]:-$0}")"
    printf "%s: unknown arg '%s' (expected c|cpp|rust|all)\n" "${_self}" "${1}" >&2
    _usage >&2
    exit 2
    ;;
esac

printf "consumer_compile: ok (%s) -> %s/debug\n" "${1}" "${CARGO_TARGET_DIR:-${SCRIPT_DIR}/../target}"
