/*
 * tinyone.h — public C API for the TinyOne runtime library.
 *
 * ABI STATUS: UNSTABLE. Contracts defined below are subject to change until
 * a stable ABI is declared. Do not claim stable ABI compatibility.
 *
 * OWNERSHIP CONTRACT
 * ==================
 * Every function that returns `char *` returns a heap-allocated, NUL-terminated
 * UTF-8 string. The caller MUST free each returned string exactly once with
 * `tinyone_free_string`.
 * The caller MUST NOT call the C standard library `free()` directly on these
 * pointers — the allocator may differ.
 *
 * `tinyone_free_string(NULL)` is always a no-op; it is safe to call on any
 * null pointer.
 *
 * PARAMETER NULLABILITY
 * =====================
 * Unless noted as "(nullable)", every `const char *` parameter MUST be a
 * valid, NUL-terminated UTF-8 string. Passing NULL for a non-nullable parameter
 * returns a JSON error object rather than crashing; see each function.
 *
 * JSON RESPONSE FORMAT
 * ====================
 * All functions return a JSON string. Success:
 *   {"ok":true,"value":{...}}
 * Compile error:
 *   {"ok":false,"kind":"compile","error":"..."}
 * Runtime error:
 *   {"ok":false,"kind":"runtime","error":"..."}
 * Internal panic (should not occur; indicates a library bug). The panic is
 * caught at the FFI boundary and reported as JSON; Rust panics are not allowed
 * to unwind into C callers:
 *   {"ok":false,"kind":"panic","error":"..."}
 */

#ifndef TINYONE_H
#define TINYONE_H

#ifdef __cplusplus
extern "C" {
#endif

/**
 * Free a string returned by any tinyone_*_json function.
 *
 * Passing NULL is safe and is a no-op.
 * Do NOT call free() or any other allocator on pointers returned by this
 * library — use only this function.
 */
void tinyone_free_string(char *value);

/**
 * Lex `source` (non-nullable) and return a JSON token list.
 *
 * Returns: {"ok":true,"value":{"tokens":[...]}}
 *          {"ok":false,"kind":"compile","error":"..."}  on lex error
 *          {"ok":false,"kind":"compile","error":"..."}  if source is NULL
 *
 * The returned string must be freed with tinyone_free_string.
 */
char *tinyone_lex_source_json(const char *source);

/**
 * Compile TinyOne source `source` (non-nullable) to a bytecode artifact.
 *
 * Returns: {"ok":true,"value":{"artifact":{...},"fingerprint":"..."}}
 *          {"ok":false,"kind":"compile","error":"..."}  on compile error
 *
 * The returned string must be freed with tinyone_free_string.
 */
char *tinyone_compile_source_json(const char *source);

/**
 * Compile the TinyOne file at `path` (non-nullable) to a bytecode artifact.
 *
 * Returns: {"ok":true,"value":{"artifact":{...},"fingerprint":"..."}}
 *          {"ok":false,"kind":"compile","error":"..."}  on error
 *
 * The returned string must be freed with tinyone_free_string.
 */
char *tinyone_compile_file_json(const char *path);

/**
 * Compile and run TinyOne `source` (non-nullable) in `mode` (non-nullable).
 *
 * `mode` must be "vm" or "jit".
 * `inputs_json` is nullable. If non-NULL, it must be a JSON array of strings
 * representing the program's input queue. If NULL, an empty input queue is used.
 *
 * Returns: {"ok":true,"value":{"stdout":"...","memory":[...],...}}
 *          {"ok":false,"kind":"runtime","error":"..."}  on runtime error
 *          {"ok":false,"kind":"compile","error":"..."}  on compile error
 *
 * The returned string must be freed with tinyone_free_string.
 */
char *tinyone_run_source_json(
    const char *source,
    const char *mode,
    const char *inputs_json /* nullable */
);

/**
 * Compile and run the TinyOne file at `path` (non-nullable) in `mode`.
 *
 * See tinyone_run_source_json for parameter and return documentation.
 * The returned string must be freed with tinyone_free_string.
 */
char *tinyone_run_file_json(
    const char *path,
    const char *mode,
    const char *inputs_json /* nullable */
);

/**
 * Run a pre-compiled artifact (non-nullable JSON) in `mode` (non-nullable).
 *
 * `artifact_json` must be the raw artifact object from
 * tinyone_compile_*_json's `value.artifact` field, not the outer response
 * wrapper. `artifact_json` must be no larger than 8 MiB, excluding the trailing
 * NUL byte.
 * See tinyone_run_source_json for `inputs_json` and return documentation.
 * The returned string must be freed with tinyone_free_string.
 */
char *tinyone_run_artifact_json(
    const char *artifact_json,
    const char *mode,
    const char *inputs_json /* nullable */
);

/**
 * Return the JIT assembly listing for a pre-compiled artifact (non-nullable).
 *
 * `artifact_json` follows the same raw-artifact rule as
 * tinyone_run_artifact_json, including the 8 MiB byte limit.
 *
 * Returns: {"ok":true,"value":{"listing":"..."}}
 *          {"ok":false,"kind":"compile","error":"..."}  on error
 *
 * The returned string must be freed with tinyone_free_string.
 */
char *tinyone_jit_listing_json(const char *artifact_json);

#ifdef __cplusplus
}
#endif

#endif /* TINYONE_H */
