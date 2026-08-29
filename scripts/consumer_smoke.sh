#!/usr/bin/env bash
set -euo pipefail

# Consumer smoke: build cdylib + compile C consumer against it + run.
# Uses the workspace CARGO_TARGET_DIR, rather than a crate-local target path.

export t=$'\t'

_usage() {
  local _self
  _self="$(basename "${BASH_SOURCE[0]:-$0}")"
  printf "%s\n" "$(cat <<EOF
Usage: ${_self} [help|-h|--help]
  or: bash ${BASH_SOURCE[0]} [help|-h|--help]

Builds cdylib (cargo build), links tests/consumers/tinylang_consumer.c against
libtinyone, and executes the resulting binary.
Honors: CC, CARGO_TARGET_DIR.
No arguments expected; help flags show this message.

Examples:
${t}bash ${BASH_SOURCE[0]}
${t}bash ${BASH_SOURCE[0]} --help

For LLM: no args required. C toolchain probing is explicit; no fallback hiding.
EOF
)"
}

case "${1:-}" in
  -h|--help|help) _usage; exit 0 ;;
esac

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-${ROOT_DIR}/target}"
BUILD_DIR="${TARGET_DIR}/debug"
CC_BIN="${CC:-cc}"

# Single base compile arg set, appended per-scope below
_base_c_args=(
  -std=c11
  -Wall
  -Wextra
  -Werror
  -I"${ROOT_DIR}"
)

cargo build

_link_c_args=(
  "${_base_c_args[@]}"
  "${ROOT_DIR}/tests/consumers/tinylang_consumer.c"
  -L"${BUILD_DIR}"
  -Wl,-rpath,"${BUILD_DIR}"
  -ltinyone
  -o "${BUILD_DIR}/tinylang_consumer_c"
)

"${CC_BIN}" "${_link_c_args[@]}"

if [[ "$(uname -s)" == "Darwin" ]]; then
  DYLD_LIBRARY_PATH="${BUILD_DIR}${DYLD_LIBRARY_PATH:+:${DYLD_LIBRARY_PATH}}" \
    "${BUILD_DIR}/tinylang_consumer_c"
else
  LD_LIBRARY_PATH="${BUILD_DIR}${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}" \
    "${BUILD_DIR}/tinylang_consumer_c"
fi
