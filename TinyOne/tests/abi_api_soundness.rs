use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::fs::{self, File};
use std::os::raw::c_char;
use std::panic;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value as JsonValue, json};
use tinyone::{
    BytecodeVerifier, Instr, JitCache, JitProgram, Op, Program, TinyMemory, VM, VerifiedProgram,
    compile_source, load_artifact, run_program, run_program_report, run_program_with_env,
    run_source, write_jit_listing,
};

const MAX_ARTIFACT_BYTES: u64 = 8 * 1024 * 1024;
const MAX_ARTIFACT_FUNCTIONS: usize = 4_096;
const MAX_ARTIFACT_CODE_OPS: usize = 65_536;
const MAX_ARTIFACT_STRINGS: usize = 65_536;
const MAX_ARTIFACT_SLOT_COUNT: usize = 65_536;
const MAX_VERIFIER_TOTAL_OPS: usize = 262_144;
const MAX_BUFFER_BYTES: u64 = 1024 * 1024;

unsafe extern "C" {
    fn tinyone_free_string(value: *mut c_char);
    fn tinyone_lex_source_json(source: *const c_char) -> *mut c_char;
    fn tinyone_compile_source_json(source: *const c_char) -> *mut c_char;
    fn tinyone_compile_file_json(path: *const c_char) -> *mut c_char;
    fn tinyone_run_source_json(
        source: *const c_char,
        mode: *const c_char,
        inputs_json: *const c_char,
    ) -> *mut c_char;
    fn tinyone_run_file_json(
        path: *const c_char,
        mode: *const c_char,
        inputs_json: *const c_char,
    ) -> *mut c_char;
    fn tinyone_run_artifact_json(
        artifact_json: *const c_char,
        mode: *const c_char,
        inputs_json: *const c_char,
    ) -> *mut c_char;
    fn tinyone_jit_listing_json(artifact_json: *const c_char) -> *mut c_char;
}

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(label: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "tinyone-abi-api-{label}-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temp test dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn cstring(value: impl AsRef<str>) -> CString {
    CString::new(value.as_ref()).expect("test strings must not contain NUL")
}

fn take_ffi_json(ptr: *mut c_char) -> JsonValue {
    assert!(!ptr.is_null(), "FFI returned a null response pointer");
    let text = unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .expect("FFI response must be UTF-8")
        .to_owned();
    unsafe { tinyone_free_string(ptr) };
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("FFI response must be valid JSON, got {text:?}: {error}"))
}

fn assert_ffi_ok(response: JsonValue) -> JsonValue {
    assert_eq!(response.get("ok").and_then(JsonValue::as_bool), Some(true));
    response
        .get("value")
        .cloned()
        .expect("successful FFI response must include value")
}

fn assert_ffi_error(response: JsonValue, kind: &str, needle: &str) {
    assert_eq!(response.get("ok").and_then(JsonValue::as_bool), Some(false));
    assert_eq!(response.get("kind").and_then(JsonValue::as_str), Some(kind));
    let error = response
        .get("error")
        .and_then(JsonValue::as_str)
        .expect("error response must include message");
    assert!(
        error.contains(needle),
        "expected FFI error to contain {needle:?}, got {error:?}"
    );
}

fn expect_error_contains<T>(result: tinyone::Result<T>, needle: &str) {
    let error = match result {
        Ok(_) => panic!("operation should fail"),
        Err(error) => error.to_string(),
    };
    assert!(
        error.contains(needle),
        "expected error to contain {needle:?}, got {error:?}"
    );
}

fn minimal_program() -> Program {
    Program {
        code: vec![Instr::new(Op::Halt, 0, 0)],
        slot_count: 0,
        names: Vec::new(),
        functions: Vec::new(),
        strings: Vec::new(),
        structs: Vec::new(),
        fields: Vec::new(),
        modules: Vec::new(),
    }
}

fn invalid_unverified_program() -> Program {
    Program {
        code: vec![Instr::new(Op::Print, 0, 0), Instr::new(Op::Halt, 0, 0)],
        ..minimal_program()
    }
}

