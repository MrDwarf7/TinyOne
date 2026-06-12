#!/bin/sh
set -eu

# Build from workspace root to get proper staticlib output
repo_root="$(cd "$(dirname "$0")/../../../.." && pwd)"
target_dir="${CARGO_TARGET_DIR:-$repo_root/target}"
lib_path="$target_dir/debug/libralloc.a"
binary_path="$target_dir/c-suite/ralloc_c_suite"
cc="${CC:-cc}"

cd "$repo_root"
cargo build --package tinyone_ralloc --features cdylib

if [ ! -f "$lib_path" ]; then
    echo "ERROR: Could not find $lib_path"
    exit 1
fi

mkdir -p "$(dirname "$binary_path")"

"$cc" \
    -std=c11 \
    -Wall \
    -Wextra \
    -Werror \
    -Icrates/tinyone_ralloc/include \
    crates/tinyone_ralloc/tests/c/ralloc_c_suite.c \
    "$lib_path" \
    -pthread \
    -o "$binary_path"

"$binary_path"