// Clippy pedantic: benchmark harness allows these design lints (large functions,
// struct bools, similar names) and numeric casts (timing math) — not production code.
#![allow(
    clippy::too_many_lines,
    clippy::struct_excessive_bools,
    clippy::similar_names,
    clippy::unnecessary_wraps,
    clippy::needless_pass_by_value,
    clippy::float_cmp,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_ptr_alignment,
    clippy::checked_conversions
)]

use std::fmt::Write as _;
use std::hint::black_box;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Barrier};
use std::thread::{self, JoinHandle};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use std::{env, fs};

use blake2::{Blake2b512, Digest};
use ralloc::{RallocBuffer, VmAllocator};
use serde_json::{Value as JsonValue, json};
use tinyone::{
    BytecodeVerifier,
    CompileCacheStatus,
    JitCache,
    JitOptions,
    JitProgram,
    Program,
    RuntimeValue,
    TinyMemory,
    TinyOneError,
    VerifiedProgram,
    compile_file_cached_verified_with_status,
    compile_file_verified,
    compile_source,
    compile_source_unoptimized,
    lex_source,
    optimize_program,
    run_source,
    run_verified_program,
};

static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(0);
const BENCHMARK_SCHEMA_VERSION: u32 = 3;
const MIN_DECISION_REPEATS: usize = 7;
const DEFAULT_CV_LIMIT: f64 = 10.0;
const HOT_LOOP_CV_LIMIT: f64 = 5.0;
const PRIORITY_3_MIN_IMPROVEMENT_PCT: f64 = 10.0;
const PRIORITY_3_ROWS: [(&str, &str); 3] = [
    ("runtime.jit_vec_push_pop_256", "vector push/pop"),
    ("runtime.jit_map_set_get_256", "map set/get"),
    ("runtime.jit_heap_churn", "heap churn"),
];
const PRIORITY_5_MAX_REGRESSION_PCT: f64 = 5.0;
const PRIORITY_5_GUARDRAIL_ROWS: [(&str, &str); 5] = [
    ("allocator.ralloc_buffer_64", "64-byte Ralloc allocation"),
    ("allocator.ralloc_buffer_4096", "4-KiB Ralloc allocation"),
    ("allocator.ralloc_resize_64_to_4096", "64-to-4096 Ralloc resize"),
    ("memory.reset_1024", "1,024-slot memory reset"),
    ("memory.snapshot_1024", "1,024-slot memory snapshot"),
];
const COLLECTION_SIZES: [usize; 3] = [16, 256, 4_096];
const MODULE_GRAPH_SIZES: [(&str, usize); 3] = [("small", 2), ("medium", 16), ("large", 64)];

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentThread() -> *mut std::ffi::c_void;
    fn QueryThreadCycleTime(thread: *mut std::ffi::c_void, cycles: *mut u64) -> i32;
    fn GetThreadTimes(
        thread: *mut std::ffi::c_void,
        creation: *mut FileTime,
        exit: *mut FileTime,
        kernel: *mut FileTime,
        user: *mut FileTime,
    ) -> i32;
}

#[cfg(windows)]
#[repr(C)]
#[derive(Default)]
struct FileTime {
    low:  u32,
    high: u32,
}

#[cfg(target_os = "linux")]
#[repr(C)]
struct LinuxTimespec {
    seconds:     std::ffi::c_long,
    nanoseconds: std::ffi::c_long,
}

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn clock_gettime(clock_id: std::ffi::c_int, time: *mut LinuxTimespec) -> std::ffi::c_int;
}

/// Returns scheduled CPU cycles for the benchmark thread on Windows. This is
/// a better signal for instruction-path work than wall time when the process
/// is briefly descheduled. Other platforms retain wall-clock measurements and
/// can use an external hardware-counter profiler.
#[cfg(windows)]
fn thread_cycle_count() -> Option<u64> {
    let mut cycles = 0u64;
    // SAFETY: GetCurrentThread returns a valid pseudo-handle for the calling
    // thread and `cycles` is a writable u64 for the duration of the call.
    let ok = unsafe { QueryThreadCycleTime(GetCurrentThread(), &mut cycles) };
    (ok != 0).then_some(cycles)
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn thread_cycle_count() -> Option<u64> {
    // SAFETY: LFENCE/RDTSC are available on x86_64. The fences keep the read
    // from moving across the measured region.
    unsafe {
        std::arch::x86_64::_mm_lfence();
        let cycles = std::arch::x86_64::_rdtsc();
        std::arch::x86_64::_mm_lfence();
        Some(cycles)
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86"))]
fn thread_cycle_count() -> Option<u64> {
    // SAFETY: LFENCE/RDTSC are available on supported x86 Linux targets.
    unsafe {
        std::arch::x86::_mm_lfence();
        let cycles = std::arch::x86::_rdtsc();
        std::arch::x86::_mm_lfence();
        Some(cycles)
    }
}

#[cfg(not(any(
    windows,
    all(target_os = "linux", target_arch = "x86_64"),
    all(target_os = "linux", target_arch = "x86")
)))]
fn thread_cycle_count() -> Option<u64> {
    None
}

#[cfg(windows)]
fn thread_cpu_time_ns() -> Option<u64> {
    let mut creation = FileTime::default();
    let mut exit = FileTime::default();
    let mut kernel = FileTime::default();
    let mut user = FileTime::default();
    // SAFETY: all FILETIME pointers are valid writable values and the thread
    // pseudo-handle refers to the calling benchmark thread.
    let ok = unsafe { GetThreadTimes(GetCurrentThread(), &mut creation, &mut exit, &mut kernel, &mut user) };
    if ok == 0 {
        return None;
    }
    let ticks = |time: FileTime| (u64::from(time.high) << 32) | u64::from(time.low);
    ticks(kernel)
        .checked_add(ticks(user))
        .and_then(|ticks| ticks.checked_mul(100))
}

#[cfg(target_os = "linux")]
fn thread_cpu_time_ns() -> Option<u64> {
    const CLOCK_THREAD_CPUTIME_ID: std::ffi::c_int = 3;
    let mut time = LinuxTimespec {
        seconds:     0,
        nanoseconds: 0,
    };
    // SAFETY: `time` is writable and CLOCK_THREAD_CPUTIME_ID is the Linux
    // per-thread CPU clock, available without perf-event permissions.
    let ok = unsafe { clock_gettime(CLOCK_THREAD_CPUTIME_ID, &raw mut time) };
    if ok != 0 || time.seconds < 0 || time.nanoseconds < 0 {
        return None;
    }
    u64::try_from(time.seconds)
        .ok()?
        .checked_mul(1_000_000_000)?
        .checked_add(u64::try_from(time.nanoseconds).ok()?)
}

#[cfg(not(any(windows, target_os = "linux")))]
fn thread_cpu_time_ns() -> Option<u64> {
    None
}

fn cycle_counter_kind() -> &'static str {
    if cfg!(windows) {
        "scheduled-thread"
    } else if cfg!(all(target_os = "linux", any(target_arch = "x86", target_arch = "x86_64"))) {
        "tsc"
    } else {
        "none"
    }
}

const STRAIGHTLINE_SOURCE: &str = r"
let a = 1
let b = a + 2
let c = b * 3
let d = c - a
let e = d / 2
print e
print e >= 4
";

const LOOP_SOURCE: &str = r"
let i = 0
let total = 0
while i < 128 {
  total = total + (i * 3)
  i = i + 1
}
print total
";

const HOT_LOOP_SOURCE: &str = r"
let i = 0
let total = 0
while i < 4096 {
  total = total + (i * 3)
  i = i + 1
}
print total
";

const SLOT_COMPARE_SOURCE: &str = r"
let i = 0
while i < 4096 {
  i = i + 1
}
print i
";

const SLOT_MUL_SOURCE: &str = r"
let i = 0
let value = 7
while i < 4096 {
  value = value * 1
  i = i + 1
}
print value
";

const SLOT_DIV_SOURCE: &str = r"
let i = 0
let value = -7
while i < 4096 {
  value = value / 1
  i = i + 1
}
print value
";

const FUNCTION_SOURCE: &str = r"
fn mul_by_count(value, count) {
  let acc = 0
  while count > 0 {
    acc = acc + value
    count = count - 1
  }
  return acc
}

fn pair(x) {
  return mul_by_count(x, 2) + mul_by_count(x + 1, 3)
}

let i = 1
let total = 0
while i <= 32 {
  total = total + pair(i)
  i = i + 1
}
print total
";

const CONTROL_INTERRUPT_SOURCE: &str = r"
let i = 0
let pulses = 0
while i < 96 {
  let gate = 1
  while gate {
    pulses = pulses + i
    gate = 0
  }
  i = i + 1
}
print pulses
";

const HEAP_SOURCE: &str = r#"
struct Point { x, y }
let values = [1, 2, 3, 4, 5]
let i = 0
while i < len(values) {
  set values[i] = values[i] * 3
  i = i + 1
}
let point = Point(values[1], len("tinyone"))
set point.y = point.y + values[4]
print point.x
print point.y
print values
"#;

const INPUT_SOURCE: &str = r"
let value = read_int()
let ptr = alloc(value)
print store(ptr, load(ptr) + 1)
let ignored = unsafe free(ptr)
";

const BUILTIN_HEAVY_SOURCE: &str = r"
let arr = array(16, 0)
let i = 0
while i < len(arr) {
  set arr[i] = to_int(i * 7)
  i = i + 1
}
let total = 0
let j = 0
while j < len(arr) {
  total = total + arr[j]
  j = j + 1
}
print total
";

const VEC_SOURCE: &str = r"
let values = vec_new()
let i = 0
while i < 256 {
  let ignored = push(values, i)
  i = i + 1
}
let total = 0
while len(values) > 0 {
  total = total + pop(values)
}
print total
";

const MAP_SOURCE: &str = r"
let values = map_new()
let i = 0
while i < 128 {
  let ignored = map_set(values, i, i * 3)
  i = i + 1
}
let total = 0
let j = 0
while j < 128 {
  total = total + map_get(values, j)
  j = j + 1
}
print total
";

const HEAP_CHURN_SOURCE: &str = r"
let i = 0
let total = 0
while i < 256 {
  let cell = alloc(i)
  total = total + load(cell)
  let ignored = unsafe free(cell)
  i = i + 1
}
print total
";

const MODULE_MAIN_SOURCE: &str = r#"
import "math.to" as math
let total = 0
let i = 0
while i < 64 {
  total = total + math.add(i, 2)
  i = i + 1
}
print total
"#;

const MODULE_SOURCE: &str = r"
fn normalize(value) {
  return value
}

export fn add(left, right) {
  return normalize(left) + right
}
";

fn leak_name(value: String) -> &'static str {
    Box::leak(value.into_boxed_str())
}

fn workload_iterations(size: usize) -> u64 {
    match size {
        0..=16 => 2_000,
        17..=256 => 500,
        _ => 50,
    }
}

fn vec_push_pop_source(size: usize) -> String {
    format!(
        r"
let values = vec_new()
let i = 0
while i < {size} {{
  let ignored = push(values, i)
  i = i + 1
}}
let total = 0
while len(values) > 0 {{
  total = total + pop(values)
}}
"
    )
}

fn map_set_get_source(size: usize) -> String {
    format!(
        r"
let values = map_new()
let i = 0
while i < {size} {{
  let ignored = map_set(values, i, i * 3)
  i = i + 1
}}
let total = 0
let j = 0
while j < {size} {{
  total = total + map_get(values, j)
  j = j + 1
}}
"
    )
}

fn vector_phase_source(phase: &str, size: usize) -> String {
    match phase {
        "push_in_capacity" => {
            format!(
                r"
let values = vec_new()
let i = 0
while i < {size} {{
  let ignored = push(values, i)
  i = i + 1
}}
while len(values) > 0 {{
  let ignored = pop(values)
}}
let j = 0
while j < {size} {{
  let ignored = push(values, j)
  j = j + 1
}}
"
            )
        }
        "capacity_growth" => {
            format!(
                r"
let values = vec_new()
let i = 0
while i < {size} {{
  let ignored = push(values, i)
  i = i + 1
}}
"
            )
        }
        "pop" => {
            format!(
                r"
let values = vec_new()
let i = 0
while i < {size} {{
  let ignored = push(values, i)
  i = i + 1
}}
while len(values) > 0 {{
  let ignored = pop(values)
}}
"
            )
        }
        "clear" => {
            format!(
                r"
let values = vec_new()
let i = 0
while i < {size} {{
  let ignored = push(values, i)
  i = i + 1
}}
let ignored = vec_clear(values)
"
            )
        }
        _ => unreachable!("unknown vector phase"),
    }
}

