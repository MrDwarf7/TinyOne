#!/usr/bin/env bash
set -euo pipefail

# check_abi_drift.sh -- ABI header drift check via uv + abi_manifest.py

export t=$'\t'

_usage() {
  local _self
  _self="$(basename "${BASH_SOURCE[0]:-$0}")"
  printf "%s\n" "$(cat <<EOF
Usage: ${_self} [--help|-h|help] [-- <abi_manifest args>]
  or: bash ${BASH_SOURCE[0]} [-- <abi_manifest args>]

Runs: uv run --no-project --python 3.12 python tools/abi_manifest.py check [args]
No positional args required; help flags show this message.
Extra args after -- are forwarded to abi_manifest.py.

Examples:
${t}bash ${BASH_SOURCE[0]}
${t}bash ${BASH_SOURCE[0]} --help
${t}bash ${BASH_SOURCE[0]} -- --verbose

For LLM: no fallback. Missing uv/python is explicit failure. Use -h for help.
EOF
)"
}

case "${1:-}" in
  -h|--help|help) _usage; exit 0 ;;
esac

ROOT="$(CDPATH=$(cd -- "$(dirname -- "$0")/..") && pwd)"

# PKGBUILD-style: base args array, exec with expansion
_base_uv_args=(
  run
  --no-project
  --python 3.12
  python
  "$ROOT/tools/abi_manifest.py"
  check
)

exec uv "${_base_uv_args[@]}" "$@"
