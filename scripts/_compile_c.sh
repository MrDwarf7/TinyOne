#!/usr/bin/env bash
set -euo pipefail

export t=$'\t'

_usage() {
  local _self
  _self="$(basename "${BASH_SOURCE[0]:-$0}")"
  printf "%s\n" "$(cat <<EOF
Usage: ${_self} [help|-h|--help]
  or: bash ${BASH_SOURCE[0]} [help|-h|--help]

Compiles tests/consumers/tinylang_consumer.c for the detected toolchain.
Utils: cl.exe (MSVC) -> clang.exe (Windows) -> cc (fallback).
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
# shellcheck source=_common.sh
source "${SCRIPT_DIR}/_common.sh"

CC_BIN="${CC:-cc}"

_compile_msvc() {
  _args=(
    "${_common_msvc_flags[@]}"
    /std:c11
    /c "$(_to_win_path "${ROOT_DIR}/tests/consumers/tinylang_consumer.c")"
    /Fo:"${BUILD_DIR_WIN}/tinylang_consumer_c.obj"
  )
  pushd "${BUILD_DIR}" >/dev/null
  MSYS2_ARG_CONV_EXCL="*" cl.exe "${_args[@]}"
  popd >/dev/null
}

_compile_clang_win() {
  _args=(
    -std=c11
    "${_common_gcc_flags[@]}"
    -c "${ROOT_DIR}/tests/consumers/tinylang_consumer.c"
    -o "${BUILD_DIR}/tinylang_consumer_c.obj"
  )
  clang.exe "${_args[@]}"
}

_compile_gcc() {
  _args=(
    -std=c11
    "${_common_gcc_flags[@]}"
    -c "${ROOT_DIR}/tests/consumers/tinylang_consumer.c"
    -o "${BUILD_DIR}/tinylang_consumer_c.o"
  )
  "${CC_BIN}" "${_args[@]}"
}

_candidates=(
  "cl.exe:_compile_msvc"
  "clang.exe:_compile_clang_win"
)

_dispatched=0
for _entry in "${_candidates[@]}"; do
  IFS=":" read -r _bin _helper <<< "${_entry}"
  command -v "${_bin}" >/dev/null 2>&1 || continue
  if [[ "${_bin}" == "clang.exe" ]]; then
    case "$(uname -s 2>/dev/null || printf "Linux")" in MINGW*|MSYS*) ;; *) continue;; esac
  fi
  "${_helper}"
  _dispatched=1
  break
done

[[ "${_dispatched}" -eq 1 ]] || _compile_gcc
