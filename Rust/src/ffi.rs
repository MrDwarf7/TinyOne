use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use serde_json::{Value as JsonValue, json};

use crate::{
    JitProgram, Program, Result, RuntimeValue, TinyHeapStats, TinyMemory, TinyOneError,
    compile_file, compile_source, lex_source, run_program_report, run_source_report,
};

#[unsafe(no_mangle)]
pub extern "C" fn tinyone_free_string(value: *mut c_char) {
    if value.is_null() {
        return;
    }
    unsafe {
        drop(CString::from_raw(value));
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn tinyone_lex_source_json(source: *const c_char) -> *mut c_char {
    respond(|| {
        let source = read_string(source, "source")?;
        Ok(json!({"tokens": lex_source(&source)?}))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn tinyone_compile_source_json(source: *const c_char) -> *mut c_char {
    respond(|| {
        let source = read_string(source, "source")?;
        program_payload(compile_source(&source)?)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn tinyone_compile_file_json(path: *const c_char) -> *mut c_char {
    respond(|| {
        let path = read_string(path, "path")?;
        program_payload(compile_file(Path::new(&path))?)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn tinyone_run_source_json(
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
        Ok(run_payload(
            stdout,
            report.memory,
            report.heap_before_shutdown,
            report.heap_after_shutdown,
        )?)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn tinyone_run_file_json(
    path: *const c_char,
    mode: *const c_char,
    inputs_json: *const c_char,
) -> *mut c_char {
    respond(|| {
        let path = read_string(path, "path")?;
        let mode = read_string(mode, "mode")?;
        let inputs = read_inputs(inputs_json)?;
        let program = compile_file(Path::new(&path))?;
        run_compiled_program(&program, &mode, inputs)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn tinyone_run_artifact_json(
    artifact_json: *const c_char,
    mode: *const c_char,
    inputs_json: *const c_char,
) -> *mut c_char {
    respond(|| {
        let artifact = read_json(artifact_json, "artifact")?;
        let mode = read_string(mode, "mode")?;
        let inputs = read_inputs(inputs_json)?;
        let program = Program::from_artifact(artifact)?;
        run_compiled_program(&program, &mode, inputs)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn tinyone_jit_listing_json(artifact_json: *const c_char) -> *mut c_char {
    respond(|| {
        let artifact = read_json(artifact_json, "artifact")?;
        let program = Program::from_artifact(artifact)?;
        Ok(json!({"listing": JitProgram::compile(&program).listing()}))
    })
}

fn respond(callback: impl FnOnce() -> Result<JsonValue>) -> *mut c_char {
    let payload = match catch_unwind(AssertUnwindSafe(callback)) {
        Ok(Ok(value)) => json!({"ok": true, "value": value}),
        Ok(Err(error)) => {
            let kind = match error {
                TinyOneError::Compile(_) => "compile",
                TinyOneError::Runtime(_) => "runtime",
            };
            json!({"ok": false, "kind": kind, "error": error.to_string()})
        }
        Err(_) => {
            json!({"ok": false, "kind": "panic", "error": "TinyOne panicked across the FFI boundary"})
        }
    };
    CString::new(payload.to_string())
        .expect("serde_json output never contains interior NUL bytes")
        .into_raw()
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
    serde_json::from_str(&text)
        .map_err(|error| TinyOneError::compile(format!("{name} must be valid JSON: {error}")))
}

fn read_inputs(value: *const c_char) -> Result<Vec<String>> {
    if value.is_null() {
        return Ok(Vec::new());
    }
    let data = read_json(value, "inputs")?;
    serde_json::from_value(data).map_err(|error| {
        TinyOneError::runtime(format!("inputs must be a JSON string list: {error}"))
    })
}

fn program_payload(program: Program) -> Result<JsonValue> {
    Ok(json!({
        "artifact": program.to_artifact(),
        "fingerprint": program.fingerprint(),
    }))
}

fn run_compiled_program(program: &Program, mode: &str, inputs: Vec<String>) -> Result<JsonValue> {
    let mut stdout = Vec::new();
    let report = run_program_report(program, mode, &mut stdout, inputs)?;
    run_payload(
        stdout,
        report.memory,
        report.heap_before_shutdown,
        report.heap_after_shutdown,
    )
}

fn run_payload(
    stdout: Vec<u8>,
    memory: TinyMemory,
    heap_before_shutdown: TinyHeapStats,
    heap_after_shutdown: TinyHeapStats,
) -> Result<JsonValue> {
    let stdout = String::from_utf8(stdout)
        .map_err(|error| TinyOneError::runtime(format!("stdout was not UTF-8: {error}")))?;
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
        RuntimeValue::Int(value) => json!({"type": "int", "value": value}),
        RuntimeValue::Heap(reference) => json!({
            "type": "heap",
            "address": reference.address,
            "generation": reference.generation,
        }),
        RuntimeValue::Pointer(pointer) => json!({
            "type": "pointer",
            "address": pointer.address,
            "kind": pointer.kind,
            "index": pointer.index,
            "field": pointer.field,
            "generation": pointer.generation,
            "cast": pointer.cast,
        }),
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