fn map_phase_source(phase: &str, size: usize) -> String {
    let setup = format!(
        r"
let values = map_new()
let i = 0
while i < {size} {{
  let ignored = map_set(values, i, i * 3)
  i = i + 1
}}
"
    );
    match phase {
        "hit" => {
            format!(
                r"{setup}
let key = 0
let total = 0
let j = 0
while j < 4096 {{
  total = total + map_get(values, key)
  key = key + 1
  if key == {size} {{
    key = 0
  }}
  j = j + 1
}}
"
            )
        }
        "miss" => {
            format!(
                r"{setup}
let misses = 0
let j = 0
while j < 4096 {{
  misses = misses + map_has(values, {size} + j)
  j = j + 1
}}
"
            )
        }
        "update" => {
            format!(
                r"{setup}
let key = 0
let j = 0
while j < 4096 {{
  let ignored = map_set(values, key, j)
  key = key + 1
  if key == {size} {{
    key = 0
  }}
  j = j + 1
}}
"
            )
        }
        "insert_in_capacity" => {
            format!(
                r"{setup}
let j = 0
while j < {size} {{
  let ignored = map_del(values, j)
  j = j + 1
}}
let k = 0
while k < {size} {{
  let ignored = map_set(values, k, k)
  k = k + 1
}}
"
            )
        }
        "delete" => {
            format!(
                r"{setup}
let j = 0
while j < {size} {{
  let ignored = map_del(values, j)
  j = j + 1
}}
"
            )
        }
        "capacity_growth" => setup,
        "pointer_key_validation" => {
            format!(
                r"
let values = map_new()
let pointers = vec_new()
let i = 0
while i < {size} {{
  let pointer = alloc(i)
  let ignored1 = push(pointers, pointer)
  let ignored2 = map_set(values, pointer, i)
  i = i + 1
}}
let key = 0
let total = 0
let j = 0
while j < 256 {{
  total = total + map_get(values, pointers[key])
  key = key + 1
  if key == {size} {{
    key = 0
  }}
  j = j + 1
}}
"
            )
        }
        _ => unreachable!("unknown map phase"),
    }
}

fn heap_phase_source(phase: &str, count: usize) -> String {
    match phase {
        "allocation" => {
            format!(
                r"
let pointers = vec_new()
let i = 0
while i < {count} {{
  let ignored = push(pointers, alloc(i))
  i = i + 1
}}
"
            )
        }
        "lookup" => {
            format!(
                r"
let values = array(1, 0)
let total = 0
let i = 0
while i < {count} {{
  total = total + len(values)
  i = i + 1
}}
"
            )
        }
        "load" => {
            format!(
                r"
let cell = alloc(7)
let total = 0
let i = 0
while i < {count} {{
  total = total + load(cell)
  i = i + 1
}}
"
            )
        }
        "store" => {
            format!(
                r"
let cell = alloc(0)
let i = 0
while i < {count} {{
  let ignored = store(cell, i)
  i = i + 1
}}
"
            )
        }
        "free" => {
            format!(
                r"
let pointers = vec_new()
let i = 0
while i < {count} {{
  let ignored = push(pointers, alloc(i))
  i = i + 1
}}
let j = 0
while j < {count} {{
  let ignored = unsafe free(pointers[j])
  j = j + 1
}}
"
            )
        }
        "slot_reuse" => {
            format!(
                r"
let i = 0
while i < {count} {{
  let cell = alloc(i)
  let ignored = unsafe free(cell)
  i = i + 1
}}
"
            )
        }
        _ => unreachable!("unknown heap phase"),
    }
}

#[derive(Clone)]
struct CorrectnessCase {
    name:     &'static str,
    source:   &'static str,
    expected: &'static str,
    inputs:   Vec<String>,
    mode:     &'static str,
}

impl CorrectnessCase {
    fn new(name: &'static str, source: &'static str, expected: &'static str) -> Self {
        Self {
            name,
            source,
            expected,
            inputs: Vec::new(),
            mode: "jit",
        }
    }

    fn mode(mut self, mode: &'static str) -> Self {
        self.mode = mode;
        self
    }

    fn inputs(mut self, inputs: &[&str]) -> Self {
        self.inputs = inputs.iter().map(std::string::ToString::to_string).collect();
        self
    }
}

#[derive(Default)]
struct Sink;

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Owns a small multi-file source tree for file compiler/cache benchmarks.
/// Keeping cleanup in `Drop` makes `build_benchmarks` safe to use from tests.
struct FileFixture {
    directory: PathBuf,
    main:      PathBuf,
    inputs:    Vec<PathBuf>,
}

fn benchmark_fixture_root() -> PathBuf {
    env::var_os("TINYONE_BENCH_FIXTURE_ROOT").map_or_else(env::temp_dir, PathBuf::from)
}

impl FileFixture {
    fn new(label: &str) -> Self {
        let id = NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let root = benchmark_fixture_root();
        fs::create_dir_all(&root).expect("create benchmark fixture root");
        let directory = root.join(format!("tinyone-bench-{label}-{}-{id}", std::process::id()));
        fs::create_dir(&directory).expect("create benchmark fixture directory");
        let main = directory.join("main.to");
        fs::write(&main, MODULE_MAIN_SOURCE).expect("write benchmark main module");
        let module = directory.join("math.to");
        fs::write(&module, MODULE_SOURCE).expect("write benchmark imported module");
        Self {
            directory,
            main: main.clone(),
            inputs: vec![main, module],
        }
    }

    fn new_graph(label: &str, module_count: usize) -> Self {
        let id = NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let root = benchmark_fixture_root();
        fs::create_dir_all(&root).expect("create benchmark fixture root");
        let directory = root.join(format!("tinyone-bench-{label}-{}-{id}", std::process::id()));
        fs::create_dir(&directory).expect("create benchmark graph directory");

        let mut main_source = String::new();
        let mut inputs = Vec::with_capacity(module_count + 1);
        for index in 0..module_count {
            let module_name = format!("module_{index:03}");
            writeln!(main_source, "import \"{module_name}.to\" as {module_name}").unwrap();
            let module_path = directory.join(format!("{module_name}.to"));
            fs::write(&module_path, format!("export fn value(input) {{ return input + {index} }}\n"))
                .expect("write benchmark graph module");
            inputs.push(module_path);
        }
        main_source.push_str("let total = 0\n");
        for index in 0..module_count {
            writeln!(main_source, "total = total + module_{index:03}.value({index})").unwrap();
        }

        let main = directory.join("main.to");
        fs::write(&main, main_source).expect("write benchmark graph root");
        inputs.insert(0, main.clone());
        Self {
            directory,
            main,
            inputs,
        }
    }

    fn main(&self) -> &Path {
        &self.main
    }

    fn inputs(&self) -> &[PathBuf] {
        &self.inputs
    }

    fn cache_file(&self, extension: &str) -> PathBuf {
        let directory = self.directory.join(".tinyone-cache");
        fs::read_dir(&directory)
            .expect("read benchmark cache directory")
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .find(|path| path.extension().and_then(|value| value.to_str()) == Some(extension))
            .unwrap_or_else(|| panic!("benchmark cache has no {extension} artifact"))
    }
}

impl Drop for FileFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

#[derive(Clone, Copy)]
enum AllocatorWorkerCommand {
    Run,
    Stop,
}

struct AllocatorContention {
    commands:  Vec<Sender<AllocatorWorkerCommand>>,
    completed: Receiver<()>,
    start:     Arc<Barrier>,
    workers:   Vec<JoinHandle<()>>,
}

impl AllocatorContention {
    fn new(worker_count: usize, allocations_per_worker: usize, allocation_bytes: usize) -> Self {
        let start = Arc::new(Barrier::new(worker_count + 1));
        let (completed_tx, completed) = mpsc::channel();
        let mut commands = Vec::with_capacity(worker_count);
        let mut workers = Vec::with_capacity(worker_count);
        for worker_index in 0..worker_count {
            let (command_tx, command_rx) = mpsc::channel();
            commands.push(command_tx);
            let completed_tx = completed_tx.clone();
            let start = Arc::clone(&start);
            workers.push(thread::spawn(move || {
                while let Ok(command) = command_rx.recv() {
                    match command {
                        AllocatorWorkerCommand::Run => {
                            start.wait();
                            for _ in 0..allocations_per_worker {
                                let mut buffer =
                                    RallocBuffer::try_new(allocation_bytes).expect("allocate contended Ralloc buffer");
                                buffer.as_mut_slice()[0] = worker_index as u8;
                                black_box(buffer.as_ptr());
                            }
                            completed_tx.send(()).expect("report allocator work");
                        }
                        AllocatorWorkerCommand::Stop => break,
                    }
                }
            }));
        }
        Self {
            commands,
            completed,
            start,
            workers,
        }
    }

    fn run_round(&self) {
        for command in &self.commands {
            command
                .send(AllocatorWorkerCommand::Run)
                .expect("start allocator worker");
        }
        self.start.wait();
        for _ in &self.commands {
            self.completed.recv().expect("join allocator worker round");
        }
    }
}