fn minimal_artifact() -> JsonValue {
    json!({
        "format": "tinyone-bytecode",
        "version": 1,
        "code": [{"op": "HALT", "arg": 0, "arg2": 0}],
        "slot_count": 0,
        "names": [],
        "functions": [],
        "strings": [],
        "structs": [],
        "fields": [],
        "modules": [],
    })
}

fn expect_artifact_error(artifact: JsonValue, needle: &str) {
    expect_error_contains(Program::from_artifact(artifact), needle);
}

#[test]
fn ffi_success_responses_are_valid_json() {
    let dir = TestDir::new("ffi-success");
    let file = dir.path().join("main.to");
    fs::write(&file, "print 11").expect("write tinyone source");
    let file_path = cstring(file.to_string_lossy());
    let source = cstring("print 7");
    let mode = cstring("vm");
    let inputs = cstring("[]");

    let lex = assert_ffi_ok(take_ffi_json(unsafe {
        tinyone_lex_source_json(source.as_ptr())
    }));
    assert!(lex.get("tokens").and_then(JsonValue::as_u64).unwrap_or(0) > 0);

    let compiled = assert_ffi_ok(take_ffi_json(unsafe {
        tinyone_compile_source_json(source.as_ptr())
    }));
    assert_eq!(
        compiled
            .pointer("/artifact/format")
            .and_then(JsonValue::as_str),
        Some("tinyone-bytecode")
    );

    let file_compiled = assert_ffi_ok(take_ffi_json(unsafe {
        tinyone_compile_file_json(file_path.as_ptr())
    }));
    assert_eq!(
        file_compiled
            .pointer("/artifact/format")
            .and_then(JsonValue::as_str),
        Some("tinyone-bytecode")
    );

    let run_source = assert_ffi_ok(take_ffi_json(unsafe {
        tinyone_run_source_json(source.as_ptr(), mode.as_ptr(), inputs.as_ptr())
    }));
    assert_eq!(
        run_source.get("stdout").and_then(JsonValue::as_str),
        Some("7\n")
    );

    let run_file = assert_ffi_ok(take_ffi_json(unsafe {
        tinyone_run_file_json(file_path.as_ptr(), mode.as_ptr(), inputs.as_ptr())
    }));
    assert_eq!(
        run_file.get("stdout").and_then(JsonValue::as_str),
        Some("11\n")
    );

    let artifact = cstring(
        compile_source("print 13")
            .expect("compile source")
            .to_artifact()
            .to_string(),
    );
    let run_artifact = assert_ffi_ok(take_ffi_json(unsafe {
        tinyone_run_artifact_json(artifact.as_ptr(), mode.as_ptr(), inputs.as_ptr())
    }));
    assert_eq!(
        run_artifact.get("stdout").and_then(JsonValue::as_str),
        Some("13\n")
    );

    let listing = assert_ffi_ok(take_ffi_json(unsafe {
        tinyone_jit_listing_json(artifact.as_ptr())
    }));
    assert!(
        listing
            .get("listing")
            .and_then(JsonValue::as_str)
            .is_some_and(|listing| listing.contains(".chunk 0 main"))
    );
}

#[test]
fn ffi_null_pointers_return_valid_json_errors() {
    let null = std::ptr::null();
    assert_ffi_error(
        take_ffi_json(unsafe { tinyone_lex_source_json(null) }),
        "compile",
        "source pointer was null",
    );
    assert_ffi_error(
        take_ffi_json(unsafe { tinyone_compile_source_json(null) }),
        "compile",
        "source pointer was null",
    );
    assert_ffi_error(
        take_ffi_json(unsafe { tinyone_compile_file_json(null) }),
        "compile",
        "path pointer was null",
    );
    assert_ffi_error(
        take_ffi_json(unsafe { tinyone_run_source_json(null, null, null) }),
        "compile",
        "source pointer was null",
    );
    assert_ffi_error(
        take_ffi_json(unsafe { tinyone_run_file_json(null, null, null) }),
        "compile",
        "path pointer was null",
    );
    assert_ffi_error(
        take_ffi_json(unsafe { tinyone_run_artifact_json(null, null, null) }),
        "compile",
        "artifact pointer was null",
    );
    assert_ffi_error(
        take_ffi_json(unsafe { tinyone_jit_listing_json(null) }),
        "compile",
        "artifact pointer was null",
    );
}

