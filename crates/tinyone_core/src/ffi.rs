use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::Arc;

use serde_json::{Value as JsonValue, json};

use crate::bytecode::artifact::MAX_ARTIFACT_BYTES;
use crate::{
    JitProgram,
    Program,
    Result,
    RuntimeValue,
    TinyHeapStats,
    TinyMemory,
    TinyOneError,
    compile_file,
    compile_source,
    lex_source,
    run_program_report,
    run_source_report,
};

/// # Safety
///
/// `value` must be null or a pointer returned by a TinyOne C-ABI function
/// that has not already been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tinyone_free_string(value: *mut c_char) {
    if value.is_null() {
        return;
    }
    unsafe {
        drop(CString::from_raw(value));
    }
}

/// # Safety
///
/// `source` may be null. If non-null, it must point to a valid
/// NUL-terminated UTF-8 C string for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tinyone_lex_source_json(source: *const c_char) -> *mut c_char {
    respond(|| {
        let source = read_string(source, "source")?;
        Ok(json!({"tokens": lex_source(&source)?}))
    })
}

/// # Safety
///
/// `source` may be null. If non-null, it must point to a valid
/// NUL-terminated UTF-8 C string for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tinyone_compile_source_json(source: *const c_char) -> *mut c_char {
    respond(|| {
        let source = read_string(source, "source")?;
        program_payload(compile_source(&source)?)
    })
}

/// # Safety
///
/// `path` may be null. If non-null, it must point to a valid NUL-terminated
/// UTF-8 C string for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tinyone_compile_file_json(path: *const c_char) -> *mut c_char {
    respond(|| {
        let path = read_string(path, "path")?;
        program_payload(compile_file(Path::new(&path))?)
    })
}

/// # Safety
///
/// `source`, `mode`, and `inputs_json` may be null. Any non-null pointer must
/// point to a valid NUL-terminated UTF-8 C string for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tinyone_run_source_json(
    source: *const c_char,
    mode: *const c_char,
    inputs_json: *const c_char,
) -> *mut c_char {
    respond(|| {
        let source = read_string(source, "source")?;
        let mode = read_string(mode, "mode")?;
        let inputs = read_inputs(inputs_json)?;
        let mut stdout = Vec::new();
        let report = run_source_report(&source, &mode, &mut stdout, inputs)?;
        run_payload(stdout, report.memory, report.heap_before_shutdown, report.heap_after_shutdown)
    })
}

/// # Safety
///
/// `path`, `mode`, and `inputs_json` may be null. Any non-null pointer must
/// point to a valid NUL-terminated UTF-8 C string for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tinyone_run_file_json(
    path: *const c_char,
    mode: *const c_char,
    inputs_json: *const c_char,
) -> *mut c_char {
    respond(|| {
        let path = read_string(path, "path")?;
        let mode = read_string(mode, "mode")?;
        let inputs = read_inputs(inputs_json)?;
        let program = compile_file(Path::new(&path))?;
        run_compiled_program(program, &mode, inputs)
    })
}

/// # Safety
///
/// `artifact_json`, `mode`, and `inputs_json` may be null. Any non-null pointer
/// must point to a valid NUL-terminated UTF-8 C string for the duration of the
/// call. `artifact_json` must not exceed the documented artifact byte limit.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tinyone_run_artifact_json(
    artifact_json: *const c_char,
    mode: *const c_char,
    inputs_json: *const c_char,
) -> *mut c_char {
    respond(|| {
        let artifact = read_artifact_json(artifact_json)?;
        let mode = read_string(mode, "mode")?;
        let inputs = read_inputs(inputs_json)?;
        let program = Arc::new(Program::from_artifact(artifact)?);
        run_compiled_program(program, &mode, inputs)
    })
}

/// # Safety
///
/// `artifact_json` may be null. If non-null, it must point to a valid
/// NUL-terminated UTF-8 C string for the duration of the call and must not
/// exceed the documented artifact byte limit.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tinyone_jit_listing_json(artifact_json: *const c_char) -> *mut c_char {
    respond(|| {
        let artifact = read_artifact_json(artifact_json)?;
        let program = Program::from_artifact(artifact)?;
        Ok(json!({"listing": JitProgram::compile(&program)?.listing()}))
    })
}

fn respond(callback: impl FnOnce() -> Result<JsonValue>) -> *mut c_char {
    match catch_unwind(AssertUnwindSafe(|| response_cstring(callback))) {
        Ok(response) => response.into_raw(),
        Err(_) => fallback_response().into_raw(),
    }
}

fn response_cstring(callback: impl FnOnce() -> Result<JsonValue>) -> CString {
    let payload = match catch_unwind(AssertUnwindSafe(callback)) {
        Ok(Ok(value)) => json!({"ok": true, "value": value}),
        Ok(Err(error)) => error_payload(&error),
        Err(_) => {
            json!({
                "ok": false,
                "kind": "panic",
                "error": "TinyOne panicked across the FFI boundary"
            })
        }
    };
    match serde_json::to_string(&payload) {
        Ok(text) => cstring_or_fallback(text),
        Err(_) => fallback_response(),
    }
}

fn error_payload(error: &TinyOneError) -> JsonValue {
    let kind = match error {
        TinyOneError::Compile(_) => "compile",
        TinyOneError::Runtime(_) => "runtime",
    };
    json!({"ok": false, "kind": kind, "error": error.to_string()})
}

fn cstring_or_fallback(text: String) -> CString {
    CString::new(text).unwrap_or_else(|_| fallback_response())
}

fn fallback_response() -> CString {
    const FALLBACK: &[u8] = b"{\"ok\":false,\"kind\":\"panic\",\"error\":\"response serialization failed\"}\0";
    // The byte string above is static valid JSON followed by exactly one NUL.
    unsafe { CString::from_vec_with_nul_unchecked(FALLBACK.to_vec()) }
}

