#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MANIFEST="${ROOT_DIR}/TinyOne/Cargo.toml"
BUILD_DIR="${ROOT_DIR}/TinyOne/target/debug"
CC_BIN="${CC:-cc}"

cargo build --manifest-path "${MANIFEST}"
"${CC_BIN}" -std=c11 -Wall -Wextra -Werror -I"${ROOT_DIR}" \
  "${ROOT_DIR}/tests/consumers/tinylang_consumer.c" \
  -L"${BUILD_DIR}" -Wl,-rpath,"${BUILD_DIR}" -ltinyone \
  -o "${BUILD_DIR}/tinylang_consumer_c"

if [[ "$(uname -s)" == "Darwin" ]]; then
  DYLD_LIBRARY_PATH="${BUILD_DIR}${DYLD_LIBRARY_PATH:+:${DYLD_LIBRARY_PATH}}" \
    "${BUILD_DIR}/tinylang_consumer_c"
else
  LD_LIBRARY_PATH="${BUILD_DIR}${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}" \
    "${BUILD_DIR}/tinylang_consumer_c"
fi
