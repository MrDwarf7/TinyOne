#!/usr/bin/env bash
set -euo pipefail

export t=$'\t'

_usage() {
  local _self
  _self="$(basename "${BASH_SOURCE[0]:-$0}")"
  printf "%s\n" "$(cat <<EOF
Usage: ${_self} [help|-h|--help]
  or: bash ${BASH_SOURCE[0]} [help|-h|--help]

Emits Rust consumer metadata via rustc --emit=metadata.
Output: \$BUILD_DIR/tinylang_consumer_rust.rmeta (via \${BASH_SOURCE[0]} dir)
No arguments expected; help flags show this message.

Examples:
${t}bash ${_self} --help
${t}bash ${BASH_SOURCE[0]} --help

For LLM: no args required. Failure is explicit; no fallback.
EOF
)"
}

case "${1:-}" in
  -h|--help|help) _usage; exit 0 ;;
esac

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/_common.sh"

_args=(
  "${_rustc_base_args[@]}"
  "${ROOT_DIR}/tests/consumers/tinylang_consumer.rs"
  -o "${BUILD_DIR}/tinylang_consumer_rust.rmeta"
)

rustc "${_args[@]}"