#[test]
fn ffi_free_string_accepts_null() {
    unsafe { tinyone_free_string(std::ptr::null_mut()) };
}

#[test]
fn ffi_invalid_mode_returns_structured_error() {
    let source = cstring("print 1");
    let mode = cstring("native");
    assert_ffi_error(
        take_ffi_json(unsafe {
            tinyone_run_source_json(source.as_ptr(), mode.as_ptr(), std::ptr::null())
        }),
        "runtime",
        "Unsupported mode",
    );
}

#[test]
fn ffi_invalid_unicode_scalar_source_returns_error() {
    let invalid_surrogate_utf8 =
        CString::new(vec![0xED, 0xA0, 0x80]).expect("invalid UTF-8 bytes do not contain NUL");
    assert_ffi_error(
        take_ffi_json(unsafe { tinyone_lex_source_json(invalid_surrogate_utf8.as_ptr()) }),
        "compile",
        "must be UTF-8",
    );
}

#[test]
fn invalid_source_codepoint_returns_error_not_panic() {
    let result = panic::catch_unwind(|| compile_source("print \u{20ac}"));
    let compile_result = result.expect("invalid source codepoint must return Err, not panic");
    expect_error_contains(compile_result, "Unexpected character");
}

#[test]
fn ffi_artifact_json_over_byte_limit_returns_valid_json_error_before_parse() {
    let oversized =
        CString::new(vec![b' '; MAX_ARTIFACT_BYTES as usize + 1]).expect("spaces contain no NUL");
    let mode = cstring("vm");

    assert_ffi_error(
        take_ffi_json(unsafe {
            tinyone_run_artifact_json(oversized.as_ptr(), mode.as_ptr(), std::ptr::null())
        }),
        "compile",
        "byte limit",
    );
    assert_ffi_error(
        take_ffi_json(unsafe { tinyone_jit_listing_json(oversized.as_ptr()) }),
        "compile",
        "byte limit",
    );
}