fn read_string(value: *const c_char, name: &str) -> Result<String> {
    if value.is_null() {
        return Err(TinyOneError::compile(format!("{name} pointer was null")));
    }
    unsafe { CStr::from_ptr(value) }
        .to_str()
        .map(ToOwned::to_owned)
        .map_err(|error| TinyOneError::compile(format!("{name} must be UTF-8: {error}")))
}

fn read_json(value: *const c_char, name: &str) -> Result<JsonValue> {
    let text = read_string(value, name)?;
    serde_json::from_str(&text).map_err(|error| TinyOneError::compile(format!("{name} must be valid JSON: {error}")))
}

fn read_artifact_json(value: *const c_char) -> Result<JsonValue> {
    let text = read_string_limited(value, "artifact", MAX_ARTIFACT_BYTES)?;
    serde_json::from_str(&text).map_err(|error| TinyOneError::compile(format!("artifact must be valid JSON: {error}")))
}

fn read_string_limited(value: *const c_char, name: &str, max_bytes: usize) -> Result<String> {
    if value.is_null() {
        return Err(TinyOneError::compile(format!("{name} pointer was null")));
    }
    for len in 0..=max_bytes {
        let byte = unsafe { *value.add(len) };
        if byte == 0 {
            let bytes = unsafe { std::slice::from_raw_parts(value.cast::<u8>(), len) };
            return std::str::from_utf8(bytes)
                .map(ToOwned::to_owned)
                .map_err(|error| TinyOneError::compile(format!("{name} must be UTF-8: {error}")));
        }
    }
    Err(TinyOneError::compile(format!("{name} exceeds byte limit {max_bytes}")))
}

fn read_inputs(value: *const c_char) -> Result<Vec<String>> {
    if value.is_null() {
        return Ok(Vec::new());
    }
    let data = read_json(value, "inputs")?;
    serde_json::from_value(data)
        .map_err(|error| TinyOneError::runtime(format!("inputs must be a JSON string list: {error}")))
}

fn program_payload(program: Arc<Program>) -> Result<JsonValue> {
    Ok(json!({
        "artifact": program.to_artifact(),
        "fingerprint": program.fingerprint(),
    }))
}

fn run_compiled_program(program: Arc<Program>, mode: &str, inputs: Vec<String>) -> Result<JsonValue> {
    let mut stdout = Vec::new();
    let report = run_program_report(program, mode, &mut stdout, inputs)?;
    run_payload(stdout, report.memory, report.heap_before_shutdown, report.heap_after_shutdown)
}

fn run_payload(
    stdout: Vec<u8>,
    memory: TinyMemory,
    heap_before_shutdown: TinyHeapStats,
    heap_after_shutdown: TinyHeapStats,
) -> Result<JsonValue> {
    let stdout =
        String::from_utf8(stdout).map_err(|error| TinyOneError::runtime(format!("stdout was not UTF-8: {error}")))?;
    Ok(json!({
        "stdout": stdout,
        "memory": memory_to_json(&memory),
        "heap_before_shutdown": heap_stats_to_json(heap_before_shutdown),
        "heap_after_shutdown": heap_stats_to_json(heap_after_shutdown),
    }))
}

fn memory_to_json(memory: &TinyMemory) -> Vec<JsonValue> {
    memory.snapshot().iter().map(value_to_json).collect()
}

fn value_to_json(value: &RuntimeValue) -> JsonValue {
    match value {
        RuntimeValue::I8(v) => json!({"type": "i8",  "value": v}),
        RuntimeValue::I16(v) => json!({"type": "i16", "value": v}),
        RuntimeValue::I32(v) => json!({"type": "i32", "value": v}),
        RuntimeValue::I64(v) => json!({"type": "i64", "value": v}),
        RuntimeValue::U8(v) => json!({"type": "u8",  "value": v}),
        RuntimeValue::U16(v) => json!({"type": "u16", "value": v}),
        RuntimeValue::U32(v) => json!({"type": "u32", "value": v}),
        RuntimeValue::U64(v) => json!({"type": "u64", "value": v}),
        RuntimeValue::Bf16(v) => json!({"type": "bf16", "bits": v}),
        RuntimeValue::Float { kind, bits } => json!({"type": kind.name(), "value": bits}),
        RuntimeValue::Bool(b) => json!({"type": "bool", "value": b}),
        RuntimeValue::Unit => json!({"type": "unit"}),
        RuntimeValue::Null => json!({"type": "null"}),
        RuntimeValue::Function(id) => json!({"type": "function", "id": id}),
        RuntimeValue::Reference(p) => json!({"type": "reference", "address": p.address}),
        RuntimeValue::Phantom => json!({"type": "phantom"}),
        RuntimeValue::Zst(k) => json!({"type": "zst", "marker": k.name()}),
        RuntimeValue::Unsafe => json!({"type": "unsafe"}),
        RuntimeValue::Heap(r) => {
            json!({
                "type": "heap",
                "address": r.address,
                "generation": r.generation,
            })
        }
        RuntimeValue::Pointer(p) => {
            json!({
                "type": "pointer",
                "address": p.address,
                "kind": p.kind,
                "index": p.index,
                "field": p.field,
                "generation": p.generation,
                "cast": p.cast,
            })
        }
    }
}

fn heap_stats_to_json(stats: TinyHeapStats) -> JsonValue {
    json!({
        "live_objects": stats.live_objects,
        "live_bytes": stats.live_bytes,
        "peak_objects": stats.peak_objects,
        "peak_bytes": stats.peak_bytes,
        "total_allocations": stats.total_allocations,
        "total_frees": stats.total_frees,
        "shutdown_frees": stats.shutdown_frees,
    })
}
