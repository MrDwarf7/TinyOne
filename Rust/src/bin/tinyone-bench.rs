use std::env;
use std::fs;
use std::hint::black_box;
use std::io::{self, Write};
use std::path::Path;
use std::time::Instant;

use serde_json::{Value as JsonValue, json};
use tinyone::{
    BytecodeVerifier, JitCache, JitProgram, Program, RuntimeValue, TinyMemory, TinyOneError,
    compile_source, compile_source_unoptimized, lex_source, optimize_program, run_program,
    run_source,
};

const STRAIGHTLINE_SOURCE: &str = r#"
let a = 1
let b = a + 2
let c = b * 3
let d = c - a
let e = d / 2
print e
print e >= 4
"#;

const LOOP_SOURCE: &str = r#"
let i = 0
let total = 0
while i < 128 {
  total = total + (i * 3)
  i = i + 1
}
print total
"#;

const FUNCTION_SOURCE: &str = r#"
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
"#;

const CONTROL_INTERRUPT_SOURCE: &str = r#"
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
"#;

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

const INPUT_SOURCE: &str = r#"
let value = read_int()
let ptr = alloc(value)
print store(ptr, load(ptr) + 1)
let ignored = unsafe free(ptr)
"#;

const BUILTIN_HEAVY_SOURCE: &str = r#"
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
"#;

#[derive(Clone)]
struct CorrectnessCase {
    name: &'static str,
    source: &'static str,
    expected: &'static str,
    inputs: Vec<String>,
    mode: &'static str,
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
        self.inputs = inputs.iter().map(|item| item.to_string()).collect();
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

#[derive(Clone)]
struct Fixture {
    raw: Program,
    program: Program,
    artifact: JsonValue,
}

fn make_fixture(source: &'static str) -> Fixture {
    let raw = compile_source_unoptimized(source).expect("fixture should compile");
    let program = optimize_program(raw.clone());
    BytecodeVerifier::verify(&program).expect("fixture should verify");
    let artifact = program.to_artifact();
    Fixture {
        raw,
        program,
        artifact,
    }
}

struct Benchmark {
    name: &'static str,
    iterations: u64,
    run: Box<dyn FnMut()>,
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
    name: &'static str,
    iterations: u64,
    best_ns: f64,
    mean_ns: f64,
    stdev_ns: f64,
}

impl BenchmarkResult {
    fn best_per_iter_ns(&self) -> f64 {
        self.best_ns / self.iterations as f64
    }

    fn mean_per_iter_ns(&self) -> f64 {
        self.mean_ns / self.iterations as f64
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
            "cv_pct": self.cv_pct(),
        })
    }
}

#[derive(Clone)]
struct Args {
    quick: bool,
    json: bool,
    filter: String,
    repeats: usize,
    skip_correctness: bool,
    correctness_only: bool,
    baseline: Option<String>,
    save_baseline: Option<String>,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            quick: false,
            json: false,
            filter: String::new(),
            repeats: 5,
            skip_correctness: false,
            correctness_only: false,
            baseline: None,
            save_baseline: None,
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
            "--baseline" => {
                args.baseline = Some(iter.next().ok_or("--baseline requires a path")?);
            }
            "--save-baseline" => {
                args.save_baseline = Some(iter.next().ok_or("--save-baseline requires a path")?);
            }
            _ => return Err(format!("unknown option {arg}")),
        }
    }
    Ok(args)
}

fn print_help() {
    println!(
        "usage: tinyone-bench [--quick] [--filter TEXT] [--repeats N] \\
         [--skip-correctness] [--correctness-only] [--json] \\
         [--baseline PATH] [--save-baseline PATH]"
    );
}

