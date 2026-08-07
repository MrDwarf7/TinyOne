#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUILD_DIR="${ROOT_DIR}/TinyOne/target/debug"
CC_BIN="${CC:-cc}"
CXX_BIN="${CXX:-c++}"

"${CC_BIN}" -std=c11 -Wall -Wextra -Werror -I"${ROOT_DIR}" \
  -c "${ROOT_DIR}/tests/consumers/tinylang_consumer.c" \
  -o "${BUILD_DIR}/tinylang_consumer_c.o"
"${CXX_BIN}" -std=c++17 -Wall -Wextra -Werror -I"${ROOT_DIR}" \
  -c "${ROOT_DIR}/tests/consumers/tinylang_consumer.cpp" \
  -o "${BUILD_DIR}/tinylang_consumer_cpp.o"
rustc --edition=2024 --emit=metadata \
  "${ROOT_DIR}/tests/consumers/tinylang_consumer.rs" \
  -o "${BUILD_DIR}/tinylang_consumer_rust.rmeta"
