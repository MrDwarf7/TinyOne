use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use tinyone::{Program, TinyOneError, VerifiedProgram, compile_file, run_program, write_binary_artifact};

struct TestDir(PathBuf);

impl TestDir {
    fn new(label: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("tinyone-module-sandbox-{label}-{}-{stamp}", std::process::id()));
        fs::create_dir_all(&path).expect("create temporary project");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run(program: Arc<Program>, mode: &str) -> Result<String, TinyOneError> {
    let mut stdout = Vec::new();
    run_program(program, mode, &mut stdout, Vec::new())?;
    Ok(String::from_utf8(stdout).expect("TinyOne output is UTF-8"))
}

#[cfg(unix)]
fn create_file_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn create_file_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(target, link)
}

#[cfg(not(any(unix, windows)))]
fn create_file_symlink(_target: &Path, _link: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "this platform does not support file symlinks in the test harness",
    ))
}

fn write_fs_module(project: &TestDir, manifest_module: &str) -> PathBuf {
    fs::write(project.path().join("tinyone.json"), format!(r#"{{"modules":{{"host":{manifest_module}}}}}"#))
        .expect("write manifest");
    fs::write(
        project.path().join("host.to"),
        r#"
        export fn probe() {
          return fs_exists(".")
        }
        "#,
    )
    .expect("write module");
    let main = project.path().join("main.to");
    fs::write(
        &main,
        r#"
        import "host" as host
        print host.probe()
        "#,
    )
    .expect("write entrypoint");
    main
}

#[test]
fn imported_modules_start_without_host_capabilities_in_both_backends() {
    let project = TestDir::new("deny-filesystem");
    let main = write_fs_module(&project, r#""host.to""#);
    let program = compile_file(main).expect("compile isolated module");

    for mode in ["vm", "jit"] {
        let error = run(program.clone(), mode).expect_err("filesystem access must be denied");
        assert!(
            error
                .to_string()
                .contains("builtin \"fs_exists\" requires \"filesystem\""),
            "{error}"
        );
    }
}

#[test]
fn manifest_grant_is_retained_by_vm_jit_and_artifacts() {
    let project = TestDir::new("allow-filesystem");
    let main = write_fs_module(&project, r#"{"path":"host.to","capabilities":["filesystem"]}"#);
    let program = compile_file(main).expect("compile module with declared grant");
    assert_eq!(program.modules()[0].capabilities(), ["filesystem"]);

    for mode in ["vm", "jit"] {
        assert_eq!(run(program.clone(), mode).expect("granted module runs"), "1\n");
    }

    let artifact = program.to_artifact();
    let untrusted_json = Program::from_artifact(artifact.clone()).expect("untrusted JSON artifact");
    assert_eq!(untrusted_json.modules()[0].capabilities(), ["filesystem"]);
    for mode in ["vm", "jit"] {
        let error = run(Arc::new(untrusted_json.clone()), mode)
            .expect_err("untrusted JSON policy must not grant filesystem access");
        assert!(
            error
                .to_string()
                .contains("builtin \"fs_exists\" requires \"filesystem\""),
            "{error}"
        );
    }

    let json_round_trip = VerifiedProgram::from_trusted_artifact(artifact)
        .expect("trusted JSON artifact")
        .into_program();
    assert_eq!(json_round_trip.modules()[0].capabilities(), ["filesystem"]);
    assert_eq!(run(Arc::new(json_round_trip), "jit").expect("JSON artifact runs"), "1\n");

    let binary_path = project.path().join("program.tob");
    write_binary_artifact(&program, &binary_path).expect("write binary artifact");
    let binary = fs::read(binary_path).expect("read binary artifact");
    let untrusted_binary = Program::from_binary_artifact(&binary).expect("untrusted binary artifact");
    assert_eq!(untrusted_binary.modules()[0].capabilities(), ["filesystem"]);
    for mode in ["vm", "jit"] {
        let error = run(Arc::new(untrusted_binary.clone()), mode)
            .expect_err("untrusted binary policy must not grant filesystem access");
        assert!(
            error
                .to_string()
                .contains("builtin \"fs_exists\" requires \"filesystem\""),
            "{error}"
        );
    }
    let binary_round_trip = VerifiedProgram::from_trusted_binary_artifact(&binary)
        .expect("trusted binary artifact")
        .into_program();
    assert_eq!(binary_round_trip.modules()[0].capabilities(), ["filesystem"]);
    assert_eq!(run(Arc::new(binary_round_trip), "vm").expect("binary artifact runs"), "1\n");
}

#[test]
fn detailed_module_permissions_limit_filesystem_writes_and_environment_reads() {
    let project = TestDir::new("fine-module-permissions");
    fs::write(
        project.path().join("tinyone.json"),
        r#"{"modules":{"files":{"path":"files.to","capabilities":["filesystem"]},"env":{"path":"env.to","capabilities":["environment"]}}}"#,
    )
    .expect("write manifest");
    fs::write(
        project.path().join("files.to"),
        r#"
        export fn write_probe() {
          let bytes = buffer(1)
          return unsafe fs_write("must-not-be-created", bytes)
        }
        "#,
    )
    .expect("write filesystem module");
    fs::write(
        project.path().join("env.to"),
        r#"
        export fn env_probe() {
          return sys_env_get("SECRET_TOKEN")
        }
        "#,
    )
    .expect("write environment module");
    let main = project.path().join("main.to");
    fs::write(
        &main,
        r#"
        import "files" as files
        import "env" as env
        print files.write_probe()
        print env.env_probe()
        "#,
    )
    .expect("write entrypoint");

    let program = compile_file(main).expect("compile broad compatibility grants");
    let mut artifact = program.to_artifact();
    let modules = artifact["modules"].as_array_mut().expect("artifact module list");
    let file_permissions = modules[0]["permissions"]
        .as_object_mut()
        .expect("filesystem permission object");
    file_permissions.insert("filesystem_read".to_string(), serde_json::Value::Bool(true));
    file_permissions.insert("filesystem_write".to_string(), serde_json::Value::Bool(false));
    let env_permissions = modules[1]["permissions"]
        .as_object_mut()
        .expect("environment permission object");
    env_permissions.insert("environment_read".to_string(), serde_json::json!(["PUBLIC_VALUE"]));

    let restricted = VerifiedProgram::from_trusted_artifact(artifact)
        .expect("trusted artifact with detailed policy")
        .into_program();
    assert_eq!(restricted.modules()[0].filesystem_permissions(), (true, false));
    assert_eq!(restricted.modules()[1].environment_read_allowlist(), Some(["PUBLIC_VALUE".to_string()].as_slice()));
    for mode in ["vm", "jit"] {
        let error =
            run(Arc::new(restricted.clone()), mode).expect_err("filesystem write must be denied before host access");
        assert!(
            error
                .to_string()
                .contains("builtin \"fs_write\" is outside the signed module declaration"),
            "{error}"
        );
    }

    let mut artifact = program.to_artifact();
    let modules = artifact["modules"].as_array_mut().expect("artifact module list");
    let file_permissions = modules[0]["permissions"]
        .as_object_mut()
        .expect("filesystem permission object");
    file_permissions.insert("filesystem_read".to_string(), serde_json::Value::Bool(true));
    file_permissions.insert("filesystem_write".to_string(), serde_json::Value::Bool(false));
    let env_permissions = modules[1]["permissions"]
        .as_object_mut()
        .expect("environment permission object");
    env_permissions.insert("environment_read".to_string(), serde_json::json!(["PUBLIC_VALUE"]));
    let restricted = VerifiedProgram::from_trusted_artifact(artifact)
        .expect("trusted artifact with detailed policy")
        .into_program();

    // Invoke only the environment module after removing the filesystem call,
    // so we assert the dynamic variable-name allowlist independently.
    let mut environment_only = restricted.to_artifact();
    environment_only["code"] = serde_json::json!([
        {"op": "CALL", "arg": 1, "arg2": 0},
        {"op": "PRINT", "arg": 0, "arg2": 0},
        {"op": "HALT", "arg": 0, "arg2": 0}
    ]);
    let environment_only = VerifiedProgram::from_trusted_artifact(environment_only)
        .expect("trusted environment-only artifact")
        .into_program();
    for mode in ["vm", "jit"] {
        let error =
            run(Arc::new(environment_only.clone()), mode).expect_err("undeclared environment variable must be denied");
        assert!(
            error
                .to_string()
                .contains("builtin \"sys_env_get\" may not read environment variable \"SECRET_TOKEN\""),
            "{error}"
        );
    }
}

#[test]
fn jit_direct_builtin_cannot_bypass_unsafe_memory_grant() {
    let project = TestDir::new("deny-unsafe-memory");
    fs::write(project.path().join("tinyone.json"), r#"{"modules":{"mem":"mem.to"}}"#).expect("write manifest");
    fs::write(
        project.path().join("mem.to"),
        r#"
        export fn release() {
          let data = buffer(1)
          return unsafe free(data)
        }
        "#,
    )
    .expect("write module");
    let main = project.path().join("main.to");
    fs::write(&main, "import \"mem\" as mem\nprint mem.release()\n").expect("write entrypoint");
    let program = compile_file(main).expect("compile module");

    for mode in ["vm", "jit"] {
        let error = run(program.clone(), mode).expect_err("unsafe memory must be denied");
        assert!(
            error
                .to_string()
                .contains("builtin \"free\" requires \"unsafe_memory\""),
            "{error}"
        );
    }
}

#[test]
fn ffi_declaration_does_not_grant_unsafe_memory_to_vm_or_direct_jit() {
    let project = TestDir::new("ffi-is-not-unsafe-memory");
    fs::write(
        project.path().join("tinyone.json"),
        r#"{"modules":{"mem":{"path":"mem.to","capabilities":["unsafe_memory"]}}}"#,
    )
    .expect("write manifest");
    fs::write(
        project.path().join("mem.to"),
        r#"
        export fn release() {
          let data = buffer(1)
          return unsafe free(data)
        }
        "#,
    )
    .expect("write module");
    let main = project.path().join("main.to");
    fs::write(&main, "import \"mem\" as mem\nprint mem.release()\n").expect("write entrypoint");
    let program = compile_file(main).expect("compile module");

    let mut artifact = program.to_artifact();
    let permissions = artifact["modules"][0]["permissions"]
        .as_object_mut()
        .expect("module permissions");
    permissions.insert("ffi_allowed".to_string(), serde_json::Value::Bool(true));
    permissions.insert("unsafe_memory_allowed".to_string(), serde_json::Value::Bool(false));
    let restricted = VerifiedProgram::from_trusted_artifact(artifact)
        .expect("trusted FFI-only policy")
        .into_program();

    for mode in ["vm", "jit"] {
        let error =
            run(Arc::new(restricted.clone()), mode).expect_err("FFI permission must not authorize unsafe memory");
        assert!(
            error
                .to_string()
                .contains("builtin \"free\" is outside the signed module declaration"),
            "{error}"
        );
    }
}

#[test]
fn string_selected_calls_cannot_turn_root_functions_into_module_deputies() {
    let project = TestDir::new("deny-root-deputy");
    fs::write(project.path().join("tinyone.json"), r#"{"modules":{"plugin":"plugin.to"}}"#).expect("write manifest");
    fs::write(
        project.path().join("plugin.to"),
        r#"
        export fn invoke_root() {
          let callback = closure_new("host_filesystem", [])
          return callback()
        }
        "#,
    )
    .expect("write module");
    let main = project.path().join("main.to");
    fs::write(
        &main,
        r#"
        import "plugin" as plugin
        fn host_filesystem() {
          return fs_exists(".")
        }
        print plugin.invoke_root()
        "#,
    )
    .expect("write entrypoint");
    let program = compile_file(main).expect("compile module");

    for mode in ["vm", "jit"] {
        let error = run(program.clone(), mode).expect_err("root function selection must fail");
        assert!(
            error
                .to_string()
                .contains("closure_new: function \"host_filesystem\" not found or not exported"),
            "{error}"
        );
    }
}

#[test]
fn imports_and_manifest_targets_cannot_escape_the_entry_directory() {
    let parent = TestDir::new("path-escape-parent");
    let project = parent.path().join("project");
    fs::create_dir(&project).expect("create project");
    fs::write(parent.path().join("outside.to"), "export fn value() { return 1 }").expect("write external module");
    let direct_escape = project.join("main.to");
    fs::write(&direct_escape, "import \"../outside.to\" as outside\nprint outside.value()\n")
        .expect("write entrypoint");
    let direct_error = compile_file(&direct_escape).expect_err("parent import must fail");
    assert!(direct_error.to_string().contains("escapes the module sandbox"));

    fs::write(project.join("tinyone.json"), r#"{"modules":{"outside":"../outside.to"}}"#)
        .expect("write escaping manifest");
    fs::write(&direct_escape, "import \"outside\" as outside\nprint outside.value()\n")
        .expect("write manifest entrypoint");
    let manifest_error = compile_file(direct_escape).expect_err("escaping manifest target must fail");
    assert!(manifest_error.to_string().contains("escaping path"));
}

#[test]
fn symlinked_imports_cannot_escape_the_entry_directory() {
    let parent = TestDir::new("symlink-escape-parent");
    let project = parent.path().join("project");
    fs::create_dir(&project).expect("create project");
    let outside = parent.path().join("outside.to");
    fs::write(&outside, "export fn value() { return 1 }").expect("write external module");
    let symlink = project.join("outside-link.to");

    // Creating symlinks requires an elevated Windows token unless Developer
    // Mode is enabled. The resolver's protection is covered whenever the
    // host permits symlink creation; unsupported test hosts remain valid.
    if let Err(error) = create_file_symlink(&outside, &symlink) {
        if matches!(error.kind(), std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::Unsupported) {
            eprintln!("skipping symlink containment test: {error}");
            return;
        }
        panic!("create symlink: {error}");
    }

    let main = project.join("main.to");
    fs::write(&main, "import \"outside-link.to\" as outside\nprint outside.value()\n").expect("write entrypoint");
    let error = compile_file(main).expect_err("symlink import must fail");
    assert!(error.to_string().contains("escapes module sandbox"), "{error}");
}

#[test]
fn config_can_explicitly_disable_the_import_sandbox_for_trusted_projects() {
    let parent = TestDir::new("sandbox-disabled");
    let project = parent.path().join("project");
    fs::create_dir(&project).expect("create project");
    fs::write(project.join("Config.toml"), "[sandbox]\nenabled = false\n\n[permissions]\ncodebase = []\n")
        .expect("write Config.toml");
    fs::write(parent.path().join("outside.to"), "export fn value() { return 9 }\n").expect("write external module");
    let main = project.join("main.to");
    fs::write(&main, "import \"../outside.to\" as outside\nprint outside.value()\n").expect("write entrypoint");

    let program = compile_file(main).expect("explicitly disabled sandbox permits import");
    assert_eq!(run(program, "vm").expect("external module runs"), "9\n");
}

#[test]
fn config_toml_declares_modules_and_caps_them_at_the_codebase_ceiling() {
    let project = TestDir::new("config-module-policy");
    fs::write(
        project.path().join("Config.toml"),
        r#"
        [permissions]
        codebase = ["filesystem"]

        [rules]
        require_configured_modules = true

        [modules.host]
        path = "host.to"
        permissions = ["filesystem"]
        "#,
    )
    .expect("write Config.toml");
    fs::write(project.path().join("host.to"), "export fn probe() { return fs_exists(\".\") }").expect("write module");
    let main = project.path().join("main.to");
    fs::write(&main, "import \"host\" as host\nprint host.probe()\n").expect("write entrypoint");

    let program = compile_file(main).expect("Config.toml module compiles without tinyone.json");
    assert_eq!(program.root_capabilities(), ["filesystem"]);
    assert_eq!(program.modules()[0].capabilities(), ["filesystem"]);
    for mode in ["vm", "jit"] {
        assert_eq!(run(program.clone(), mode).expect("granted module runs"), "1\n");
    }
}

#[test]
fn config_rules_can_require_every_module_to_be_declared_in_config_toml() {
    let project = TestDir::new("configured-module-rule");
    fs::write(project.path().join("Config.toml"), "[rules]\nrequire_configured_modules = true\n")
        .expect("write Config.toml");
    fs::write(project.path().join("tinyone.json"), r#"{"modules":{"legacy":"legacy.to"}}"#)
        .expect("write legacy manifest");
    fs::write(project.path().join("legacy.to"), "export fn value() { return 1 }\n").expect("write module");
    let main = project.path().join("main.to");
    fs::write(&main, "import \"legacy\" as legacy\nprint legacy.value()\n").expect("write entrypoint");

    let error = compile_file(main).expect_err("legacy-only mapping is rejected");
    assert!(error.to_string().contains("is not declared in Config.toml [modules]"), "{error}");
}

#[test]
fn config_toml_denies_root_code_and_preserves_the_policy_in_artifacts() {
    let project = TestDir::new("config-root-policy");
    fs::write(
        project.path().join("Config.toml"),
        r#"
        [permissions]
        codebase = []

        [vm]
        max_call_depth = 1
        "#,
    )
    .expect("write Config.toml");
    let main = project.path().join("main.to");
    fs::write(&main, "print fs_exists(\".\")\n").expect("write entrypoint");
    let program = compile_file(&main).expect("compile restricted root program");
    assert_eq!(program.root_capabilities(), [] as [std::string::String; 0]);
    assert_eq!(program.max_call_depth(), 1);

    let artifact = VerifiedProgram::from_trusted_artifact(program.to_artifact())
        .expect("trusted JSON artifact")
        .into_program();
    assert_eq!(artifact.root_capabilities(), [] as [std::string::String; 0]);
    assert_eq!(artifact.max_call_depth(), 1);
    for mode in ["vm", "jit"] {
        let error = run(Arc::new(artifact.clone()), mode).expect_err("root filesystem denied");
        assert!(
            error
                .to_string()
                .contains("builtin \"fs_exists\" requires \"filesystem\""),
            "{error}"
        );
    }

    let binary_path = project.path().join("restricted-root.tob");
    write_binary_artifact(&program, &binary_path).expect("write binary artifact");
    let binary = fs::read(binary_path).expect("read binary artifact");
    let binary_artifact = VerifiedProgram::from_trusted_binary_artifact(&binary)
        .expect("trusted binary artifact preserves root policy")
        .into_program();
    assert_eq!(binary_artifact.root_capabilities(), [] as [std::string::String; 0]);
    assert_eq!(binary_artifact.max_call_depth(), 1);
    for mode in ["vm", "jit"] {
        let error = run(Arc::new(binary_artifact.clone()), mode).expect_err("binary root filesystem denied");
        assert!(
            error
                .to_string()
                .contains("builtin \"fs_exists\" requires \"filesystem\""),
            "{error}"
        );
    }

    fs::write(
        &main,
        r#"
        fn one_call_is_allowed() {
          return 7
        }
        print one_call_is_allowed()
        "#,
    )
    .expect("write exactly-one-call entrypoint");
    let depth_boundary = compile_file(&main).expect("compile depth-boundary program");
    for mode in ["vm", "jit"] {
        assert_eq!(run(depth_boundary.clone(), mode).expect("one nested call is permitted"), "7\n");
    }

    fs::write(
        &main,
        r#"
        fn recurse(n) {
          if n == 0 { return 0 }
          return recurse(n - 1)
        }
        print recurse(2)
        "#,
    )
    .expect("write call-depth entrypoint");
    let depth_limited = compile_file(main).expect("compile call-depth program");
    for mode in ["vm", "jit"] {
        let error = run(depth_limited.clone(), mode).expect_err("call depth is enforced");
        assert!(error.to_string().contains("Call stack overflow after 1 nested call"), "{error}");
    }
}

#[test]
fn string_selected_threads_cannot_turn_root_functions_into_module_deputies() {
    let project = TestDir::new("deny-root-thread-deputy");
    fs::write(
        project.path().join("tinyone.json"),
        r#"{"modules":{"plugin":{"path":"plugin.to","capabilities":["threads"]}}}"#,
    )
    .expect("write manifest");
    fs::write(
        project.path().join("plugin.to"),
        r#"
        export fn invoke_root() {
          let thread = thread_spawn("host_filesystem")
          return thread_join(thread)
        }
        "#,
    )
    .expect("write module");
    let main = project.path().join("main.to");
    fs::write(
        &main,
        r#"
        import "plugin" as plugin
        fn host_filesystem() {
          return fs_exists(".")
        }
        print plugin.invoke_root()
        "#,
    )
    .expect("write entrypoint");
    let program = compile_file(main).expect("compile module");

    for mode in ["vm", "jit"] {
        let error = run(program.clone(), mode).expect_err("root function selection must fail");
        assert!(
            error
                .to_string()
                .contains("thread_spawn: function \"host_filesystem\" not found or not exported"),
            "{error}"
        );
    }
}

#[test]
fn signing_fails_closed_without_a_recognized_central_root() {
    let project = TestDir::new("signed-modules");
    fs::write(
        project.path().join("Config.toml"),
        r#"
            [permissions]
            codebase = []

            [modules.host]
            path = "host.to"
            permissions = []

            [signing]
            require_module_signatures = true

            [[signing.authorities]]
            id = "self-signed"
            issuer = "not-a-tinyone-root"
            public_key = "0000000000000000000000000000000000000000000000000000000000000000"
            not_before = 0
            expires = 4102444800
            certificate = "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
            "#,
    )
    .expect("write Config.toml");
    let source = "export fn value() { return 7 }\n";
    fs::write(project.path().join("host.to"), source).expect("write module");
    let main = project.path().join("main.to");
    fs::write(&main, "import \"host\" as host\nprint host.value()\n").expect("write entrypoint");

    let error = compile_file(main).expect_err("unsigned central authority setup must fail closed");
    assert!(
        error.to_string().contains("central root") || error.to_string().contains("central issuer"),
        "{error}"
    );
}
