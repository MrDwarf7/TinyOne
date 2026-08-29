#![cfg(feature = "testing-hooks")]

use tinyone::testing::{
    TestRuntimeCostCounters,
    compile_source_fixture,
    reset_runtime_cost_counters,
    run_backend,
    runtime_cost_counters,
};

static PROFILE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn profile(source: &str, name: &str) -> TestRuntimeCostCounters {
    let program = compile_source_fixture(source, name).expect("profile source should compile");
    reset_runtime_cost_counters();
    run_backend(program, "jit", Vec::new()).expect("profile source should run");
    runtime_cost_counters()
}

#[test]
fn collection_run_exposes_lock_codec_and_allocator_costs() {
    let _guard = PROFILE_LOCK.lock().expect("profile lock");
    let counters = profile(
        r#"
        let values = vec_new()
        let i = 0
        while i < 64 {
          let ignored = push(values, i)
          i = i + 1
        }
        while len(values) > 0 {
          let ignored = pop(values)
        }
        "#,
        "runtime-cost-counter-smoke.to",
    );

    assert!(counters.heap_lock_acquisitions > 0);
    assert!(counters.value_encodes > 0);
    assert!(counters.value_decodes > 0);
    assert!(counters.ralloc_growth_events > 0);
    assert!(counters.ralloc_bytes_copied > 0);
}

#[test]
fn map_hits_decode_values_while_map_has_skips_them() {
    let _guard = PROFILE_LOCK.lock().expect("profile lock");
    let setup = r#"
        let values = map_new()
        let i = 0
        while i < 16 {
          let ignored = map_set(values, i, i * 3)
          i = i + 1
        }
    "#;
    let has = profile(
        &format!(
            r#"{setup}
            let j = 0
            while j < 64 {{
              let ignored = map_has(values, 7)
              j = j + 1
            }}
            "#
        ),
        "map-has-cost.to",
    );
    let get = profile(
        &format!(
            r#"{setup}
            let j = 0
            while j < 64 {{
              let ignored = map_get(values, 7)
              j = j + 1
            }}
            "#
        ),
        "map-get-cost.to",
    );

    assert_eq!(get.heap_lock_acquisitions, has.heap_lock_acquisitions);
    assert_eq!(get.value_decodes, has.value_decodes + 64);
}

#[test]
#[ignore = "diagnostic profile; run explicitly with --ignored --nocapture"]
fn report_collection_cost_profiles() {
    let _guard = PROFILE_LOCK.lock().expect("profile lock");
    let workloads = [
        (
            "vec_push_pop",
            r#"
            let values = vec_new()
            let i = 0
            while i < 256 {
              let ignored = push(values, i)
              i = i + 1
            }
            while len(values) > 0 {
              let ignored = pop(values)
            }
            "#,
        ),
        (
            "map_set_get",
            r#"
            let values = map_new()
            let i = 0
            while i < 256 {
              let ignored = map_set(values, i, i * 3)
              i = i + 1
            }
            let total = 0
            let j = 0
            let key = 0
            while j < 4096 {
              total = total + map_get(values, key)
              key = key + 1
              if key == 256 {
                key = 0
              }
              j = j + 1
            }
            "#,
        ),
        (
            "heap_load_store",
            r#"
            let cell = alloc(0)
            let total = 0
            let i = 0
            while i < 4096 {
              let ignored = store(cell, i)
              total = total + load(cell)
              i = i + 1
            }
            "#,
        ),
        (
            "map_pointer_validation",
            r#"
            let values = map_new()
            let pointers = vec_new()
            let i = 0
            while i < 64 {
              let pointer = alloc(i)
              let ignored1 = push(pointers, pointer)
              let ignored2 = map_set(values, pointer, i)
              i = i + 1
            }
            let key = 0
            let total = 0
            let j = 0
            while j < 256 {
              total = total + map_get(values, pointers[key])
              key = key + 1
              if key == 64 {
                key = 0
              }
              j = j + 1
            }
            "#,
        ),
    ];

    for (name, source) in workloads {
        let counters = profile(source, &format!("{name}.to"));
        eprintln!("{name}: {counters:?}");
    }
}
