#!/bin/sh
set -eu

target_dir="${CARGO_TARGET_DIR:-target}/c-suite"
lib_path="$target_dir/debug/libralloc.a"
binary_path="$target_dir/ralloc_c_suite"
cc="${CC:-cc}"

cargo build --package ralloc-staticlib --target-dir "$target_dir"

"$cc" \
    -std=c11 \
    -Wall \
    -Wextra \
    -Werror \
    -Iinclude \
    tests/c/ralloc_c_suite.c \
    "$lib_path" \
    -pthread \
    -o "$binary_path"

"$binary_path"
