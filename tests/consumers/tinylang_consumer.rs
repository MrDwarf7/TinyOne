use std::ffi::c_char;

unsafe extern "C" {
    fn tinyone_abi_version() -> u32;
    fn tinyone_free_string(value: *mut c_char);
    fn tinyone_run_source_json(
        source: *const c_char,
        mode: *const c_char,
        inputs_json: *const c_char,
    ) -> *mut c_char;
}

fn main() {
    // This fixture is compile-checked in CI. Linking and execution are covered
    // by the C fixture because Rust's platform-specific cdylib link flags vary.
    let _ = (tinyone_abi_version, tinyone_free_string, tinyone_run_source_json);
}