#[test]
fn c_header_ffi_smoke_covers_ownership_null_and_mode_contracts() {
    let dir = TestDir::new("c-ffi-smoke");
    let source = dir.path().join("ffi_smoke.c");
    let exe = dir.path().join("ffi_smoke");
    fs::write(
        &source,
        r#"
#include "tinyone.h"

#include <stdio.h>
#include <string.h>

static int require_contains(const char *label, const char *text, const char *needle) {
    if (text == NULL) {
        fprintf(stderr, "%s returned NULL\n", label);
        return 1;
    }
    if (strstr(text, needle) == NULL) {
        fprintf(stderr, "%s response did not contain %s: %s\n", label, needle, text);
        return 1;
    }
    return 0;
}

int main(void) {
    tinyone_free_string(NULL);

    char *ok = tinyone_run_source_json("print 23", "vm", NULL);
    if (require_contains("run_source ok", ok, "\"ok\":true") != 0) {
        tinyone_free_string(ok);
        return 1;
    }
    if (require_contains("run_source stdout", ok, "\"stdout\":\"23\\n\"") != 0) {
        tinyone_free_string(ok);
        return 1;
    }
    tinyone_free_string(ok);

    char *null_source = tinyone_lex_source_json(NULL);
    if (require_contains("lex null", null_source, "source pointer was null") != 0) {
        tinyone_free_string(null_source);
        return 1;
    }
    tinyone_free_string(null_source);

    char *bad_mode = tinyone_run_source_json("print 1", "native", NULL);
    if (require_contains("bad mode", bad_mode, "Unsupported mode") != 0) {
        tinyone_free_string(bad_mode);
        return 1;
    }
    tinyone_free_string(bad_mode);

    return 0;
}
"#,
    )
    .expect("write C FFI smoke source");

    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().expect("Rust crate has repo parent");
    let target_dir = manifest_dir.join("target").join("debug");
    let dylib = target_dir.join(format!(
        "{}tinyone{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    if !dylib.exists() {
        eprintln!(
            "skipping C FFI smoke: cdylib not found at {} — run `cargo build` first",
            dylib.display()
        );
        return;
    }

    let compile = Command::new("cc")
        .arg("-std=c11")
        .arg("-Wall")
        .arg("-Wextra")
        .arg("-I")
        .arg(repo_root)
        .arg(&source)
        .arg("-L")
        .arg(&target_dir)
        .arg(format!("-Wl,-rpath,{}", target_dir.display()))
        .arg("-ltinyone")
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("run C compiler for FFI smoke");
    assert!(
        compile.status.success(),
        "C FFI smoke compile failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&compile.stdout),
        String::from_utf8_lossy(&compile.stderr)
    );

    let run = Command::new(&exe)
        .env("LD_LIBRARY_PATH", &target_dir)
        .output()
        .expect("run C FFI smoke");
    assert!(
        run.status.success(),
        "C FFI smoke failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
}

#[test]
fn artifact_rejects_huge_counts_before_accepting_program() {
    let mut artifact = minimal_artifact();
    artifact["slot_count"] = json!(MAX_ARTIFACT_SLOT_COUNT + 1);
    expect_artifact_error(artifact, "slot_count limit");

    let mut artifact = minimal_artifact();
    artifact["strings"] = JsonValue::Array(
        (0..=MAX_ARTIFACT_STRINGS)
            .map(|index| JsonValue::String(index.to_string()))
            .collect(),
    );
    expect_artifact_error(artifact, "strings limit");

    let mut artifact = minimal_artifact();
    let function = json!({
        "name": "f",
        "param_count": 0,
        "code": [
            {"op": "PUSH_INT", "arg": 0, "arg2": 0},
            {"op": "RETURN", "arg": 0, "arg2": 0}
        ],
        "slot_count": 0,
        "names": [],
    });
    artifact["functions"] = JsonValue::Array(vec![function; MAX_ARTIFACT_FUNCTIONS + 1]);
    expect_artifact_error(artifact, "functions limit");

    let mut artifact = minimal_artifact();
    artifact["code"] = JsonValue::Array(vec![JsonValue::Null; MAX_ARTIFACT_CODE_OPS + 1]);
    expect_artifact_error(artifact, "code limit");
}

#[test]
fn artifact_rejects_invalid_integer_fields_and_file_size() {
    let mut artifact = minimal_artifact();
    artifact["code"] = json!([{"op": "PUSH_STRING", "arg": i64::MAX, "arg2": 0}, {"op": "HALT", "arg": 0, "arg2": 0}]);
    artifact["strings"] = json!(["x"]);
    expect_artifact_error(artifact, "invalid string index");

    let mut artifact = minimal_artifact();
    artifact["code"] = json!([{"op": "HALT", "arg2": 0}]);
    expect_artifact_error(artifact, "instruction arg");

    let mut artifact = minimal_artifact();
    artifact["code"] = json!([{"op": "HALT", "arg": 0}]);
    expect_artifact_error(artifact, "instruction arg2");

    let dir = TestDir::new("artifact-size");
    let path = dir.path().join("huge.tbc.json");
    let file = File::create(&path).expect("create sparse artifact");
    file.set_len(MAX_ARTIFACT_BYTES + 1)
        .expect("make sparse artifact too large");
    expect_error_contains(load_artifact(&path), "byte size limit");
}

#[test]
fn verifier_handles_dense_jump_graph_without_path_explosion() {
    let mut code = Vec::new();
    for _ in 0..128 {
        let join = code.len() as i64 + 3;
        code.push(Instr::new(Op::PushInt, 0, 0));
        code.push(Instr::new(Op::JumpIfZero, join, 0));
        code.push(Instr::new(Op::Jump, join, 0));
    }
    code.push(Instr::new(Op::Halt, 0, 0));

    BytecodeVerifier::verify(&Program {
        code,
        ..minimal_program()
    })
    .expect("dense diamond graph should verify in bounded work");
}

#[test]
fn verifier_rejects_oversized_dense_jump_graph_before_stress() {
    let mut code = Vec::with_capacity(MAX_VERIFIER_TOTAL_OPS + 2);
    while code.len() <= MAX_VERIFIER_TOTAL_OPS {
        let join = code.len() as i64 + 3;
        code.push(Instr::new(Op::PushInt, 0, 0));
        code.push(Instr::new(Op::JumpIfZero, join, 0));
        code.push(Instr::new(Op::Jump, join, 0));
    }
    code.push(Instr::new(Op::Halt, 0, 0));

    expect_error_contains(
        BytecodeVerifier::verify(&Program {
            code,
            ..minimal_program()
        }),
        "total instruction count",
    );
}

#[test]
fn verifier_rejects_stack_depth_bomb() {
    let mut code = vec![Instr::new(Op::PushInt, 0, 0); 65_537];
    code.push(Instr::new(Op::Halt, 0, 0));
    expect_error_contains(
        BytecodeVerifier::verify(&Program {
            code,
            ..minimal_program()
        }),
        "stack depth",
    );
}

#[test]
fn fs_read_rejects_oversized_file_before_buffer_allocation() {
    let dir = TestDir::new("fs-read-large");
    let path = dir.path().join("large.bin");
    let file = File::create(&path).expect("create sparse file");
    file.set_len(MAX_BUFFER_BYTES + 1)
        .expect("make sparse file too large for TinyOne buffer");

    let source = format!(
        "let body = unsafe fs_read({:?}) print len(body)",
        path.to_string_lossy()
    );
    expect_error_contains(
        run_source(&source, "vm", &mut Vec::new(), Vec::new()),
        "file size",
    );
}

#[test]
fn fs_list_dir_limit_returns_error_not_panic() {
    let dir = TestDir::new("fs-list-limit");
    let padding = "x".repeat(249);
    for index in 0..5_000 {
        let name = format!("{index:05}_{padding}");
        File::create(dir.path().join(name)).expect("create directory entry");
    }
    let source = format!(
        "let names = unsafe fs_list_dir({:?}) print len(names)",
        dir.path().to_string_lossy()
    );

    let result = panic::catch_unwind(|| {
        let mut stdout = Vec::new();
        run_source(&source, "vm", &mut stdout, Vec::new())
    });
    let runtime_result = result.expect("fs_list_dir must return Err instead of panicking");
    expect_error_contains(runtime_result, "limit");
}

#[test]
fn map_pointer_keys_do_not_alias_after_heap_generation_reuse() {
    let source = r#"
    let first = [1]
    let oldp = ptr(first, 0)
    let m = map_new()
    let ignored = map_set(m, oldp, 123)
    let freed = unsafe free(first)
    let second = [2]
    let newp = ptr(second, 0)
    print map_has(m, newp)
    "#;
    for mode in ["vm", "jit"] {
        expect_error_contains(
            run_source(source, mode, &mut Vec::new(), Vec::new()),
            "Stale heap pointer",
        );
    }
}

#[test]
fn map_growth_obeys_heap_byte_budget() {
    let source = r#"
    let m = map_new()
    let i = 0
    while i < 25000 {
      let ignored = map_set(m, i, i)
      i = i + 1
    }
    print map_len(m)
    "#;
    for mode in ["vm", "jit"] {
        expect_error_contains(
            run_source(source, mode, &mut Vec::new(), Vec::new()),
            "Heap byte limit",
        );
    }
}

#[test]
fn vec_clear_releases_heap_byte_budget_before_free() {
    let source = r#"
    let v = []
    let i = 0
    while i < 40000 {
      let ignored = push(v, i)
      i = i + 1
    }
    print vec_clear(v)
    let freed = unsafe free(v)
    let b = buffer(400000)
    print len(b)
    "#;
    for mode in ["vm", "jit"] {
        let mut stdout = Vec::new();
        run_source(source, mode, &mut stdout, Vec::new()).expect("vec_clear should release bytes");
        assert_eq!(
            String::from_utf8(stdout).expect("stdout UTF-8"),
            "0\n400000\n"
        );
    }
}

#[test]
fn verifier_rejects_unreachable_invalid_operands() {
    let invalid = Program {
        code: vec![
            Instr::new(Op::Jump, 2, 0),
            Instr::new(Op::Load, -1, 0),
            Instr::new(Op::Halt, 0, 0),
        ],
        ..minimal_program()
    };

    expect_error_contains(BytecodeVerifier::verify(&invalid), "invalid slot");
    expect_error_contains(
        run_program(Arc::new(invalid.clone()), "vm", &mut Vec::new(), Vec::new()),
        "invalid slot",
    );
    expect_error_contains(
        run_program(Arc::new(invalid), "jit", &mut Vec::new(), Vec::new()),
        "invalid slot",
    );
}

#[test]
fn jit_compile_rejects_invalid_unverified_program() {
    let invalid = invalid_unverified_program();
    expect_error_contains(JitProgram::compile(&invalid), "Verifier");

    let mut cache = JitCache::new();
    expect_error_contains(cache.compile(&invalid), "Verifier");
    assert!(
        cache.is_empty(),
        "invalid programs must not populate JIT cache"
    );
}

#[test]
fn invalid_char_index_path_returns_error_not_panic() {
    let result = panic::catch_unwind(|| {
        run_source(
            "print str_char_at(\"a\", 9223372036854775807)",
            "vm",
            &mut Vec::new(),
            Vec::new(),
        )
    });
    let runtime_result = result.expect("invalid char index must not panic");
    expect_error_contains(runtime_result, "index");
}

#[test]
fn public_safe_rust_paths_verify_untrusted_programs() {
    let invalid = invalid_unverified_program();

    expect_error_contains(VerifiedProgram::verify(invalid.clone()), "Verifier");
    expect_error_contains(
        run_program(Arc::new(invalid.clone()), "vm", &mut Vec::new(), Vec::new()),
        "Verifier",
    );
    expect_error_contains(
        run_program(Arc::new(invalid.clone()), "jit", &mut Vec::new(), Vec::new()),
        "Verifier",
    );
    expect_error_contains(
        run_program_report(Arc::new(invalid.clone()), "vm", &mut Vec::new(), Vec::new()),
        "Verifier",
    );
    expect_error_contains(
        run_program_with_env(
            Arc::new(invalid.clone()),
            "vm",
            &mut Vec::new(),
            Vec::new(),
            Vec::new(),
            HashMap::new(),
        ),
        "Verifier",
    );
    expect_error_contains(
        VM::new(Arc::new(invalid.clone()), TinyMemory::new(invalid.slot_count), Vec::new()),
        "Verifier",
    );

    let mut cache = JitCache::new();
    expect_error_contains(
        cache.run_program(&invalid, &mut Vec::new(), Vec::new()),
        "Verifier",
    );
    expect_error_contains(
        cache.run_program_report(&invalid, &mut Vec::new(), Vec::new()),
        "Verifier",
    );

    let dir = TestDir::new("jit-listing-invalid");
    expect_error_contains(
        write_jit_listing(&invalid, dir.path().join("invalid.tjit")),
        "Verifier",
    );

    let valid = minimal_program();
    let verified = VerifiedProgram::verify(valid.clone()).expect("minimal program verifies");
    assert_eq!(verified.program().fingerprint(), valid.fingerprint());
}

#[test]
fn public_safe_rust_paths_reject_oversized_raw_program_before_execution_allocation() {
    let oversized = Program {
        slot_count: MAX_ARTIFACT_SLOT_COUNT + 1,
        ..minimal_program()
    };

    expect_error_contains(BytecodeVerifier::verify(&oversized), "slot_count");
    expect_error_contains(VerifiedProgram::verify(oversized.clone()), "slot_count");
    expect_error_contains(
        run_program(Arc::new(oversized.clone()), "vm", &mut Vec::new(), Vec::new()),
        "slot_count",
    );
    expect_error_contains(
        run_program(Arc::new(oversized.clone()), "jit", &mut Vec::new(), Vec::new()),
        "slot_count",
    );
    expect_error_contains(
        VM::new(Arc::new(oversized.clone()), TinyMemory::new(0), Vec::new()),
        "slot_count",
    );
    expect_error_contains(JitProgram::compile(&oversized), "slot_count");
}

#[test]
fn adversarial_artifact_at_exact_slot_count_limit_does_not_panic() {
    // An artifact at exactly MAX_ARTIFACT_SLOT_COUNT should not be rejected
    // by the slot_count check (only if verifier finds another issue).
    let mut artifact = minimal_artifact();
    artifact["slot_count"] = json!(MAX_ARTIFACT_SLOT_COUNT);
    // Must not panic — any result (ok or structured error) is fine.
    let result = panic::catch_unwind(|| Program::from_artifact(artifact));
    assert!(
        result.is_ok(),
        "artifact at exact slot_count limit must not panic"
    );
}

#[test]
fn adversarial_artifact_negative_slot_count_is_rejected() {
    let mut artifact = minimal_artifact();
    artifact["slot_count"] = json!(-1i64);
    expect_artifact_error(artifact, "slot_count");
}

#[test]
fn adversarial_artifact_float_slot_count_is_rejected() {
    let mut artifact = minimal_artifact();
    artifact["slot_count"] = json!(1.5f64);
    expect_artifact_error(artifact, "slot_count");
}

#[test]
fn adversarial_artifact_u64_max_slot_count_is_rejected() {
    let mut artifact = minimal_artifact();
    artifact["slot_count"] = json!(u64::MAX);
    // u64::MAX is far above MAX_ARTIFACT_SLOT_COUNT; the range check must reject it
    expect_artifact_error(artifact, "slot_count");
}

#[test]
fn adversarial_artifact_duplicate_format_key_last_value_wins() {
    // serde_json uses the last value for duplicate keys. The format check
    // must catch the final "bad" value, not the first "tinyone-bytecode".
    let text = r#"{"format":"tinyone-bytecode","version":1,"format":"bad","code":[{"op":"HALT","arg":0,"arg2":0}],"slot_count":0,"names":[],"functions":[],"strings":[],"structs":[],"fields":[]}"#;
    let artifact: JsonValue = serde_json::from_str(text).expect("valid JSON");
    // "Unsupported" matches the literal prefix of the error emitted by
    // Program::from_artifact: "Unsupported TinyOne artifact format"
    // (see src/bytecode/artifact.rs, from_artifact(), format check).
    expect_artifact_error(artifact, "Unsupported");
}

#[test]
fn adversarial_verifier_tight_loop_terminates_cleanly() {
    // A simple infinite loop (from verifier's perspective, reachable code)
    // should verify in O(nodes) time, not O(paths).
    let code = vec![
        Instr::new(Op::PushInt, 0, 0),    // pc=0: push 0
        Instr::new(Op::JumpIfZero, 3, 0), // pc=1: if zero, skip to halt
        Instr::new(Op::Jump, 0, 0),       // pc=2: back-edge
        Instr::new(Op::Halt, 0, 0),       // pc=3
    ];
    BytecodeVerifier::verify(&Program {
        code,
        ..minimal_program()
    })
    .expect("tight loop should verify without timeout");
}

#[test]
fn adversarial_ffi_artifact_at_max_minus_one_bytes_returns_clean_error() {
    // MAX_ARTIFACT_BYTES - 1 spaces is just under the byte limit, so the
    // limit check passes, but it's invalid JSON — should return a clean error.
    let near_limit =
        CString::new(vec![b' '; MAX_ARTIFACT_BYTES as usize - 1]).expect("no NUL in spaces");
    let mode = cstring("vm");
    let response = take_ffi_json(unsafe {
        tinyone_run_artifact_json(near_limit.as_ptr(), mode.as_ptr(), std::ptr::null())
    });
    assert_eq!(
        response.get("ok").and_then(JsonValue::as_bool),
        Some(false),
        "near-limit invalid JSON must return ok=false"
    );
    assert!(
        response.get("kind").and_then(JsonValue::as_str).is_some(),
        "error response must include kind field"
    );
}

#[test]
fn adversarial_jit_program_from_artifact_roundtrip_does_not_panic() {
    // Compile source, serialize to artifact, deserialize, JIT compile.
    // The whole roundtrip should be panic-free.
    let program = compile_source("print 42").expect("compile");
    let artifact = program.to_artifact();
    let result = panic::catch_unwind(|| {
        let p = Program::from_artifact(artifact).expect("deserialize");
        JitProgram::compile(&p).expect("jit compile")
    });
    assert!(
        result.is_ok(),
        "artifact roundtrip through JIT must not panic"
    );
}

#[test]
fn vm_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<tinyone::VM>();
}
