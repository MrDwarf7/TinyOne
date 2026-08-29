#!/usr/bin/env bash
# _common.sh -- sourced by compile_* leaves and consumer dispatchers
# Single source for ROOT/TARGET/BUILD and PKGBUILD-style common flag sets.

set -euo pipefail

export t=$'\t'

_usage() {
  local _self
  _self="$(basename "${BASH_SOURCE[0]:-$0}")"
  printf "%s\n" "$(cat <<EOF
Usage: source ${_self}  # or: source scripts/_common.sh

Sourced helper -- not executed directly.
Sets: ROOT_DIR, TARGET_DIR, BUILD_DIR, _common_gcc_flags, _common_msvc_flags, _rustc_base_args
For LLM: source this file, do not execute. No args expected.

Example:
${t}source \${BASH_SOURCE[0]}
EOF
)"
}

case "${1:-}" in
  -h|--help|help) _usage; exit 0 ;;
esac

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-${ROOT_DIR}/target}"
BUILD_DIR="${TARGET_DIR}/debug"

mkdir -p "${BUILD_DIR}"

# Convert ROOT_DIR to Windows style for cl.exe (/I) when on MSYS/MinGW.
# On Windows runners, ROOT_DIR is /d/a/TinyOne/TinyOne but cl.exe needs D:\a\...
# Use cygpath if available, else manual /c/ -> C:\ conversion.
_to_win_path() {
  local p="$1"
  if command -v cygpath >/dev/null 2>&1; then
    cygpath -w "$p" 2>/dev/null || printf '%s' "$p"
    return
  fi
  if [[ "$p" =~ ^/([A-Za-z])/(.*) ]]; then
    local drive="${BASH_REMATCH[1]}"
    local rest="${BASH_REMATCH[2]}"
    # Uppercase drive
    drive="$(printf '%s' "$drive" | tr '[:lower:]' '[:upper:]')"
    printf '%s:\\%s' "$drive" "${rest//\//\\}"
  else
    printf '%s' "$p"
  fi
}

if [[ "$(uname -s 2>/dev/null || echo Linux)" == MINGW* ]] || [[ "$(uname -s 2>/dev/null || echo Linux)" == MSYS* ]]; then
  ROOT_DIR_WIN="$(_to_win_path "$ROOT_DIR")"
  BUILD_DIR_WIN="$(_to_win_path "$BUILD_DIR")"
else
  ROOT_DIR_WIN="$ROOT_DIR"
  BUILD_DIR_WIN="$BUILD_DIR"
fi

# Single common sets -- only -std / extension vary per-file below
_common_gcc_flags=(
  -Wall
  -Wextra
  -Werror
  -I"${ROOT_DIR}"
)

_common_msvc_flags=(
  /nologo
  /W4
  /WX
  /I "${ROOT_DIR_WIN}"
)

_rustc_base_args=(
  --edition=2024
  --emit=metadata
)