fn correctness_cases() -> Vec<CorrectnessCase> {
    vec![
        CorrectnessCase::new("straightline/jit", STRAIGHTLINE_SOURCE, "4\n1\n"),
        CorrectnessCase::new("straightline/vm", STRAIGHTLINE_SOURCE, "4\n1\n").mode("vm"),
        CorrectnessCase::new("loop/jit", LOOP_SOURCE, "24384\n"),
        CorrectnessCase::new("loop/vm", LOOP_SOURCE, "24384\n").mode("vm"),
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
                    println!(
                        "  FAIL  {}  expected {:?} got {:?}",
                        case.name, case.expected, actual
                    );
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

fn run_mode(program: &Program, mode: &str, inputs: Vec<String>) {
    let mut sink = Sink;
    black_box(run_program(program, mode, &mut sink, inputs).expect("benchmark program should run"));
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

fn compile_jit(program: &Program, cache: &mut JitCache) {
    let compiled = cache
        .compile(program)
        .expect("benchmark program should compile") as *const _;
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

fn build_benchmarks() -> Vec<Benchmark> {
    let straightline = make_fixture(STRAIGHTLINE_SOURCE);
    let loop_fixture = make_fixture(LOOP_SOURCE);
    let functions = make_fixture(FUNCTION_SOURCE);
    let interrupts = make_fixture(CONTROL_INTERRUPT_SOURCE);
    let heap = make_fixture(HEAP_SOURCE);
    let builtins = make_fixture(BUILTIN_HEAVY_SOURCE);
    let input = make_fixture(INPUT_SOURCE);

    let mut shared_memory = TinyMemory::new(1024);

    vec![
        bench("memory.allocate_8", 100_000, || {
            black_box(TinyMemory::new(8));
        }),
        bench("memory.allocate_1024", 30_000, || {
            black_box(TinyMemory::new(1024));
        }),
        bench("memory.load_store_64", 15_000, move || {
            for slot in 0..64 {
                shared_memory
                    .store(slot, RuntimeValue::Int((slot * 3) as i64))
                    .expect("store slot");
                black_box(shared_memory.load(slot).expect("load slot"));
            }
        }),
        bench("memory.reset_1024", 30_000, {
            let mut memory = TinyMemory::new(1024);
            move || {
                memory.store(511, RuntimeValue::Int(7)).expect("store slot");
                memory.reset();
                black_box(memory.snapshot());
            }
        }),
        bench("memory.snapshot_1024", 30_000, {
            let mut memory = TinyMemory::new(1024);
            for slot in 0..1024 {
                memory
                    .store(slot, RuntimeValue::Int(slot as i64))
                    .expect("store slot");
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
        bench("jit.codegen_straightline_cold", 5_000, {
            let program = straightline.program.clone();
            move || {
                let mut cache = JitCache::new();
                compile_jit(&program, &mut cache);
            }
        }),
        bench("jit.codegen_dispatch_cold", 1_000, {
            let program = functions.program.clone();
            move || {
                let mut cache = JitCache::new();
                compile_jit(&program, &mut cache);
            }
        }),
        bench("jit.codegen_heap_cold", 1_000, {
            let program = heap.program.clone();
            move || {
                let mut cache = JitCache::new();
                compile_jit(&program, &mut cache);
            }
        }),
        bench("jit.codegen_builtin_cold", 1_000, {
            let program = builtins.program.clone();
            move || {
                let mut cache = JitCache::new();
                compile_jit(&program, &mut cache);
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
            let program = straightline.program.clone();
            move || run_mode(&program, "vm", Vec::new())
        }),
        bench("runtime.vm_loop_control", 2_000, {
            let program = loop_fixture.program.clone();
            move || run_mode(&program, "vm", Vec::new())
        }),
        bench("runtime.vm_function_calls", 600, {
            let program = functions.program.clone();
            move || run_mode(&program, "vm", Vec::new())
        }),
        bench("runtime.vm_control_interrupts", 2_000, {
            let program = interrupts.program.clone();
            move || run_mode(&program, "vm", Vec::new())
        }),
        bench("runtime.vm_heap_structs", 1_000, {
            let program = heap.program.clone();
            move || run_mode(&program, "vm", Vec::new())
        }),
        bench("runtime.vm_builtin_heavy", 2_000, {
            let program = builtins.program.clone();
            move || run_mode(&program, "vm", Vec::new())
        }),
        bench("runtime.jit_straightline", 10_000, {
            let mut program = JitProgram::compile(&straightline.program)
                .expect("benchmark program should compile");
            move || run_compiled_jit(&mut program, Vec::new())
        }),
        bench("runtime.jit_loop_control", 2_000, {
            let mut program = JitProgram::compile(&loop_fixture.program)
                .expect("benchmark program should compile");
            move || run_compiled_jit(&mut program, Vec::new())
        }),
        bench("runtime.jit_function_calls", 600, {
            let mut program =
                JitProgram::compile(&functions.program).expect("benchmark program should compile");
            move || run_compiled_jit(&mut program, Vec::new())
        }),
        bench("runtime.jit_control_interrupts", 2_000, {
            let mut program =
                JitProgram::compile(&interrupts.program).expect("benchmark program should compile");
            move || run_compiled_jit(&mut program, Vec::new())
        }),
        bench("runtime.jit_heap_structs", 1_000, {
            let mut program =
                JitProgram::compile(&heap.program).expect("benchmark program should compile");
            move || run_compiled_jit(&mut program, Vec::new())
        }),
        bench("runtime.jit_builtin_heavy", 2_000, {
            let mut program =
                JitProgram::compile(&builtins.program).expect("benchmark program should compile");
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
    ]
}

fn run_benchmark(benchmark: &mut Benchmark, repeats: usize, quick: bool) -> BenchmarkResult {
    let iterations = if quick {
        (benchmark.iterations / 20).max(1)
    } else {
        benchmark.iterations
    };

    for _ in 0..3 {
        (benchmark.run)();
    }

    let mut samples = Vec::with_capacity(repeats);
    for _ in 0..repeats {
        let start = Instant::now();
        for _ in 0..iterations {
            (benchmark.run)();
        }
        samples.push(start.elapsed().as_nanos() as f64);
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

    BenchmarkResult {
        name: benchmark.name,
        iterations,
        best_ns,
        mean_ns,
        stdev_ns,
    }
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

fn print_table(results: &[BenchmarkResult]) {
    const CV_WARN: f64 = 10.0;
    println!(
        "\n{:<44} {:>7} {:>12} {:>12} {:>6}",
        "benchmark", "iters", "best/iter", "mean/iter", "cv%"
    );
    println!("{}", "-".repeat(86));
    for result in results {
        let flag = if result.cv_pct() > CV_WARN {
            " !"
        } else {
            "  "
        };
        println!(
            "{:<44} {:>7} {:>12} {:>12} {:>5.1}%{}",
            result.name,
            result.iterations,
            format_duration(result.best_per_iter_ns()),
            format_duration(result.mean_per_iter_ns()),
            result.cv_pct(),
            flag
        );
    }
    if results.iter().any(|result| result.cv_pct() > CV_WARN) {
        println!(
            "\n  ! = cv > {CV_WARN}% - high jitter; try --repeats or close background processes"
        );
    }
}

fn compare_to_baseline(results: &[BenchmarkResult], path: &str) -> Result<(), TinyOneError> {
    let text = fs::read_to_string(path)
        .map_err(|error| TinyOneError::Runtime(format!("Baseline read error: {error}")))?;
    let baseline: JsonValue = serde_json::from_str(&text)
        .map_err(|error| TinyOneError::Runtime(format!("Baseline JSON error: {error}")))?;
    let Some(items) = baseline.as_array() else {
        return Err(TinyOneError::Runtime(
            "Baseline must be a JSON list".to_string(),
        ));
    };

    println!("\nBaseline comparison ({})", Path::new(path).display());
    println!(
        "{:<44} {:>12} {:>12} {:>9}",
        "benchmark", "baseline", "current", "delta"
    );
    println!("{}", "-".repeat(82));

    for result in results {
        let old = items.iter().find_map(|item| {
            if item.get("name").and_then(JsonValue::as_str) == Some(result.name) {
                item.get("best_per_iter_ns").and_then(JsonValue::as_f64)
            } else {
                None
            }
        });
        let Some(old) = old else {
            println!("{:<44} {:>12}", result.name, "(new)");
            continue;
        };
        let new = result.best_per_iter_ns();
        let delta = ((new - old) / old) * 100.0;
        println!(
            "{:<44} {:>12} {:>12} {:+8.1}%",
            result.name,
            format_duration(old),
            format_duration(new),
            delta
        );
    }
    Ok(())
}

fn save_baseline(results: &[BenchmarkResult], path: &str) -> Result<(), TinyOneError> {
    let data = JsonValue::Array(results.iter().map(BenchmarkResult::to_json).collect());
    let text = serde_json::to_string_pretty(&data)
        .map_err(|error| TinyOneError::Runtime(format!("Baseline JSON error: {error}")))?;
    fs::write(path, format!("{text}\n"))
        .map_err(|error| TinyOneError::Runtime(format!("Baseline write error: {error}")))?;
    println!("\nBaseline saved -> {path}");
    Ok(())
}

fn run() -> Result<i32, TinyOneError> {
    let args = parse_args(env::args()).map_err(TinyOneError::Compile)?;

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
        .collect::<Vec<_>>();

    if benchmarks.is_empty() {
        return Err(TinyOneError::Runtime(format!(
            "No benchmarks matched {:?}",
            args.filter
        )));
    }

    println!("TinyOne VM/JIT benchmark suite");
    println!(
        "benchmarks={}  repeats={}  quick={}\n",
        benchmarks.len(),
        args.repeats,
        title_bool(args.quick)
    );

    let results = benchmarks
        .iter_mut()
        .map(|benchmark| run_benchmark(benchmark, args.repeats, args.quick))
        .collect::<Vec<_>>();

    if args.json {
        let data = JsonValue::Array(results.iter().map(BenchmarkResult::to_json).collect());
        println!(
            "{}",
            serde_json::to_string_pretty(&data)
                .map_err(|error| TinyOneError::Runtime(format!("Benchmark JSON error: {error}")))?
        );
    } else {
        print_table(&results);
    }

    if let Some(path) = args.baseline {
        compare_to_baseline(&results, &path)?;
    }

    if let Some(path) = args.save_baseline {
        save_baseline(&results, &path)?;
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

    #[test]
    fn benchmark_surface_includes_adaptive_jit_codegen_cache_and_api_rows() {
        let names = build_benchmarks()
            .into_iter()
            .map(|benchmark| benchmark.name)
            .collect::<Vec<_>>();

        for expected in [
            "jit.codegen_straightline_cold",
            "jit.codegen_dispatch_cold",
            "jit.codegen_heap_cold",
            "jit.codegen_builtin_cold",
            "jit.cache_hit_dispatch",
            "jit.cache_hit_straightline",
            "jit.cache_hit_heap",
            "api.run_source_jit_cold",
            "api.run_source_jit_warm",
        ] {
            assert!(names.contains(&expected), "missing benchmark {expected}");
        }
    }
}
