$ErrorActionPreference = "Stop"
$rootDir = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$buildDir = Join-Path $rootDir "TinyOne\target\debug"
$header = Join-Path $rootDir "tinylang.h"
$c = Join-Path $rootDir "tests\consumers\tinylang_consumer.c"
$cpp = Join-Path $rootDir "tests\consumers\tinylang_consumer.cpp"
$rs = Join-Path $rootDir "tests\consumers\tinylang_consumer.rs"

if (Get-Command cl.exe -ErrorAction SilentlyContinue) {
    Push-Location $buildDir
    try {
        cl.exe /nologo /std:c11 /W4 /WX /I $rootDir /c $c /Fo:tinylang_consumer_c.obj
        cl.exe /nologo /std:c++17 /W4 /WX /I $rootDir /c $cpp /Fo:tinylang_consumer_cpp.obj
    } finally {
        Pop-Location
    }
} elseif (Get-Command clang.exe -ErrorAction SilentlyContinue) {
    clang.exe -std=c11 -Wall -Wextra -Werror -I $rootDir -c $c -o (Join-Path $buildDir "tinylang_consumer_c.obj")
    clang++.exe -std=c++17 -Wall -Wextra -Werror -I $rootDir -c $cpp -o (Join-Path $buildDir "tinylang_consumer_cpp.obj")
} else {
    throw "No MSVC or Clang C compiler found"
}

rustc --edition 2024 --emit metadata $rs -o (Join-Path $buildDir "tinylang_consumer_rust.rmeta")