impl Drop for AllocatorContention {
    fn drop(&mut self) {
        for command in &self.commands {
            let _ = command.send(AllocatorWorkerCommand::Stop);
        }
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

#[derive(Clone)]
struct Fixture {
    raw:      Arc<Program>,
    program:  Arc<Program>,
    verified: VerifiedProgram,
    artifact: JsonValue,
}

fn make_fixture(source: &str) -> Fixture {
    let raw = compile_source_unoptimized(source).expect("fixture should compile");
    let program = optimize_program(Arc::clone(&raw));
    let verified = VerifiedProgram::verify((*program).clone()).expect("fixture should verify");
    let artifact = program.to_artifact();
    Fixture {
        raw,
        program,
        verified,
        artifact,
    }
}

struct Benchmark {
    name:       &'static str,
    iterations: u64,
    run:        Box<dyn FnMut()>,
}

fn bench(name: &'static str, iterations: u64, run: impl FnMut() + 'static) -> Benchmark {
    Benchmark {
        name,
        iterations,
        run: Box::new(run),
    }
}

#[derive(Clone)]
struct BenchmarkResult {
    name:        &'static str,
    iterations:  u64,
    best_ns:     f64,
    mean_ns:     f64,
    stdev_ns:    f64,
    best_cycles: Option<f64>,
    mean_cycles: Option<f64>,
    best_cpu_ns: Option<f64>,
    mean_cpu_ns: Option<f64>,
}

impl BenchmarkResult {
    fn best_per_iter_ns(&self) -> f64 {
        self.best_ns / self.iterations as f64
    }

    fn mean_per_iter_ns(&self) -> f64 {
        self.mean_ns / self.iterations as f64
    }

    fn best_per_iter_cycles(&self) -> Option<f64> {
        self.best_cycles.map(|cycles| cycles / self.iterations as f64)
    }

    fn mean_per_iter_cycles(&self) -> Option<f64> {
        self.mean_cycles.map(|cycles| cycles / self.iterations as f64)
    }

    fn best_per_iter_cpu_ns(&self) -> Option<f64> {
        self.best_cpu_ns.map(|nanoseconds| nanoseconds / self.iterations as f64)
    }

    fn mean_per_iter_cpu_ns(&self) -> Option<f64> {
        self.mean_cpu_ns.map(|nanoseconds| nanoseconds / self.iterations as f64)
    }

    fn cv_pct(&self) -> f64 {
        if self.mean_ns == 0.0 {
            0.0
        } else {
            (self.stdev_ns / self.mean_ns) * 100.0
        }
    }

    fn to_json(&self) -> JsonValue {
        json!({
            "name": self.name,
            "iterations": self.iterations,
            "best_per_iter_ns": self.best_per_iter_ns(),
            "mean_per_iter_ns": self.mean_per_iter_ns(),
            "best_cycles_per_iter": self.best_per_iter_cycles(),
            "mean_cycles_per_iter": self.mean_per_iter_cycles(),
            "best_cpu_time_per_iter_ns": self.best_per_iter_cpu_ns(),
            "mean_cpu_time_per_iter_ns": self.mean_per_iter_cpu_ns(),
            "cv_pct": self.cv_pct(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EvidenceQuality {
    decision_eligible: bool,
    rejections:        Vec<String>,
}

impl EvidenceQuality {
    fn accepted() -> Self {
        Self {
            decision_eligible: true,
            rejections:        Vec::new(),
        }
    }

    fn rejected(rejections: Vec<String>) -> Self {
        Self {
            decision_eligible: false,
            rejections,
        }
    }
}

fn cv_limit_for(name: &str) -> f64 {
    if name.contains("hot_loop") {
        HOT_LOOP_CV_LIMIT
    } else {
        DEFAULT_CV_LIMIT
    }
}

fn assess_current_evidence(args: &Args, results: &[BenchmarkResult], correctness_checked: bool) -> EvidenceQuality {
    let mut rejections = Vec::new();
    if !correctness_checked {
        rejections.push("pre-timing correctness checks were skipped".to_string());
    }
    if args.quick {
        rejections.push("--quick is not a decision-grade measurement mode".to_string());
    }
    if args.repeats < MIN_DECISION_REPEATS {
        rejections.push(format!("{} repeats is below the decision minimum of {MIN_DECISION_REPEATS}", args.repeats));
    }
    for result in results {
        let limit = cv_limit_for(result.name);
        if result.cv_pct() > limit {
            rejections.push(format!("{} has {:.2}% CV, above its {:.0}% limit", result.name, result.cv_pct(), limit));
        }
    }
    if rejections.is_empty() {
        EvidenceQuality::accepted()
    } else {
        EvidenceQuality::rejected(rejections)
    }
}

#[derive(Clone)]
struct Args {
    quick:              bool,
    json:               bool,
    filter:             String,
    repeats:            usize,
    sample_scale:       u64,
    skip_correctness:   bool,
    correctness_only:   bool,
    baseline:           Option<String>,
    save_baseline:      Option<String>,
    save_baseline_auto: bool,
    priority_3_only:    bool,
    priority_3_gate:    bool,
    priority_5_only:    bool,
    priority_5_gate:    bool,
    machine_label:      Option<String>,
    power_policy:       Option<String>,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            quick:              false,
            json:               false,
            filter:             String::new(),
            repeats:            5,
            sample_scale:       1,
            skip_correctness:   false,
            correctness_only:   false,
            baseline:           None,
            save_baseline:      None,
            save_baseline_auto: false,
            priority_3_only:    false,
            priority_3_gate:    false,
            priority_5_only:    false,
            priority_5_gate:    false,
            machine_label:      None,
            power_policy:       None,
        }
    }
}

fn parse_args(argv: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut args = Args::default();
    let mut iter = argv.into_iter();
    let _ = iter.next();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            "--quick" => args.quick = true,
            "--json" => args.json = true,
            "--skip-correctness" => args.skip_correctness = true,
            "--correctness-only" => args.correctness_only = true,
            "--filter" => args.filter = iter.next().ok_or("--filter requires text")?,
            "--repeats" => {
                args.repeats = iter
                    .next()
                    .ok_or("--repeats requires a value")?
                    .parse()
                    .map_err(|_| "--repeats must be an integer".to_string())?;
                if args.repeats == 0 {
                    return Err("--repeats must be >= 1".to_string());
                }
            }
            "--sample-scale" => {
                args.sample_scale = iter
                    .next()
                    .ok_or("--sample-scale requires a value")?
                    .parse()
                    .map_err(|_| "--sample-scale must be an integer".to_string())?;
                if args.sample_scale == 0 {
                    return Err("--sample-scale must be >= 1".to_string());
                }
            }
            "--baseline" => {
                args.baseline = Some(iter.next().ok_or("--baseline requires a path")?);
            }
            "--save-baseline" => {
                args.save_baseline = Some(iter.next().ok_or("--save-baseline requires a path")?);
            }
            "--save-baseline-auto" => args.save_baseline_auto = true,
            "--priority-3-only" => args.priority_3_only = true,
            "--priority-3-gate" => args.priority_3_gate = true,
            "--priority-5-only" => args.priority_5_only = true,
            "--priority-5-gate" => args.priority_5_gate = true,
            "--machine-label" => {
                args.machine_label = Some(iter.next().ok_or("--machine-label requires text")?);
            }
            "--power-policy" => {
                args.power_policy = Some(iter.next().ok_or("--power-policy requires text")?);
            }
            _ => return Err(format!("unknown option {arg}")),
        }
    }
    Ok(args)
}

fn print_help() {
    println!(
        "usage: tinylang-bench [--quick] [--filter TEXT] [--repeats N] [--sample-scale N] \\
         [--skip-correctness] [--correctness-only] [--json] \\
         [--baseline PATH] [--save-baseline PATH] [--save-baseline-auto] \\
         [--priority-3-only] [--priority-3-gate] [--priority-5-only] [--priority-5-gate] \\
         [--machine-label TEXT] [--power-policy TEXT]"
    );
}

fn command_stdout(program: &str, args: &[&str], directory: Option<&Path>) -> Option<String> {
    let mut command = Command::new(program);
    command.args(args);
    if let Some(directory) = directory {
        command.current_dir(directory);
    }
    let output = command.output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(windows)]
fn cpu_model() -> String {
    env::var("PROCESSOR_IDENTIFIER").unwrap_or_else(|_| "unknown".to_string())
}

#[cfg(target_os = "linux")]
fn cpu_model() -> String {
    fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|text| {
            text.lines().find_map(|line| {
                let (key, value) = line.split_once(':')?;
                matches!(key.trim(), "model name" | "Hardware").then(|| value.trim().to_string())
            })
        })
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(not(any(windows, target_os = "linux")))]
fn cpu_model() -> String {
    "unknown".to_string()
}

fn filesystem_context_for(path: &Path) -> String {
    if cfg!(windows) {
        return "windows-native".to_string();
    }
    if cfg!(target_os = "linux") {
        let is_wsl =
            fs::read_to_string("/proc/version").is_ok_and(|text| text.to_ascii_lowercase().contains("microsoft"));
        if is_wsl {
            let mounted_windows = path.starts_with("/mnt");
            return if mounted_windows {
                "wsl-mounted-windows-filesystem"
            } else {
                "wsl-native-filesystem"
            }
            .to_string();
        }
        return "linux-native".to_string();
    }
    "unknown".to_string()
}

fn filesystem_context() -> String {
    env::current_dir().map_or_else(|_| "unknown".to_string(), |path| filesystem_context_for(&path))
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")))
        .to_path_buf()
}

fn git_metadata() -> (String, bool) {
    let root = repository_root();
    let commit = command_stdout("git", &["rev-parse", "HEAD"], Some(&root))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    let dirty = match command_stdout("git", &["status", "--porcelain", "--untracked-files=normal"], Some(&root)) {
        Some(value) => !value.is_empty(),
        None => true,
    };
    (commit, dirty)
}

fn run_metadata(args: &Args, correctness_checked: bool, evidence_quality: &EvidenceQuality) -> JsonValue {
    let (git_commit, git_dirty) = git_metadata();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    let fixture_root = benchmark_fixture_root();
    let fixture_root = fixture_root.canonicalize().unwrap_or(fixture_root);
    json!({
        "timestamp_unix_seconds": timestamp,
        "package_version": env!("CARGO_PKG_VERSION"),
        "build_profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "os": env::consts::OS,
        "architecture": env::consts::ARCH,
        "cpu_model": cpu_model(),
        "rust_version": command_stdout("rustc", &["--version"], None)
            .unwrap_or_else(|| "unknown".to_string()),
        "git_commit": git_commit,
        "git_dirty": git_dirty,
        "filesystem_context": filesystem_context(),
        "fixture_filesystem_context": filesystem_context_for(&fixture_root),
        "fixture_root": fixture_root,
        "machine_label": args.machine_label,
        "power_policy": args.power_policy,
        "benchmark_options": {
            "filter": args.filter,
            "repeats": args.repeats,
            "sample_scale": args.sample_scale,
            "quick": args.quick,
            "priority_3_only": args.priority_3_only,
            "priority_5_only": args.priority_5_only,
            "correctness_checked": correctness_checked,
            "thread_cpu_time": thread_cpu_time_ns().is_some(),
            "cycle_counter": cycle_counter_kind(),
        },
        "evidence_quality": {
            "decision_eligible": evidence_quality.decision_eligible,
            "rejections": evidence_quality.rejections,
        },
    })
}

fn benchmark_document(results: &[BenchmarkResult], metadata: JsonValue) -> JsonValue {
    json!({
        "schema_version": BENCHMARK_SCHEMA_VERSION,
        "metadata": metadata,
        "benchmarks": results.iter().map(BenchmarkResult::to_json).collect::<Vec<_>>(),
    })
}

fn automatic_baseline_path(metadata: &JsonValue) -> PathBuf {
    let filesystem = metadata
        .get("filesystem_context")
        .and_then(JsonValue::as_str)
        .unwrap_or("unknown")
        .replace(|character: char| !character.is_ascii_alphanumeric(), "-");
    let platform = format!("{}-{}-{filesystem}", env::consts::OS, env::consts::ARCH);
    let commit = metadata
        .get("git_commit")
        .and_then(JsonValue::as_str)
        .unwrap_or("unknown");
    let short_commit = commit.get(..12.min(commit.len())).unwrap_or(commit);
    let timestamp = metadata
        .get("timestamp_unix_seconds")
        .and_then(JsonValue::as_u64)
        .unwrap_or(0);
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("perf")
        .join(platform)
        .join(format!("baseline-{short_commit}-{timestamp}.json"))
}

fn correctness_cases() -> Vec<CorrectnessCase> {
    vec![
        CorrectnessCase::new("straightline/jit", STRAIGHTLINE_SOURCE, "4\ntrue\n"),
        CorrectnessCase::new("straightline/vm", STRAIGHTLINE_SOURCE, "4\ntrue\n").mode("vm"),
        CorrectnessCase::new("loop/jit", LOOP_SOURCE, "24384\n"),
        CorrectnessCase::new("loop/vm", LOOP_SOURCE, "24384\n").mode("vm"),
        CorrectnessCase::new("hot-loop/jit", HOT_LOOP_SOURCE, "25159680\n"),
        CorrectnessCase::new("hot-loop/vm", HOT_LOOP_SOURCE, "25159680\n").mode("vm"),
        CorrectnessCase::new("functions/jit", FUNCTION_SOURCE, "2736\n"),
        CorrectnessCase::new("functions/vm", FUNCTION_SOURCE, "2736\n").mode("vm"),
        CorrectnessCase::new("interrupts/jit", CONTROL_INTERRUPT_SOURCE, "4560\n"),
        CorrectnessCase::new("interrupts/vm", CONTROL_INTERRUPT_SOURCE, "4560\n").mode("vm"),
        CorrectnessCase::new("heap/jit", HEAP_SOURCE, "6\n22\n[3, 6, 9, 12, 15]\n"),
        CorrectnessCase::new("heap/vm", HEAP_SOURCE, "6\n22\n[3, 6, 9, 12, 15]\n").mode("vm"),
        CorrectnessCase::new("input/jit", INPUT_SOURCE, "42\n").inputs(&["41"]),
        CorrectnessCase::new("input/vm", INPUT_SOURCE, "42\n")
            .inputs(&["41"])
            .mode("vm"),
        CorrectnessCase::new("builtins/jit", BUILTIN_HEAVY_SOURCE, "840\n"),
        CorrectnessCase::new("builtins/vm", BUILTIN_HEAVY_SOURCE, "840\n").mode("vm"),
        CorrectnessCase::new("vec/jit", VEC_SOURCE, "32640\n"),
        CorrectnessCase::new("vec/vm", VEC_SOURCE, "32640\n").mode("vm"),
        CorrectnessCase::new("map/jit", MAP_SOURCE, "24384\n"),
        CorrectnessCase::new("map/vm", MAP_SOURCE, "24384\n").mode("vm"),
        CorrectnessCase::new("heap-churn/jit", HEAP_CHURN_SOURCE, "32640\n"),
        CorrectnessCase::new("heap-churn/vm", HEAP_CHURN_SOURCE, "32640\n").mode("vm"),
    ]
}

fn run_correctness_checks(cases: &[CorrectnessCase]) -> usize {
    let mut failures = 0usize;
    for case in cases {
        let mut stdout = Vec::new();
        match run_source(case.source, case.mode, &mut stdout, case.inputs.clone()) {
            Ok(_) => {
                let actual = String::from_utf8(stdout).expect("TinyOne output is UTF-8");
                if actual == case.expected {
                    println!("  pass  {}", case.name);
                } else {
                    failures += 1;
                    println!("  FAIL  {}  expected {:?} got {:?}", case.name, case.expected, actual);
                }
            }
            Err(error) => {
                failures += 1;
                println!("  FAIL  {}  raised {}", case.name, error);
            }
        }
    }
    failures
}

fn run_verified_mode(program: &VerifiedProgram, mode: &str, inputs: Vec<String>) {
    let mut sink = Sink;
    black_box(run_verified_program(program, mode, &mut sink, inputs).expect("benchmark program should run"));
}

fn run_compiled_jit(program: &mut JitProgram, inputs: Vec<String>) {
    let mut sink = Sink;
    black_box(
        program
            .run(&mut sink, inputs)
            .expect("benchmark JIT program should run"),
    );
}

fn run_source_mode(source: &str, mode: &str, inputs: Vec<String>) {
    let mut sink = Sink;
    black_box(run_source(source, mode, &mut sink, inputs).expect("benchmark source should run"));
}

fn compile_jit(program: &Arc<Program>, cache: &mut JitCache) {
    let compiled = std::ptr::from_ref(cache.compile(program).expect("benchmark program should compile"));
    black_box(compiled);
}

fn compile_jit_verified(program: &VerifiedProgram, cache: &mut JitCache) {
    let compiled = std::ptr::from_ref(
        cache
            .compile_verified(program)
            .expect("verified benchmark program should compile"),
    );
    black_box(compiled);
}

fn run_source_jit_warm(source: &str, cache: &mut JitCache, inputs: Vec<String>) {
    let mut sink = Sink;
    black_box(
        cache
            .run_source(source, &mut sink, inputs)
            .expect("benchmark source should run"),
    );
}

fn run_source_jit_cold(source: &str, inputs: Vec<String>) {
    let mut cache = JitCache::new();
    run_source_jit_warm(source, &mut cache, inputs);
}

fn runtime_pair(label: &str, source: String, iterations: u64) -> Vec<Benchmark> {
    let fixture = make_fixture(&source);
    let vm_name = leak_name(format!("runtime.vm_{label}"));
    let jit_name = leak_name(format!("runtime.jit_{label}"));
    let vm_program = fixture.verified.clone();
    let mut jit_program =
        JitProgram::compile_verified(&fixture.verified).expect("attribution benchmark should compile");
    vec![
        bench(vm_name, iterations, move || run_verified_mode(&vm_program, "vm", Vec::new())),
        bench(jit_name, iterations, move || run_compiled_jit(&mut jit_program, Vec::new())),
    ]
}

fn collection_and_heap_benchmarks() -> Vec<Benchmark> {
    let mut benchmarks = Vec::new();
    for size in COLLECTION_SIZES {
        benchmarks.extend(runtime_pair(
            &format!("vec_push_pop_{size}"),
            vec_push_pop_source(size),
            workload_iterations(size),
        ));
        benchmarks.extend(runtime_pair(
            &format!("map_set_get_{size}"),
            map_set_get_source(size),
            workload_iterations(size),
        ));
    }

    for phase in ["push_in_capacity", "capacity_growth", "pop", "clear"] {
        let size = if phase == "capacity_growth" { 4_096 } else { 256 };
        benchmarks.extend(runtime_pair(
            &format!("vec_{phase}_{size}"),
            vector_phase_source(phase, size),
            workload_iterations(size),
        ));
    }

    for phase in [
        "hit",
        "miss",
        "update",
        "insert_in_capacity",
        "delete",
        "capacity_growth",
        "pointer_key_validation",
    ] {
        let size = if phase == "capacity_growth" { 4_096 } else { 256 };
        benchmarks.extend(runtime_pair(
            &format!("map_{phase}_{size}"),
            map_phase_source(phase, size),
            workload_iterations(size),
        ));
    }

    for phase in ["allocation", "lookup", "load", "store", "free", "slot_reuse"] {
        benchmarks.extend(runtime_pair(&format!("heap_{phase}_256"), heap_phase_source(phase, 256), 500));
    }
    benchmarks
}

fn module_graph_benchmarks() -> Vec<Benchmark> {
    let mut benchmarks = Vec::new();
    for (label, module_count) in MODULE_GRAPH_SIZES {
        let iterations = match module_count {
            0..=2 => 1_000,
            3..=16 => 200,
            _ => 50,
        };
        let uncached_name = leak_name(format!("compiler.module_graph_{label}_uncached"));
        benchmarks.push(bench(uncached_name, iterations, {
            let fixture = FileFixture::new_graph(&format!("graph-{label}-uncached"), module_count);
            move || {
                black_box(compile_file_verified(fixture.main()).expect("compile benchmark module graph"));
            }
        }));

        let policy_bypasses = module_count <= 2
            || (cfg!(windows) && (3..=16).contains(&module_count))
            || filesystem_context_for(&benchmark_fixture_root()) == "wsl-mounted-windows-filesystem";
        let (cache_label, expected_status) = if policy_bypasses {
            ("cache_bypass", CompileCacheStatus::Bypassed)
        } else {
            ("cache_hit", CompileCacheStatus::Hit)
        };
        let cached_name = leak_name(format!("compiler.module_graph_{label}_{cache_label}"));
        benchmarks.push(bench(cached_name, iterations, {
            let fixture = FileFixture::new_graph(&format!("graph-{label}-cached"), module_count);
            let (_, status) =
                compile_file_cached_verified_with_status(fixture.main()).expect("prime benchmark graph cache");
            let primed_status = if expected_status == CompileCacheStatus::Bypassed {
                CompileCacheStatus::Bypassed
            } else {
                CompileCacheStatus::Miss
            };
            assert_eq!(status, primed_status);
            move || {
                let (program, status) = compile_file_cached_verified_with_status(fixture.main())
                    .expect("load benchmark module graph cache");
                assert_eq!(status, expected_status);
                black_box(program);
            }
        }));

        if module_count == 16 && !policy_bypasses {
            benchmarks.push(bench("compiler.module_graph_medium_incremental", 200, {
                let fixture = FileFixture::new_graph("graph-medium-incremental", module_count);
                let (_, status) = compile_file_cached_verified_with_status(fixture.main())
                    .expect("prime incremental benchmark graph cache");
                assert_eq!(status, CompileCacheStatus::Miss);
                let changed_module = fixture.inputs()[1].clone();
                let mut replacement = 1;
                move || {
                    fs::write(&changed_module, format!("export fn value(input) {{ return input + {replacement} }}\n"))
                        .expect("change incremental benchmark module");
                    replacement ^= 1;
                    let (program, status) = compile_file_cached_verified_with_status(fixture.main())
                        .expect("incrementally compile benchmark graph");
                    assert_eq!(status, CompileCacheStatus::Incremental);
                    black_box(program);
                }
            }));
        }
    }
    benchmarks
}

fn compile_cache_phase_benchmarks() -> Vec<Benchmark> {
    // Keep phase attribution on a class that remains cacheable on Windows;
    // medium graphs intentionally bypass there.
    let fixture = Arc::new(FileFixture::new_graph("cache-phases", 64));
    let (verified, status) =
        compile_file_cached_verified_with_status(fixture.main()).expect("prime cache phase fixture");
    if status == CompileCacheStatus::Bypassed {
        return Vec::new();
    }
    assert_eq!(status, CompileCacheStatus::Miss);

    let metadata_path = fixture.cache_file("json");
    let artifact_path = fixture.cache_file("tob");
    let metadata_bytes = fs::read(&metadata_path).expect("read cache phase metadata");
    let artifact_bytes = fs::read(&artifact_path).expect("read cache phase artifact");
    let input_bytes = fixture
        .inputs()
        .iter()
        .map(|path| fs::read(path).expect("read cache phase input"))
        .collect::<Vec<_>>();
    let raw_program = verified.program().clone();
    let expected_fingerprint = verified.fingerprint().to_string();

    vec![
        bench("compiler.cache_phase.metadata_read", 20_000, {
            let path = metadata_path.clone();
            move || {
                black_box(fs::read(&path).expect("read cache metadata"));
            }
        }),
        bench("compiler.cache_phase.metadata_decode", 20_000, {
            let bytes = metadata_bytes.clone();
            move || {
                black_box(serde_json::from_slice::<JsonValue>(&bytes).expect("decode cache metadata"));
            }
        }),
        bench("compiler.cache_phase.input_metadata_prefilter", 200, {
            let fixture = Arc::clone(&fixture);
            move || {
                for path in fixture.inputs() {
                    black_box(fs::metadata(path).expect("stat cache input"));
                }
            }
        }),
        bench("compiler.cache_phase.input_read", 10_000, {
            let fixture = Arc::clone(&fixture);
            move || {
                for path in fixture.inputs() {
                    black_box(fs::read(path).expect("read cache input"));
                }
            }
        }),
        bench("compiler.cache_phase.input_hashing", 20_000, {
            let bytes = input_bytes.clone();
            move || {
                for input in &bytes {
                    black_box(Blake2b512::digest(input));
                }
            }
        }),
        bench("compiler.cache_phase.canonicalization", 10_000, {
            let fixture = Arc::clone(&fixture);
            move || {
                for path in fixture.inputs() {
                    black_box(path.canonicalize().expect("canonicalize cache input"));
                }
            }
        }),
        bench("compiler.cache_phase.artifact_read", 10_000, {
            let path = artifact_path.clone();
            move || {
                black_box(fs::read(&path).expect("read cache artifact"));
            }
        }),
        bench("compiler.cache_phase.binary_decode_verify", 5_000, {
            let bytes = artifact_bytes.clone();
            move || {
                black_box(VerifiedProgram::from_binary_artifact(&bytes).expect("decode verified cache artifact"));
            }
        }),
        bench("compiler.cache_phase.verification", 10_000, {
            let program = raw_program.clone();
            move || {
                BytecodeVerifier::verify(&program).expect("verify cache program");
                black_box(());
            }
        }),
        bench("compiler.cache_phase.fingerprint_compare", 10_000, {
            let program = raw_program;
            let expected = expected_fingerprint;
            move || {
                black_box(program.fingerprint() == expected);
            }
        }),
    ]
}

fn jit_attribution_benchmarks() -> Vec<Benchmark> {
    let empty = make_fixture("let value = 0\n");
    let mut setup_program = JitProgram::compile_verified(&empty.verified).expect("compile JIT setup fixture");
    let dispatch = make_fixture(STRAIGHTLINE_SOURCE);
    let mut dispatch_program = JitProgram::compile_verified(&dispatch.verified).expect("compile JIT dispatch fixture");
    let calls = make_fixture(FUNCTION_SOURCE);
    let mut calls_program = JitProgram::compile_verified(&calls.verified).expect("compile JIT call fixture");
    let stack_reuse = make_fixture(STRAIGHTLINE_SOURCE);
    let mut stack_reuse_program =
        JitProgram::compile_verified(&stack_reuse.verified).expect("compile JIT stack-reuse fixture");
    run_compiled_jit(&mut stack_reuse_program, Vec::new());
    let promotion = make_fixture(LOOP_SOURCE);
    let cold_promotion = JitProgram::compile_verified_with_options(
        &promotion.verified,
        JitOptions::new().with_hot_back_edge_threshold(1),
    )
    .expect("compile JIT promotion fixture");

    vec![
        bench("jit.execution_context_setup", 20_000, move || run_compiled_jit(&mut setup_program, Vec::new())),
        bench("jit.operand_stack_allocate_32", 100_000, || {
            black_box(Vec::<RuntimeValue>::with_capacity(32));
        }),
        bench("jit.operand_stack_reuse_32", 20_000, move || run_compiled_jit(&mut stack_reuse_program, Vec::new())),
        bench("jit.chunk_dispatch", 10_000, move || run_compiled_jit(&mut dispatch_program, Vec::new())),
        bench("jit.calls", 600, move || run_compiled_jit(&mut calls_program, Vec::new())),
        bench("jit.cold_program_clone", 10_000, {
            let program = cold_promotion.clone();
            move || {
                black_box(program.clone());
            }
        }),
        bench("jit.back_edge_promotion", 1_000, move || {
            let mut program = cold_promotion.clone();
            run_compiled_jit(&mut program, Vec::new());
            let stats = program.stats();
            assert!(stats.hot_ranges > 0);
            black_box(stats);
        }),
    ]
}

fn build_benchmarks() -> Vec<Benchmark> {
    let straightline = make_fixture(STRAIGHTLINE_SOURCE);
    let loop_fixture = make_fixture(LOOP_SOURCE);
    let hot_loop = make_fixture(HOT_LOOP_SOURCE);
    let functions = make_fixture(FUNCTION_SOURCE);
    let interrupts = make_fixture(CONTROL_INTERRUPT_SOURCE);
    let heap = make_fixture(HEAP_SOURCE);
    let builtins = make_fixture(BUILTIN_HEAVY_SOURCE);
    let input = make_fixture(INPUT_SOURCE);
    let vec_fixture = make_fixture(VEC_SOURCE);
    let map_fixture = make_fixture(MAP_SOURCE);
    let heap_churn = make_fixture(HEAP_CHURN_SOURCE);

    let mut shared_memory = TinyMemory::new(1024);

    let mut benchmarks = vec![
        bench("allocator.ralloc_buffer_64", 100_000, || {
            black_box(RallocBuffer::try_new(64).expect("allocate 64-byte Ralloc buffer"));
        }),
        bench("allocator.ralloc_buffer_4096", 30_000, || {
            black_box(RallocBuffer::try_new(4096).expect("allocate 4-KiB Ralloc buffer"));
        }),
        bench("allocator.ralloc_zero_fill_4096", 20_000, || {
            let mut buffer = RallocBuffer::try_new(4096).expect("allocate 4-KiB Ralloc buffer");
            buffer.as_mut_slice().fill(0);
            black_box(buffer.as_slice());
        }),
        bench("allocator.ralloc_resize_64_to_4096", 20_000, || {
            let mut buffer = RallocBuffer::try_new(64).expect("allocate Ralloc buffer");
            buffer.try_resize(4096).expect("grow Ralloc buffer");
            black_box(buffer.as_slice());
        }),
        bench("allocator.ralloc_arena_capacity_boundary", 200, || {
            let bytes = VmAllocator::max_allocation_size().saturating_sub(64);
            black_box(RallocBuffer::try_new(bytes).expect("allocate near-arena-capacity buffer"));
        }),
        bench("allocator.ralloc_fragmented_arena_cycle", 500, || {
            let mut slots = (0..512)
                .map(|_| Some(RallocBuffer::try_new(64).expect("seed fragmented arena")))
                .collect::<Vec<_>>();
            for index in (0..slots.len()).step_by(2) {
                slots[index].take();
            }
            black_box(RallocBuffer::try_new(128).expect("probe fragmented arena"));
            black_box(slots);
        }),
        bench("allocator.ralloc_contention_4x32", 1_000, {
            let mut contention = None;
            move || {
                let contention = contention.get_or_insert_with(|| AllocatorContention::new(4, 32, 64));
                contention.run_round();
            }
        }),
        bench("memory.allocate_8", 100_000, || {
            black_box(TinyMemory::new(8));
        }),
        bench("memory.allocate_1024", 30_000, || {
            black_box(TinyMemory::new(1024));
        }),
        bench("memory.load_store_64", 15_000, move || {
            for slot in 0..64 {
                shared_memory
                    .store(slot, RuntimeValue::I64((slot * 3) as i64))
                    .expect("store slot");
                black_box(shared_memory.load(slot).expect("load slot"));
            }
        }),
        bench("memory.reset_1024", 30_000, {
            let mut memory = TinyMemory::new(1024);
            move || {
                memory.store(511, RuntimeValue::I64(7)).expect("store slot");
                memory.reset();
                black_box(&memory);
            }
        }),
        bench("memory.snapshot_1024", 30_000, {
            let mut memory = TinyMemory::new(1024);
            for slot in 0..1024 {
                memory.store(slot, RuntimeValue::I64(slot as i64)).expect("store slot");
            }
            move || {
                black_box(memory.snapshot());
            }
        }),
        bench("frontend.lex", 10_000, || {
            black_box(lex_source(FUNCTION_SOURCE).expect("lex source"));
        }),
        bench("compiler.emit_bytecode", 3_000, || {
            black_box(compile_source_unoptimized(FUNCTION_SOURCE).expect("compile raw"));
        }),
        bench("optimizer.straightline", 20_000, {
            let raw = straightline.raw.clone();
            move || {
                black_box(optimize_program(raw.clone()));
            }
        }),
        bench("optimizer.control_flow_passthrough", 20_000, {
            let raw = loop_fixture.raw.clone();
            move || {
                black_box(optimize_program(raw.clone()));
            }
        }),
        bench("verifier.loop_cfg", 30_000, {
            let program = loop_fixture.program.clone();
            move || {
                BytecodeVerifier::verify(&program).expect("verify loop");
                black_box(());
            }
        }),
        bench("verifier.function_cfg", 20_000, {
            let program = functions.program.clone();
            move || {
                BytecodeVerifier::verify(&program).expect("verify functions");
                black_box(());
            }
        }),
        bench("verifier.heap_structs", 20_000, {
            let program = heap.program.clone();
            move || {
                BytecodeVerifier::verify(&program).expect("verify heap");
                black_box(());
            }
        }),
        bench("compile.full_pipeline", 2_000, || {
            black_box(compile_source(FUNCTION_SOURCE).expect("compile full"));
        }),
        bench("compiler.file_modules_uncached", 1_000, {
            let fixture = FileFixture::new("uncached");
            move || {
                black_box(compile_file_verified(fixture.main()).expect("compile multi-file benchmark fixture"));
            }
        }),
        bench("compiler.file_modules_cache_bypass", 2_000, {
            let fixture = FileFixture::new("cached");
            let (_, status) =
                compile_file_cached_verified_with_status(fixture.main()).expect("probe file compiler cache policy");
            assert_eq!(status, CompileCacheStatus::Bypassed);
            move || {
                let (program, status) = compile_file_cached_verified_with_status(fixture.main())
                    .expect("compile bypassed multi-file benchmark fixture");
                assert_eq!(status, CompileCacheStatus::Bypassed);
                black_box(program);
            }
        }),
        bench("program.fingerprint", 50_000, {
            let program = functions.program.clone();
            move || {
                black_box(program.fingerprint());
            }
        }),
        bench("program.to_artifact", 5_000, {
            let program = functions.program.clone();
            move || {
                black_box(program.to_artifact());
            }
        }),
        bench("program.from_artifact", 2_000, {
            let artifact = functions.artifact.clone();
            move || {
                black_box(Program::from_artifact(artifact.clone()).expect("artifact"));
            }
        }),
        bench("program.to_binary_artifact", 10_000, {
            let program = functions.program.clone();
            move || {
                black_box(program.to_binary_artifact().expect("binary artifact"));
            }
        }),
        bench("program.from_binary_artifact", 5_000, {
            let bytes = functions.program.to_binary_artifact().expect("binary artifact");
            move || {
                black_box(Program::from_binary_artifact(&bytes).expect("binary artifact"));
            }
        }),
        bench("jit.codegen_straightline_cold", 5_000, {
            let program = straightline.verified.clone();
            move || {
                let mut cache = JitCache::new();
                compile_jit_verified(&program, &mut cache);
            }
        }),
        bench("jit.codegen_dispatch_cold", 1_000, {
            let program = functions.verified.clone();
            move || {
                let mut cache = JitCache::new();
                compile_jit_verified(&program, &mut cache);
            }
        }),
        bench("jit.codegen_heap_cold", 1_000, {
            let program = heap.verified.clone();
            move || {
                let mut cache = JitCache::new();
                compile_jit_verified(&program, &mut cache);
            }
        }),
        bench("jit.codegen_builtin_cold", 1_000, {
            let program = builtins.verified.clone();
            move || {
                let mut cache = JitCache::new();
                compile_jit_verified(&program, &mut cache);
            }
        }),
        bench("jit.cache_hit_dispatch", 100_000, {
            let program = functions.program.clone();
            let mut cache = JitCache::new();
            compile_jit(&program, &mut cache);
            move || {
                compile_jit(&program, &mut cache);
            }
        }),
        bench("jit.cache_hit_verified_dispatch", 100_000, {
            let verified = functions.verified.clone();
            let mut cache = JitCache::new();
            cache.compile_verified(&verified).expect("warm verified cache");
            move || {
                black_box(std::ptr::from_ref(cache.compile_verified(&verified).expect("verified cache hit")));
            }
        }),
        bench("jit.cache_hit_straightline", 100_000, {
            let program = straightline.program.clone();
            let mut cache = JitCache::new();
            compile_jit(&program, &mut cache);
            move || {
                compile_jit(&program, &mut cache);
            }
        }),
        bench("jit.cache_hit_heap", 100_000, {
            let program = heap.program.clone();
            let mut cache = JitCache::new();
            compile_jit(&program, &mut cache);
            move || {
                compile_jit(&program, &mut cache);
            }
        }),
        bench("runtime.vm_straightline", 10_000, {
            let program = straightline.verified.clone();
            move || run_verified_mode(&program, "vm", Vec::new())
        }),
        bench("runtime.vm_loop_control", 2_000, {
            let program = loop_fixture.verified.clone();
            move || run_verified_mode(&program, "vm", Vec::new())
        }),
        bench("runtime.vm_hot_loop_4096", 100, {
            let program = hot_loop.verified.clone();
            move || run_verified_mode(&program, "vm", Vec::new())
        }),
        bench("runtime.vm_function_calls", 600, {
            let program = functions.verified.clone();
            move || run_verified_mode(&program, "vm", Vec::new())
        }),
        bench("runtime.vm_control_interrupts", 2_000, {
            let program = interrupts.verified.clone();
            move || run_verified_mode(&program, "vm", Vec::new())
        }),
        bench("runtime.vm_heap_structs", 1_000, {
            let program = heap.verified.clone();
            move || run_verified_mode(&program, "vm", Vec::new())
        }),
        bench("runtime.vm_builtin_heavy", 2_000, {
            let program = builtins.verified.clone();
            move || run_verified_mode(&program, "vm", Vec::new())
        }),
        bench("runtime.vm_vec_push_pop", 500, {
            let program = vec_fixture.verified.clone();
            move || run_verified_mode(&program, "vm", Vec::new())
        }),
        bench("runtime.vm_map_set_get", 500, {
            let program = map_fixture.verified.clone();
            move || run_verified_mode(&program, "vm", Vec::new())
        }),
        bench("runtime.vm_heap_churn", 500, {
            let program = heap_churn.verified.clone();
            move || run_verified_mode(&program, "vm", Vec::new())
        }),
        bench("runtime.jit_straightline", 10_000, {
            let mut program =
                JitProgram::compile_verified(&straightline.verified).expect("benchmark program should compile");
            move || run_compiled_jit(&mut program, Vec::new())
        }),
        bench("runtime.jit_loop_control", 2_000, {
            let mut program =
                JitProgram::compile_verified(&loop_fixture.verified).expect("benchmark program should compile");
            move || run_compiled_jit(&mut program, Vec::new())
        }),
        bench("runtime.jit_hot_loop_4096_quickened", 100, {
            let mut program =
                JitProgram::compile_verified(&hot_loop.verified).expect("benchmark program should compile");
            move || run_compiled_jit(&mut program, Vec::new())
        }),
        bench("runtime.jit_hot_loop_4096_no_quickening", 100, {
            let options = JitOptions::new().with_hot_back_edge_threshold(0);
            let mut program = JitProgram::compile_verified_with_options(&hot_loop.verified, options)
                .expect("benchmark program should compile");
            move || run_compiled_jit(&mut program, Vec::new())
        }),
        bench("runtime.jit_function_calls", 600, {
            let mut program =
                JitProgram::compile_verified(&functions.verified).expect("benchmark program should compile");
            move || run_compiled_jit(&mut program, Vec::new())
        }),
        bench("runtime.jit_control_interrupts", 2_000, {
            let mut program =
                JitProgram::compile_verified(&interrupts.verified).expect("benchmark program should compile");
            move || run_compiled_jit(&mut program, Vec::new())
        }),
        bench("runtime.jit_heap_structs", 1_000, {
            let mut program = JitProgram::compile_verified(&heap.verified).expect("benchmark program should compile");
            move || run_compiled_jit(&mut program, Vec::new())
        }),
        bench("runtime.jit_builtin_heavy", 2_000, {
            let mut program =
                JitProgram::compile_verified(&builtins.verified).expect("benchmark program should compile");
            move || run_compiled_jit(&mut program, Vec::new())
        }),
        bench("runtime.jit_vec_push_pop", 500, {
            let mut program =
                JitProgram::compile_verified(&vec_fixture.verified).expect("benchmark program should compile");
            move || run_compiled_jit(&mut program, Vec::new())
        }),
        bench("runtime.jit_map_set_get", 500, {
            let mut program =
                JitProgram::compile_verified(&map_fixture.verified).expect("benchmark program should compile");
            move || run_compiled_jit(&mut program, Vec::new())
        }),
        bench("runtime.jit_heap_churn", 500, {
            let mut program =
                JitProgram::compile_verified(&heap_churn.verified).expect("benchmark program should compile");
            move || run_compiled_jit(&mut program, Vec::new())
        }),
        bench("api.run_source_vm", 500, || {
            run_source_mode(LOOP_SOURCE, "vm", Vec::new());
        }),
        bench("api.run_source_jit_cold", 500, || {
            run_source_jit_cold(LOOP_SOURCE, Vec::new());
        }),
        bench("api.run_source_jit_warm", 500, {
            let mut cache = JitCache::new();
            run_source_jit_warm(LOOP_SOURCE, &mut cache, Vec::new());
            move || {
                run_source_jit_warm(LOOP_SOURCE, &mut cache, Vec::new());
            }
        }),
        bench("api.run_source_input_heap", 500, {
            let _program = input.program;
            move || {
                run_source_mode(INPUT_SOURCE, "jit", vec!["41".to_string()]);
            }
        }),
    ];
    benchmarks.extend(collection_and_heap_benchmarks());
    benchmarks.extend(runtime_pair("slot_compare_immediate_4096", SLOT_COMPARE_SOURCE.to_string(), 100));
    benchmarks.extend(runtime_pair("slot_mul_immediate_4096", SLOT_MUL_SOURCE.to_string(), 100));
    benchmarks.extend(runtime_pair("slot_div_immediate_4096", SLOT_DIV_SOURCE.to_string(), 100));
    benchmarks.extend(module_graph_benchmarks());
    benchmarks.extend(compile_cache_phase_benchmarks());
    benchmarks.extend(jit_attribution_benchmarks());
    benchmarks
}

fn run_benchmark(
    benchmark: &mut Benchmark,
    repeats: usize,
    quick: bool,
    sample_scale: u64,
) -> std::result::Result<BenchmarkResult, TinyOneError> {
    let base_iterations = if quick {
        (benchmark.iterations / 20).max(1)
    } else {
        benchmark.iterations
    };
    let iterations = base_iterations
        .checked_mul(sample_scale)
        .ok_or_else(|| TinyOneError::Runtime("Benchmark iteration count overflow".to_string()))?;

    for _ in 0..3 {
        (benchmark.run)();
    }

    let mut samples = Vec::with_capacity(repeats);
    let mut cycle_samples = Vec::with_capacity(repeats);
    let mut cpu_samples = Vec::with_capacity(repeats);
    for _ in 0..repeats {
        let cycle_start = thread_cycle_count();
        let cpu_start = thread_cpu_time_ns();
        let start = Instant::now();
        for _ in 0..iterations {
            (benchmark.run)();
        }
        let elapsed_ns = start.elapsed().as_nanos() as u64;
        samples.push(elapsed_ns as f64);
        if let (Some(start), Some(end)) = (cycle_start, thread_cycle_count())
            && end > start
        {
            cycle_samples.push((end - start) as f64);
        }
        if let (Some(start), Some(end)) = (cpu_start, thread_cpu_time_ns())
            && end > start
            && end - start <= elapsed_ns.saturating_add(50_000)
        {
            cpu_samples.push((end - start) as f64);
        }
    }

    let best_ns = samples.iter().copied().fold(f64::INFINITY, f64::min);
    let mean_ns = samples.iter().sum::<f64>() / samples.len() as f64;
    let stdev_ns = if samples.len() > 1 {
        let variance = samples
            .iter()
            .map(|sample| {
                let delta = sample - mean_ns;
                delta * delta
            })
            .sum::<f64>()
            / (samples.len() - 1) as f64;
        variance.sqrt()
    } else {
        0.0
    };
    let best_cycles = cycle_samples.iter().copied().reduce(f64::min);
    let mean_cycles =
        (!cycle_samples.is_empty()).then(|| cycle_samples.iter().sum::<f64>() / cycle_samples.len() as f64);
    let best_cpu_ns = cpu_samples.iter().copied().reduce(f64::min);
    let mean_cpu_ns = (!cpu_samples.is_empty()).then(|| cpu_samples.iter().sum::<f64>() / cpu_samples.len() as f64);

    Ok(BenchmarkResult {
        name: benchmark.name,
        iterations,
        best_ns,
        mean_ns,
        stdev_ns,
        best_cycles,
        mean_cycles,
        best_cpu_ns,
        mean_cpu_ns,
    })
}

fn format_duration(ns: f64) -> String {
    if ns < 1_000.0 {
        format!("{ns:.1} ns")
    } else if ns < 1_000_000.0 {
        format!("{:.2} us", ns / 1_000.0)
    } else if ns < 1_000_000_000.0 {
        format!("{:.2} ms", ns / 1_000_000.0)
    } else {
        format!("{:.2} s", ns / 1_000_000_000.0)
    }
}

fn title_bool(value: bool) -> &'static str {
    if value { "True" } else { "False" }
}

fn format_cycles(cycles: Option<f64>) -> String {
    let Some(cycles) = cycles else {
        return "-".to_string();
    };
    if cycles < 1_000.0 {
        format!("{cycles:.0}")
    } else if cycles < 1_000_000.0 {
        format!("{:.2} K", cycles / 1_000.0)
    } else if cycles < 1_000_000_000.0 {
        format!("{:.2} M", cycles / 1_000_000.0)
    } else {
        format!("{:.2} B", cycles / 1_000_000_000.0)
    }
}

fn print_table(results: &[BenchmarkResult]) {
    println!(
        "\n{:<44} {:>7} {:>12} {:>12} {:>12} {:>13} {:>6}",
        "benchmark", "iters", "best/iter", "mean/iter", "CPU time", "cycles", "cv%"
    );
    println!("{}", "-".repeat(113));
    for result in results {
        let cv_limit = cv_limit_for(result.name);
        let flag = if result.cv_pct() > cv_limit { " !" } else { "  " };
        println!(
            "{:<44} {:>7} {:>12} {:>12} {:>12} {:>13} {:>5.1}%{}",
            result.name,
            result.iterations,
            format_duration(result.best_per_iter_ns()),
            format_duration(result.mean_per_iter_ns()),
            result
                .best_per_iter_cpu_ns()
                .map_or_else(|| "-".to_string(), format_duration),
            format_cycles(result.best_per_iter_cycles()),
            result.cv_pct(),
            flag
        );
    }
    if results.iter().any(|result| result.cv_pct() > cv_limit_for(result.name)) {
        println!(
            "\n  ! = cv above the decision limit (5% for hot loops, 10% otherwise); \
             try --repeats or close background processes"
        );
    }
}

fn load_baseline(path: &str) -> Result<JsonValue, TinyOneError> {
    let text =
        fs::read_to_string(path).map_err(|error| TinyOneError::Runtime(format!("Baseline read error: {error}")))?;
    serde_json::from_str(&text).map_err(|error| TinyOneError::Runtime(format!("Baseline JSON error: {error}")))
}

fn baseline_items(document: &JsonValue) -> Result<&Vec<JsonValue>, TinyOneError> {
    document
        .as_array()
        .or_else(|| document.get("benchmarks").and_then(JsonValue::as_array))
        .ok_or_else(|| {
            TinyOneError::Runtime("Baseline must be a legacy JSON list or an object with a benchmarks list".to_string())
        })
}

fn assess_saved_evidence(document: &JsonValue, items: &[JsonValue]) -> EvidenceQuality {
    let mut rejections = Vec::new();
    let Some(metadata) = document.get("metadata") else {
        return EvidenceQuality::rejected(vec!["legacy baseline has no measurement metadata".to_string()]);
    };
    let Some(options) = metadata.get("benchmark_options") else {
        return EvidenceQuality::rejected(vec!["baseline has no benchmark measurement options".to_string()]);
    };
    if options.get("correctness_checked").and_then(JsonValue::as_bool) != Some(true) {
        rejections.push("baseline skipped pre-timing correctness checks".to_string());
    }
    let repeats = options.get("repeats").and_then(JsonValue::as_u64);
    if repeats.is_none_or(|repeats| repeats < MIN_DECISION_REPEATS as u64) {
        rejections.push(format!("baseline repeats are below the decision minimum of {MIN_DECISION_REPEATS}"));
    }
    if options.get("quick").and_then(JsonValue::as_bool) == Some(true) {
        rejections.push("baseline used --quick".to_string());
    }
    if metadata
        .get("evidence_quality")
        .and_then(|quality| quality.get("decision_eligible"))
        .and_then(JsonValue::as_bool)
        == Some(false)
    {
        rejections.push("baseline is marked non-decision-grade by its harness".to_string());
    }
    for item in items {
        let name = item
            .get("name")
            .and_then(JsonValue::as_str)
            .unwrap_or("<unnamed benchmark>");
        let Some(cv_pct) = item.get("cv_pct").and_then(JsonValue::as_f64) else {
            rejections.push(format!("{name} has no recorded coefficient of variation"));
            continue;
        };
        let limit = cv_limit_for(name);
        if cv_pct > limit {
            rejections.push(format!("baseline {name} has {cv_pct:.2}% CV, above its {limit:.0}% limit"));
        }
    }
    if rejections.is_empty() {
        EvidenceQuality::accepted()
    } else {
        EvidenceQuality::rejected(rejections)
    }
}

fn require_decision_evidence(label: &str, evidence_quality: &EvidenceQuality) -> Result<(), TinyOneError> {
    if evidence_quality.decision_eligible {
        return Ok(());
    }
    Err(TinyOneError::Runtime(format!(
        "{label} is not decision-grade: {}",
        evidence_quality.rejections.join("; ")
    )))
}

fn compare_to_baseline(results: &[BenchmarkResult], items: &[JsonValue], path: &str) {
    println!("\nBaseline comparison ({})", Path::new(path).display());
    println!(
        "{:<44} {:>12} {:>12} {:>9} {:>10} {:>10}",
        "benchmark", "baseline", "current", "wall", "CPU time", "cycles"
    );
    println!("{}", "-".repeat(108));

    for result in results {
        let item = items
            .iter()
            .find(|item| item.get("name").and_then(JsonValue::as_str) == Some(result.name));
        let Some(item) = item else {
            println!("{:<44} {:>12}", result.name, "(new)");
            continue;
        };
        let Some(old) = item.get("best_per_iter_ns").and_then(JsonValue::as_f64) else {
            println!("{:<44} {:>12}", result.name, "(invalid)");
            continue;
        };
        let new = result.best_per_iter_ns();
        let delta = ((new - old) / old) * 100.0;
        let metric_delta = |key: &str, new: Option<f64>| {
            item.get(key)
                .and_then(JsonValue::as_f64)
                .zip(new)
                .map_or_else(|| "-".to_string(), |(old, new)| format!("{:+.1}%", ((new - old) / old) * 100.0))
        };
        let cpu_delta = metric_delta("best_cpu_time_per_iter_ns", result.best_per_iter_cpu_ns());
        let cycle_delta = metric_delta("best_cycles_per_iter", result.best_per_iter_cycles());
        println!(
            "{:<44} {:>12} {:>12} {:+8.1}% {:>10} {:>10}",
            result.name,
            format_duration(old),
            format_duration(new),
            delta,
            cpu_delta,
            cycle_delta
        );
    }
}

fn metric_improvement_pct(item: &JsonValue, key: &str, current: Option<f64>) -> Option<f64> {
    item.get(key)
        .and_then(JsonValue::as_f64)
        .filter(|old| *old > 0.0)
        .zip(current)
        .map(|(old, new)| ((old - new) / old) * 100.0)
}

fn metric_regression_pct(item: &JsonValue, key: &str, current: Option<f64>) -> Option<f64> {
    item.get(key)
        .and_then(JsonValue::as_f64)
        .filter(|old| *old > 0.0)
        .zip(current)
        .map(|(old, new)| ((new - old) / old) * 100.0)
}

fn verify_priority_3_gate(results: &[BenchmarkResult], baseline: &[JsonValue]) -> Result<(), TinyOneError> {
    let mut failures = Vec::new();
    for (name, label) in PRIORITY_3_ROWS {
        let Some(current) = results.iter().find(|result| result.name == name) else {
            failures.push(format!("missing current {label} row ({name})"));
            continue;
        };
        let Some(previous) = baseline
            .iter()
            .find(|item| item.get("name").and_then(JsonValue::as_str) == Some(name))
        else {
            failures.push(format!("baseline is missing {label} row ({name})"));
            continue;
        };
        let cpu_improvement =
            metric_improvement_pct(previous, "best_cpu_time_per_iter_ns", current.best_per_iter_cpu_ns());
        let cycle_improvement =
            metric_improvement_pct(previous, "best_cycles_per_iter", current.best_per_iter_cycles());
        if [cpu_improvement, cycle_improvement]
            .into_iter()
            .flatten()
            .any(|improvement| improvement >= PRIORITY_3_MIN_IMPROVEMENT_PCT)
        {
            continue;
        }
        let metric = |name: &str, improvement: Option<f64>| {
            match improvement {
                Some(improvement) => format!("{name} {improvement:.2}%"),
                None => format!("{name} unavailable"),
            }
        };
        failures.push(format!(
            "{label} needs >= {PRIORITY_3_MIN_IMPROVEMENT_PCT:.0}% CPU-time or cycle reduction ({}; {})",
            metric("CPU", cpu_improvement),
            metric("cycles", cycle_improvement),
        ));
    }
    if failures.is_empty() {
        println!(
            "\nPriority 3 gate passed: all vector, map, and heap-churn rows improved by at least {PRIORITY_3_MIN_IMPROVEMENT_PCT:.0}% in CPU time or cycles."
        );
        Ok(())
    } else {
        Err(TinyOneError::Runtime(format!("Priority 3 gate failed: {}", failures.join("; "))))
    }
}

fn verify_priority_5_gate(results: &[BenchmarkResult], baseline: &[JsonValue]) -> Result<(), TinyOneError> {
    let mut failures = Vec::new();
    for (name, label) in PRIORITY_5_GUARDRAIL_ROWS {
        let Some(current) = results.iter().find(|result| result.name == name) else {
            failures.push(format!("missing current {label} row ({name})"));
            continue;
        };
        let Some(previous) = baseline
            .iter()
            .find(|item| item.get("name").and_then(JsonValue::as_str) == Some(name))
        else {
            failures.push(format!("baseline is missing {label} row ({name})"));
            continue;
        };

        let primary_metrics = [
            ("CPU", metric_regression_pct(previous, "mean_cpu_time_per_iter_ns", current.mean_per_iter_cpu_ns())),
            ("cycles", metric_regression_pct(previous, "mean_cycles_per_iter", current.mean_per_iter_cycles())),
        ];
        let mut checked_primary_metric = false;
        for (metric, regression) in primary_metrics {
            let Some(regression) = regression else {
                continue;
            };
            checked_primary_metric = true;
            if regression > PRIORITY_5_MAX_REGRESSION_PCT {
                failures.push(format!(
                    "{label} regressed {metric} by {regression:.2}% (limit {PRIORITY_5_MAX_REGRESSION_PCT:.0}%)",
                ));
            }
        }
        if !checked_primary_metric {
            let wall_regression = metric_regression_pct(previous, "mean_per_iter_ns", Some(current.mean_per_iter_ns()));
            let Some(wall_regression) = wall_regression else {
                failures.push(format!("{label} has no comparable wall-time metric"));
                continue;
            };
            if wall_regression > PRIORITY_5_MAX_REGRESSION_PCT {
                failures.push(format!(
                    "{label} regressed wall by {wall_regression:.2}% (limit {PRIORITY_5_MAX_REGRESSION_PCT:.0}%)",
                ));
            }
        }
    }
    if failures.is_empty() {
        println!(
            "\nPriority 5 gate passed: all allocator/memory guardrail rows stayed within {PRIORITY_5_MAX_REGRESSION_PCT:.0}% in every available CPU/cycle metric (or wall time when no primary metric is available)."
        );
        Ok(())
    } else {
        Err(TinyOneError::Runtime(format!("Priority 5 gate failed: {}", failures.join("; "))))
    }
}

fn require_priority_5_baseline_compatibility(args: &Args, baseline_document: &JsonValue) -> Result<(), TinyOneError> {
    let Some(metadata) = baseline_document.get("metadata") else {
        return Err(TinyOneError::Runtime("Priority 5 baseline has no metadata".to_string()));
    };
    let Some(options) = metadata.get("benchmark_options") else {
        return Err(TinyOneError::Runtime("Priority 5 baseline has no benchmark options".to_string()));
    };
    let mut mismatches = Vec::new();
    let expected_metadata = [
        ("package version", "package_version", json!(env!("CARGO_PKG_VERSION"))),
        ("operating system", "os", json!(env::consts::OS)),
        ("architecture", "architecture", json!(env::consts::ARCH)),
        ("filesystem context", "filesystem_context", json!(filesystem_context())),
        ("machine label", "machine_label", json!(args.machine_label.as_deref())),
        ("power policy", "power_policy", json!(args.power_policy.as_deref())),
    ];
    for (label, key, expected) in expected_metadata {
        if metadata.get(key) != Some(&expected) {
            mismatches.push(label.to_string());
        }
    }
    let expected_options = [
        ("repeat count", "repeats", json!(args.repeats)),
        ("sample scale", "sample_scale", json!(args.sample_scale)),
        ("quick mode", "quick", json!(args.quick)),
        ("Priority 5 selection", "priority_5_only", json!(true)),
        ("pre-timing correctness", "correctness_checked", json!(true)),
    ];
    for (label, key, expected) in expected_options {
        if options.get(key) != Some(&expected) {
            mismatches.push(label.to_string());
        }
    }
    if mismatches.is_empty() {
        Ok(())
    } else {
        Err(TinyOneError::Runtime(format!(
            "Priority 5 baseline metadata does not match the current capture: {}",
            mismatches.join(", ")
        )))
    }
}

fn save_baseline(document: &JsonValue, path: &Path) -> Result<(), TinyOneError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| TinyOneError::Runtime(format!("Baseline directory error: {error}")))?;
    }
    let text = serde_json::to_string_pretty(document)
        .map_err(|error| TinyOneError::Runtime(format!("Baseline JSON error: {error}")))?;
    fs::write(path, format!("{text}\n"))
        .map_err(|error| TinyOneError::Runtime(format!("Baseline write error: {error}")))?;
    println!("\nBaseline saved -> {}", path.display());
    Ok(())
}

