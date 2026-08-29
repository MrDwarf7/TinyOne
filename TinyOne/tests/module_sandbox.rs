use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use tinyone::{Program, TinyOneError, compile_file, run_program, write_binary_artifact};

struct TestDir(PathBuf);

impl TestDir {
    fn new(label: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "tinyone-module-sandbox-{label}-{}-{stamp}",
            std::process::id()
        ));
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

fn write_fs_module(project: &TestDir, manifest_module: &str) -> PathBuf {
    fs::write(
        project.path().join("tinyone.json"),
        format!(r#"{{"modules":{{"host":{manifest_module}}}}}"#),
    )
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
    let main = write_fs_module(
        &project,
        r#"{"path":"host.to","capabilities":["filesystem"]}"#,
    );
    let program = compile_file(main).expect("compile module with declared grant");
    assert_eq!(program.modules()[0].capabilities(), ["filesystem"]);

    for mode in ["vm", "jit"] {
        assert_eq!(
            run(program.clone(), mode).expect("granted module runs"),
            "1\n"
        );
    }

    let json_round_trip = Program::from_artifact(program.to_artifact()).expect("JSON artifact");
    assert_eq!(json_round_trip.modules()[0].capabilities(), ["filesystem"]);
    assert_eq!(
        run(Arc::new(json_round_trip), "jit").expect("JSON artifact runs"),
        "1\n"
    );

    let binary_path = project.path().join("program.tob");
    write_binary_artifact(&program, &binary_path).expect("write binary artifact");
    let binary = fs::read(binary_path).expect("read binary artifact");
    let binary_round_trip = Program::from_binary_artifact(&binary).expect("binary artifact");
    assert_eq!(
        binary_round_trip.modules()[0].capabilities(),
        ["filesystem"]
    );
    assert_eq!(
        run(Arc::new(binary_round_trip), "vm").expect("binary artifact runs"),
        "1\n"
    );
}

#[test]
fn jit_direct_builtin_cannot_bypass_unsafe_memory_grant() {
    let project = TestDir::new("deny-unsafe-memory");
    fs::write(
        project.path().join("tinyone.json"),
        r#"{"modules":{"mem":"mem.to"}}"#,
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
fn string_selected_calls_cannot_turn_root_functions_into_module_deputies() {
    let project = TestDir::new("deny-root-deputy");
    fs::write(
        project.path().join("tinyone.json"),
        r#"{"modules":{"plugin":"plugin.to"}}"#,
    )
    .expect("write manifest");
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
    fs::write(
        parent.path().join("outside.to"),
        "export fn value() { return 1 }",
    )
    .expect("write external module");
    let direct_escape = project.join("main.to");
    fs::write(
        &direct_escape,
        "import \"../outside.to\" as outside\nprint outside.value()\n",
    )
    .expect("write entrypoint");
    let direct_error = compile_file(&direct_escape).expect_err("parent import must fail");
    assert!(
        direct_error
            .to_string()
            .contains("escapes the module sandbox")
    );

    fs::write(
        project.join("tinyone.json"),
        r#"{"modules":{"outside":"../outside.to"}}"#,
    )
    .expect("write escaping manifest");
    fs::write(
        &direct_escape,
        "import \"outside\" as outside\nprint outside.value()\n",
    )
    .expect("write manifest entrypoint");
    let manifest_error =
        compile_file(direct_escape).expect_err("escaping manifest target must fail");
    assert!(manifest_error.to_string().contains("escaping path"));
}

#[test]
fn config_can_explicitly_disable_the_import_sandbox_for_trusted_projects() {
    let parent = TestDir::new("sandbox-disabled");
    let project = parent.path().join("project");
    fs::create_dir(&project).expect("create project");
    fs::write(
        project.join("Config.toml"),
        "[sandbox]\nenabled = false\n\n[permissions]\ncodebase = []\n",
    )
    .expect("write Config.toml");
    fs::write(
        parent.path().join("outside.to"),
        "export fn value() { return 9 }\n",
    )
    .expect("write external module");
    let main = project.join("main.to");
    fs::write(
        &main,
        "import \"../outside.to\" as outside\nprint outside.value()\n",
    )
    .expect("write entrypoint");

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
    fs::write(
        project.path().join("host.to"),
        "export fn probe() { return fs_exists(\".\") }",
    )
    .expect("write module");
    let main = project.path().join("main.to");
    fs::write(&main, "import \"host\" as host\nprint host.probe()\n").expect("write entrypoint");

    let program = compile_file(main).expect("Config.toml module compiles without tinyone.json");
    assert_eq!(program.root_capabilities(), ["filesystem"]);
    assert_eq!(program.modules()[0].capabilities(), ["filesystem"]);
    for mode in ["vm", "jit"] {
        assert_eq!(
            run(program.clone(), mode).expect("granted module runs"),
            "1\n"
        );
    }
}

#[test]
fn config_rules_can_require_every_module_to_be_declared_in_config_toml() {
    let project = TestDir::new("configured-module-rule");
    fs::write(
        project.path().join("Config.toml"),
        "[rules]\nrequire_configured_modules = true\n",
    )
    .expect("write Config.toml");
    fs::write(
        project.path().join("tinyone.json"),
        r#"{"modules":{"legacy":"legacy.to"}}"#,
    )
    .expect("write legacy manifest");
    fs::write(
        project.path().join("legacy.to"),
        "export fn value() { return 1 }\n",
    )
    .expect("write module");
    let main = project.path().join("main.to");
    fs::write(&main, "import \"legacy\" as legacy\nprint legacy.value()\n")
        .expect("write entrypoint");

    let error = compile_file(main).expect_err("legacy-only mapping is rejected");
    assert!(
        error
            .to_string()
            .contains("is not declared in Config.toml [modules]"),
        "{error}"
    );
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
    assert!(program.root_capabilities().is_empty());
    assert_eq!(program.max_call_depth(), 1);

    let artifact = Program::from_artifact(program.to_artifact()).expect("JSON artifact");
    assert!(artifact.root_capabilities().is_empty());
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
        assert!(
            error
                .to_string()
                .contains("Call stack overflow after 1 nested call"),
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