fn run() -> Result<i32, TinyOneError> {
    let args = parse_args(env::args()).map_err(TinyOneError::Compile)?;

    if (args.priority_3_gate || args.priority_5_gate) && args.baseline.is_none() {
        return Err(TinyOneError::Runtime(
            "--priority-3-gate and --priority-5-gate require --baseline PATH".to_string(),
        ));
    }
    if args.priority_5_gate && !args.priority_5_only {
        return Err(TinyOneError::Runtime("--priority-5-gate requires --priority-5-only".to_string()));
    }

    if !args.skip_correctness || args.correctness_only {
        let cases = correctness_cases();
        println!("Correctness checks");
        println!("{}", "-".repeat(40));
        let failures = run_correctness_checks(&cases);
        if failures > 0 {
            println!("\n{failures} check(s) failed - aborting.");
            return Ok(1);
        }
        println!("\nAll {} checks passed.\n", cases.len());
    }

    if args.correctness_only {
        return Ok(0);
    }

    let mut benchmarks = build_benchmarks()
        .into_iter()
        .filter(|benchmark| args.filter.is_empty() || benchmark.name.contains(&args.filter))
        .filter(|benchmark| !args.priority_3_only || PRIORITY_3_ROWS.iter().any(|(name, _)| benchmark.name == *name))
        .filter(|benchmark| {
            !args.priority_5_only
                || PRIORITY_5_GUARDRAIL_ROWS
                    .iter()
                    .any(|(name, _)| benchmark.name == *name)
        })
        .collect::<Vec<_>>();

    if benchmarks.is_empty() {
        return Err(TinyOneError::Runtime(format!("No benchmarks matched {:?}", args.filter)));
    }

    println!("TinyOne VM/JIT benchmark suite");
    println!(
        "benchmarks={}  repeats={}  sample_scale={}  quick={}  thread_cpu_time={}  cycle_counter={}\n",
        benchmarks.len(),
        args.repeats,
        args.sample_scale,
        title_bool(args.quick),
        title_bool(thread_cpu_time_ns().is_some()),
        cycle_counter_kind()
    );

    let results = benchmarks
        .iter_mut()
        .map(|benchmark| run_benchmark(benchmark, args.repeats, args.quick, args.sample_scale))
        .collect::<std::result::Result<Vec<_>, TinyOneError>>()?;

    let correctness_checked = !args.skip_correctness;
    let evidence_quality = assess_current_evidence(&args, &results, correctness_checked);
    let metadata = run_metadata(&args, correctness_checked, &evidence_quality);
    let document = benchmark_document(&results, metadata);

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&document)
                .map_err(|error| TinyOneError::Runtime(format!("Benchmark JSON error: {error}")))?
        );
    } else {
        print_table(&results);
    }

    if let Some(path) = args.baseline.as_deref() {
        let baseline_document = load_baseline(path)?;
        let baseline = baseline_items(&baseline_document)?;
        let baseline_quality = assess_saved_evidence(&baseline_document, baseline);
        compare_to_baseline(&results, baseline, path);
        if !evidence_quality.decision_eligible || !baseline_quality.decision_eligible {
            println!("\nThis is a diagnostic comparison only; it cannot support an optimization claim.");
        }
        if args.priority_3_gate || args.priority_5_gate {
            require_decision_evidence("Current run", &evidence_quality)?;
            require_decision_evidence("Baseline", &baseline_quality)?;
        }
        if args.priority_3_gate {
            verify_priority_3_gate(&results, baseline)?;
        }
        if args.priority_5_gate {
            require_priority_5_baseline_compatibility(&args, &baseline_document)?;
            verify_priority_5_gate(&results, baseline)?;
        }
    }

    if let Some(path) = args.save_baseline.as_deref() {
        require_decision_evidence("Baseline save", &evidence_quality)?;
        save_baseline(&document, Path::new(&path))?;
    }

    if args.save_baseline_auto {
        require_decision_evidence("Automatic baseline save", &evidence_quality)?;
        let path = automatic_baseline_path(document.get("metadata").expect("benchmark document metadata"));
        save_baseline(&document, &path)?;
    }

    Ok(0)
}

fn main() {
    match run() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("TinyOne benchmark error: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(name: &'static str, best_cpu_ns: f64, cv_pct: f64) -> BenchmarkResult {
        BenchmarkResult {
            name,
            iterations: 1,
            best_ns: 100.0,
            mean_ns: 100.0,
            stdev_ns: cv_pct,
            best_cycles: None,
            mean_cycles: None,
            best_cpu_ns: Some(best_cpu_ns),
            mean_cpu_ns: Some(best_cpu_ns),
        }
    }

    #[test]
    fn benchmark_surface_covers_optimization_targets() {
        let names = build_benchmarks()
            .into_iter()
            .map(|benchmark| benchmark.name)
            .collect::<Vec<_>>();

        let medium_cache_name = if cfg!(windows) {
            "compiler.module_graph_medium_cache_bypass"
        } else {
            "compiler.module_graph_medium_cache_hit"
        };
        for expected in [
            "allocator.ralloc_buffer_64",
            "allocator.ralloc_buffer_4096",
            "allocator.ralloc_zero_fill_4096",
            "allocator.ralloc_resize_64_to_4096",
            "allocator.ralloc_arena_capacity_boundary",
            "allocator.ralloc_fragmented_arena_cycle",
            "allocator.ralloc_contention_4x32",
            "memory.reset_1024",
            "memory.snapshot_1024",
            "compiler.file_modules_uncached",
            "compiler.file_modules_cache_bypass",
            "compiler.module_graph_small_uncached",
            "compiler.module_graph_small_cache_bypass",
            medium_cache_name,
            "compiler.module_graph_large_cache_hit",
            "compiler.cache_phase.metadata_decode",
            "compiler.cache_phase.input_metadata_prefilter",
            "compiler.cache_phase.input_hashing",
            "compiler.cache_phase.canonicalization",
            "compiler.cache_phase.binary_decode_verify",
            "compiler.cache_phase.fingerprint_compare",
            "compiler.cache_phase.verification",
            "jit.codegen_straightline_cold",
            "jit.codegen_dispatch_cold",
            "jit.codegen_heap_cold",
            "jit.codegen_builtin_cold",
            "jit.cache_hit_dispatch",
            "jit.cache_hit_verified_dispatch",
            "jit.cache_hit_straightline",
            "jit.cache_hit_heap",
            "jit.execution_context_setup",
            "jit.operand_stack_allocate_32",
            "jit.operand_stack_reuse_32",
            "jit.chunk_dispatch",
            "jit.calls",
            "jit.back_edge_promotion",
            "runtime.jit_loop_control",
            "runtime.vm_hot_loop_4096",
            "runtime.jit_hot_loop_4096_quickened",
            "runtime.jit_hot_loop_4096_no_quickening",
            "runtime.vm_vec_push_pop",
            "runtime.jit_vec_push_pop",
            "runtime.vm_vec_push_pop_16",
            "runtime.jit_vec_push_pop_4096",
            "runtime.jit_vec_push_in_capacity_256",
            "runtime.jit_vec_capacity_growth_4096",
            "runtime.jit_vec_clear_256",
            "runtime.vm_map_set_get",
            "runtime.jit_map_set_get",
            "runtime.vm_map_set_get_16",
            "runtime.jit_map_set_get_4096",
            "runtime.jit_map_hit_256",
            "runtime.jit_map_miss_256",
            "runtime.jit_map_update_256",
            "runtime.jit_map_insert_in_capacity_256",
            "runtime.jit_map_delete_256",
            "runtime.jit_map_pointer_key_validation_256",
            "runtime.vm_heap_churn",
            "runtime.jit_heap_churn",
            "runtime.jit_heap_allocation_256",
            "runtime.jit_heap_lookup_256",
            "runtime.jit_heap_load_256",
            "runtime.jit_heap_store_256",
            "runtime.jit_heap_free_256",
            "runtime.jit_heap_slot_reuse_256",
            "runtime.jit_slot_compare_immediate_4096",
            "runtime.jit_slot_mul_immediate_4096",
            "runtime.jit_slot_div_immediate_4096",
            "api.run_source_jit_cold",
            "api.run_source_jit_warm",
        ] {
            assert!(names.contains(&expected), "missing benchmark {expected}");
        }
        if !cfg!(windows) {
            assert!(
                names.contains(&"compiler.module_graph_medium_incremental"),
                "missing medium incremental benchmark"
            );
        }
    }

    #[test]
    fn correctness_surface_remains_the_expected_22_vm_jit_checks() {
        assert_eq!(correctness_cases().len(), 22);
    }

    #[test]
    fn metadata_document_uses_versioned_schema() {
        let result = BenchmarkResult {
            name:        "test.row",
            iterations:  1,
            best_ns:     1.0,
            mean_ns:     1.0,
            stdev_ns:    0.0,
            best_cycles: Some(2.0),
            mean_cycles: Some(2.0),
            best_cpu_ns: Some(1.0),
            mean_cpu_ns: Some(1.0),
        };
        let document = benchmark_document(&[result], json!({"git_commit": "abc"}));
        assert_eq!(
            document.get("schema_version").and_then(JsonValue::as_u64),
            Some(u64::from(BENCHMARK_SCHEMA_VERSION))
        );
        assert_eq!(document.get("benchmarks").and_then(JsonValue::as_array).map(Vec::len), Some(1));
    }

    #[test]
    fn automatic_baselines_stay_under_target_perf() {
        let path = automatic_baseline_path(&json!({
            "filesystem_context": "test-filesystem",
            "git_commit": "1234567890abcdef",
            "timestamp_unix_seconds": 42,
        }));
        let expected_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("perf");
        assert!(path.starts_with(expected_root));
        assert_eq!(path.file_name().and_then(|value| value.to_str()), Some("baseline-1234567890ab-42.json"));
    }

    #[test]
    fn metadata_cli_options_parse() {
        let args = parse_args(
            [
                "tinylang-bench",
                "--save-baseline-auto",
                "--sample-scale",
                "8",
                "--priority-3-only",
                "--priority-3-gate",
                "--priority-5-only",
                "--priority-5-gate",
                "--machine-label",
                "workstation",
                "--power-policy",
                "performance",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .expect("metadata options parse");
        assert!(args.save_baseline_auto);
        assert_eq!(args.sample_scale, 8);
        assert!(args.priority_3_only);
        assert!(args.priority_3_gate);
        assert!(args.priority_5_only);
        assert!(args.priority_5_gate);
        assert_eq!(args.machine_label.as_deref(), Some("workstation"));
        assert_eq!(args.power_policy.as_deref(), Some("performance"));
    }

    #[test]
    fn evidence_quality_rejects_skipped_checks_and_unstable_rows() {
        let args = Args {
            repeats: MIN_DECISION_REPEATS,
            skip_correctness: true,
            ..Args::default()
        };
        let quality = assess_current_evidence(&args, &[result("runtime.test", 100.0, 10.01)], false);
        assert!(!quality.decision_eligible);
        assert!(
            quality
                .rejections
                .iter()
                .any(|reason| reason.contains("correctness checks were skipped"))
        );
        assert!(quality.rejections.iter().any(|reason| reason.contains("10.01% CV")));
    }

    #[test]
    fn stored_evidence_requires_metadata_and_acceptable_variance() {
        let baseline = json!({
            "metadata": {
                "benchmark_options": {
                    "correctness_checked": false,
                    "repeats": 11,
                    "quick": false,
                }
            },
            "benchmarks": [{
                "name": "runtime.jit_heap_churn",
                "cv_pct": 12.46,
            }],
        });
        let quality = assess_saved_evidence(&baseline, baseline_items(&baseline).expect("benchmark rows"));
        assert!(!quality.decision_eligible);
        assert!(
            quality
                .rejections
                .iter()
                .any(|reason| reason.contains("skipped pre-timing correctness"))
        );
        assert!(quality.rejections.iter().any(|reason| reason.contains("12.46% CV")));
    }

    #[test]
    fn priority_3_gate_rejects_a_subthreshold_vector_result() {
        let current = [
            result("runtime.jit_vec_push_pop_256", 90.95, 1.0),
            result("runtime.jit_map_set_get_256", 89.0, 1.0),
            result("runtime.jit_heap_churn", 89.0, 1.0),
        ];
        let baseline = json!([
            {"name": "runtime.jit_vec_push_pop_256", "best_cpu_time_per_iter_ns": 100.0},
            {"name": "runtime.jit_map_set_get_256", "best_cpu_time_per_iter_ns": 100.0},
            {"name": "runtime.jit_heap_churn", "best_cpu_time_per_iter_ns": 100.0},
        ]);
        let error = verify_priority_3_gate(&current, baseline.as_array().expect("baseline rows"))
            .expect_err("9.05% must not pass a 10% gate");
        assert!(error.to_string().contains("CPU 9.05%"));
    }

    #[test]
    fn priority_5_gate_rejects_a_guardrail_regression_above_five_percent() {
        let mut current = [
            result("allocator.ralloc_buffer_64", 100.0, 1.0),
            result("allocator.ralloc_buffer_4096", 100.0, 1.0),
            result("allocator.ralloc_resize_64_to_4096", 100.0, 1.0),
            result("memory.reset_1024", 100.0, 1.0),
            result("memory.snapshot_1024", 100.0, 1.0),
        ];
        current[0].mean_cpu_ns = Some(105.01);
        let baseline = json!([
            {"name": "allocator.ralloc_buffer_64", "mean_per_iter_ns": 100.0, "mean_cpu_time_per_iter_ns": 100.0},
            {"name": "allocator.ralloc_buffer_4096", "mean_per_iter_ns": 100.0, "mean_cpu_time_per_iter_ns": 100.0},
            {"name": "allocator.ralloc_resize_64_to_4096", "mean_per_iter_ns": 100.0, "mean_cpu_time_per_iter_ns": 100.0},
            {"name": "memory.reset_1024", "mean_per_iter_ns": 100.0, "mean_cpu_time_per_iter_ns": 100.0},
            {"name": "memory.snapshot_1024", "mean_per_iter_ns": 100.0, "mean_cpu_time_per_iter_ns": 100.0},
        ]);
        let error = verify_priority_5_gate(&current, baseline.as_array().expect("baseline rows"))
            .expect_err("a 5.01% CPU regression must fail the guardrail gate");
        assert!(
            error
                .to_string()
                .contains("64-byte Ralloc allocation regressed CPU by 5.01%")
        );
    }

    #[test]
    fn priority_5_gate_requires_matching_capture_metadata() {
        let args = Args {
            repeats: 7,
            sample_scale: 4,
            priority_5_only: true,
            machine_label: Some("workstation".to_string()),
            power_policy: Some("performance".to_string()),
            ..Args::default()
        };
        let mut baseline = json!({
            "metadata": {
                "package_version": env!("CARGO_PKG_VERSION"),
                "os": env::consts::OS,
                "architecture": env::consts::ARCH,
                "filesystem_context": filesystem_context(),
                "machine_label": "workstation",
                "power_policy": "performance",
                "benchmark_options": {
                    "repeats": 7,
                    "sample_scale": 4,
                    "quick": false,
                    "priority_5_only": true,
                    "correctness_checked": true,
                },
            },
        });
        require_priority_5_baseline_compatibility(&args, &baseline).expect("matching guardrail metadata");
        baseline["metadata"]["benchmark_options"]["sample_scale"] = json!(8);
        let error = require_priority_5_baseline_compatibility(&args, &baseline)
            .expect_err("mismatched sample scale must reject the baseline");
        assert!(error.to_string().contains("sample scale"));
    }

    #[test]
    fn default_loop_workload_reaches_the_quickened_tier() {
        let fixture = make_fixture(HOT_LOOP_SOURCE);
        let mut program = JitProgram::compile_verified(&fixture.verified).expect("benchmark program should compile");
        run_compiled_jit(&mut program, Vec::new());

        let stats = program.stats();
        assert!(stats.hot_ranges > 0, "loop benchmark never quickened");
        assert!(stats.quickened_ops > 0, "loop benchmark quickened no ops");
    }

    #[cfg(any(windows, all(target_os = "linux", any(target_arch = "x86", target_arch = "x86_64"))))]
    #[test]
    fn thread_cycle_counter_advances_during_cpu_work() {
        let start = thread_cycle_count().expect("thread cycle counter");
        let mut value = 0u64;
        for item in 0..100_000u64 {
            value = black_box(value.wrapping_add(item.rotate_left(7)));
        }
        black_box(value);
        let end = thread_cycle_count().expect("thread cycle counter");

        assert!(end > start, "thread cycle counter did not advance");
    }

    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn thread_cpu_time_is_monotonic_during_cpu_work() {
        let start = thread_cpu_time_ns().expect("thread CPU clock");
        let mut value = 0u64;
        for item in 0..100_000u64 {
            value = black_box(value.wrapping_add(item.rotate_left(11)));
        }
        black_box(value);
        let end = thread_cpu_time_ns().expect("thread CPU clock");

        assert!(end >= start, "thread CPU clock moved backwards");
    }
}
