//! End-to-end integration tests for the Asatsuyu CLI.
//!
//! Tests build the real binary and run it as a subprocess.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Workspace root: two levels up from this crate's manifest dir.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

/// Build and return a `Command` pointing at the asatsuyu-cli binary,
/// with working directory set to the workspace root.
fn asatsuyu() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_asatsuyu-cli"));
    cmd.current_dir(workspace_root());
    cmd
}

/// Path to examples relative to the workspace root.
fn example(name: &str) -> String {
    format!("examples/{name}")
}

// ── 1. run hello.asty ──────────────────────────────────────────────

#[test]
fn run_hello_asty() {
    let output = asatsuyu().args(["run", &example("hello.asty")]).output().unwrap();
    assert!(
        output.status.success(),
        "exit code: {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
}

// ── 2. build hello.asty ────────────────────────────────────────────

#[test]
fn build_hello_asty() {
    // Use a unique output dir to avoid test interference.
    let dir = workspace_root().join("target/test-dist-hello");
    let dir_str = dir.display().to_string();
    let output =
        asatsuyu().args(["build", &example("hello.asty"), "-o", &dir_str]).output().unwrap();

    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("test-dist-hello"), "stdout should contain output dir: {stdout}",);

    // Verify the package tree was created (python/ layout).
    let py_path = dir.join("python/hello/hello.py");
    let content = std::fs::read_to_string(&py_path).expect("generated .py should exist");
    assert!(content.contains("def main()"), "generated Python: {content}");
    assert!(content.contains("return 42"), "generated Python: {content}");

    // Verify __init__.py, py.typed, and pyproject.toml exist.
    assert!(dir.join("python/hello/__init__.py").exists(), "__init__.py should exist");
    assert!(dir.join("python/hello/py.typed").exists(), "py.typed should exist");
    assert!(dir.join("pyproject.toml").exists(), "pyproject.toml should exist");
    assert!(dir.join("python/hello/__main__.py").exists(), "__main__.py should exist");
    assert!(
        !dir.join("python/hello/asatsuyu_prelude.py").exists(),
        "unused prelude should be omitted",
    );

    // Clean up.
    let _ = std::fs::remove_dir_all(&dir);
}

// ── 3. check hello.asty ────────────────────────────────────────────

#[test]
fn check_hello_asty() {
    let output = asatsuyu().args(["check", &example("hello.asty")]).output().unwrap();
    assert!(output.status.success());
    // check is silent on success.
    assert!(output.stdout.is_empty(), "stdout should be empty on success");
}

#[test]
fn check_multiple_files() {
    let output = asatsuyu()
        .args(["check", &example("hello.asty"), &example("greet.asty")])
        .output()
        .unwrap();
    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
}

// ── 4. check with type error ───────────────────────────────────────

#[test]
fn check_type_error() {
    // Create a temp file with a type error.
    let dir = workspace_root().join("target/test-type-error");
    std::fs::create_dir_all(&dir).unwrap();
    let bad_file = dir.join("bad.asty");
    std::fs::write(&bad_file, "fn f() -> Int { \"hello\" }").unwrap();

    let bad_file_str = bad_file.display().to_string();
    let output = asatsuyu().args(["check", &bad_file_str]).output().unwrap();
    assert!(!output.status.success(), "should fail on type error");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("type mismatch"), "stderr: {stderr}");

    let _ = std::fs::remove_dir_all(&dir);
}

// ── 4b. check shows source context via miette ─────────────────────

#[test]
fn check_type_error_shows_source_context() {
    let dir = workspace_root().join("target/test-source-ctx");
    std::fs::create_dir_all(&dir).unwrap();
    let bad_file = dir.join("ctx.asty");
    std::fs::write(&bad_file, "fn f() -> Int { \"hello\" }").unwrap();

    let bad_file_str = bad_file.display().to_string();
    let output = asatsuyu().args(["check", &bad_file_str]).output().unwrap();
    assert!(!output.status.success());

    let stderr = String::from_utf8_lossy(&output.stderr);
    // miette should show the actual source line.
    assert!(
        stderr.contains("fn f() -> Int"),
        "stderr should contain source code snippet: {stderr}",
    );
    // miette should show the filename.
    assert!(stderr.contains("ctx.asty"), "stderr should show filename: {stderr}");

    let _ = std::fs::remove_dir_all(&dir);
}

// ── 5. run missing file ────────────────────────────────────────────

#[test]
fn run_missing_file() {
    let output = asatsuyu().args(["run", "nonexistent.asty"]).output().unwrap();
    assert!(!output.status.success());

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.is_empty(), "should report error for missing file");
}

// ── 6. build greet.asty ────────────────────────────────────────────

#[test]
fn build_greet_asty() {
    let dir = workspace_root().join("target/test-dist-greet");
    let dir_str = dir.display().to_string();
    let output =
        asatsuyu().args(["build", &example("greet.asty"), "-o", &dir_str]).output().unwrap();

    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));

    let content = std::fs::read_to_string(dir.join("python/greet/greet.py"))
        .expect("generated .py should exist");
    assert!(content.contains("def greet(name: str) -> str:"), "content: {content}");
    assert!(content.contains("def add(x: int, y: int) -> int:"), "content: {content}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_requests_checked_ffi_emits_runtime_files() {
    let dir = workspace_root().join("target/test-dist-requests");
    let src_dir = workspace_root().join("target/test-src-requests");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&src_dir);
    std::fs::create_dir_all(&src_dir).unwrap();

    let sample = src_dir.join("requests_sample.asty");
    std::fs::write(
        &sample,
        "from python import requests\npub fn main() -> Int {\n  let response = requests.get(\"https://example.test\")\n  response.status_code\n}\n",
    )
    .unwrap();

    let sample_str = sample.display().to_string();
    let dir_str = dir.display().to_string();
    let output = asatsuyu().args(["build", &sample_str, "-o", &dir_str]).output().unwrap();

    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    assert!(
        dir.join("python/requests_sample/asatsuyu_prelude.py").exists(),
        "Checked FFI should emit prelude",
    );
    assert!(
        dir.join("python/requests_sample/_asatsuyu_runtime.py").exists(),
        "Checked FFI should emit runtime shim",
    );
    assert!(
        dir.join("python/requests_sample/_asatsuyu_runtime.pyi").exists(),
        "Checked FFI should emit type stubs",
    );
    assert!(dir.join("python/requests_sample/py.typed").exists(), "py.typed should exist");
    assert!(dir.join("Cargo.toml").exists(), "maturin Cargo.toml should exist");
    assert!(dir.join("src/lib.rs").exists(), "maturin src/lib.rs should exist");

    let cargo_toml =
        std::fs::read_to_string(dir.join("Cargo.toml")).expect("Cargo.toml should exist");
    assert!(!cargo_toml.contains("PATH_TO_RUNTIME"), "runtime crate path should be resolved");
    assert!(
        cargo_toml.contains("crates/asatsuyu-runtime-python"),
        "Cargo.toml should point at the runtime crate: {cargo_toml}",
    );

    let pyproject =
        std::fs::read_to_string(dir.join("pyproject.toml")).expect("pyproject.toml should exist");
    assert!(pyproject.contains("maturin"), "should use maturin backend: {pyproject}");

    let py =
        std::fs::read_to_string(dir.join("python/requests_sample/requests_sample.py")).unwrap();
    assert!(
        py.contains("_asatsuyu_runtime.call_function(_checked_runtime_requests, \"get\""),
        "generated python should use Checked FFI runtime: {py}",
    );

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&src_dir);
}

#[test]
fn run_requests_checked_ffi_with_local_stub_module() {
    let src_dir = workspace_root().join("target/test-run-requests");
    let run_dir = workspace_root().join("target/run");
    let _ = std::fs::remove_dir_all(&src_dir);
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::create_dir_all(&run_dir).unwrap();

    let sample = src_dir.join("requests_run.asty");
    std::fs::write(
        &sample,
        "from python import requests\npub fn main() -> Int {\n  let response = requests.get(\"https://example.test\")\n  response.status_code\n}\n",
    )
    .unwrap();
    // Stub requests module must be in python/ subdirectory (new layout).
    let python_run_dir = run_dir.join("python");
    std::fs::create_dir_all(&python_run_dir).unwrap();
    std::fs::write(
        python_run_dir.join("requests.py"),
        "class Response:\n    def __init__(self, status_code: int, text: str):\n        self.status_code = status_code\n        self.text = text\n\n    def json(self):\n        return {\"ok\": True}\n\n\ndef _make_response(url: str):\n    return Response(204, f\"stub:{url}\")\n\n\ndef get(url: str):\n    return _make_response(url)\n\n\ndef post(url: str):\n    return _make_response(url)\n\n\ndef put(url: str):\n    return _make_response(url)\n\n\ndef delete(url: str):\n    return _make_response(url)\n",
    )
    .unwrap();

    let sample_str = sample.display().to_string();
    let output = asatsuyu().args(["run", &sample_str]).output().unwrap();
    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));

    let _ = std::fs::remove_file(python_run_dir.join("requests.py"));
    let _ = std::fs::remove_dir_all(src_dir);
}

// ── 7. build shows summary on stderr ──────────────────────────────

#[test]
fn build_shows_summary() {
    let dir = workspace_root().join("target/test-dist-summary");
    let dir_str = dir.display().to_string();
    let output =
        asatsuyu().args(["build", &example("hello.asty"), "-o", &dir_str]).output().unwrap();

    assert!(output.status.success());

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Compiled hello"), "summary on stderr: {stderr}");
    assert!(stderr.contains("files"), "file count in summary: {stderr}");

    let _ = std::fs::remove_dir_all(&dir);
}

// ── 8. check error summary ────────────────────────────────────────

#[test]
fn check_error_summary() {
    let dir = workspace_root().join("target/test-error-summary");
    std::fs::create_dir_all(&dir).unwrap();
    let bad_file = dir.join("bad.asty");
    std::fs::write(&bad_file, "fn f() -> Int { \"hello\" }").unwrap();

    let bad_file_str = bad_file.display().to_string();
    let output = asatsuyu().args(["check", &bad_file_str]).output().unwrap();
    assert!(!output.status.success());

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("aborting due to"), "error summary: {stderr}");

    let _ = std::fs::remove_dir_all(&dir);
}

// ── 9. new creates project ────────────────────────────────────────

#[test]
fn new_creates_project() {
    let dir = workspace_root().join("target/test-new-project");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let output = asatsuyu().current_dir(&dir).args(["new", "demo"]).output().unwrap();
    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));

    // Verify project structure.
    assert!(dir.join("demo/src/main.asty").exists(), "main.asty should exist");
    assert!(dir.join("demo/asatsuyu.toml").exists(), "asatsuyu.toml should exist");
    assert!(dir.join("demo/.gitignore").exists(), ".gitignore should exist");

    // Verify asatsuyu.toml content.
    let toml = std::fs::read_to_string(dir.join("demo/asatsuyu.toml")).unwrap();
    assert!(toml.contains("schema_version = 1"), "toml schema_version: {toml}");
    assert!(toml.contains("name = \"demo\""), "toml name: {toml}");

    // Verify main.asty content.
    let main = std::fs::read_to_string(dir.join("demo/src/main.asty")).unwrap();
    assert!(main.contains("pub fn main()"), "main fn: {main}");

    let _ = std::fs::remove_dir_all(&dir);
}

// ── 10. new then run ──────────────────────────────────────────────

#[test]
fn new_then_run() {
    let dir = workspace_root().join("target/test-new-run");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // Create project.
    let output = asatsuyu().current_dir(&dir).args(["new", "myapp"]).output().unwrap();
    assert!(output.status.success());

    // Run the project.
    let main_path = dir.join("myapp/src/main.asty");
    let main_str = main_path.display().to_string();
    let output = asatsuyu().args(["run", &main_str]).output().unwrap();
    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr),);

    let _ = std::fs::remove_dir_all(&dir);
}

// ── 11. new rejects existing directory ─────────────────────────────

#[test]
fn new_rejects_existing_dir() {
    let dir = workspace_root().join("target/test-new-exists");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // Create first time — should succeed.
    let output = asatsuyu().current_dir(&dir).args(["new", "dup"]).output().unwrap();
    assert!(output.status.success());

    // Create again — should fail.
    let output = asatsuyu().current_dir(&dir).args(["new", "dup"]).output().unwrap();
    assert!(!output.status.success());

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("already exists"), "stderr: {stderr}");

    let _ = std::fs::remove_dir_all(&dir);
}

// ── 12. verify-ffi ────────────────────────────────────────────────

#[test]
fn verify_ffi_outputs_trust_report() {
    let output = asatsuyu().args(["verify-ffi"]).output().unwrap();
    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("pathlib"), "should list pathlib: {stdout}");
    assert!(stdout.contains("json"), "should list json: {stdout}");
    assert!(stdout.contains("os"), "should list os: {stdout}");
    assert!(stdout.contains("sys"), "should list sys: {stdout}");
    assert!(stdout.contains("requests"), "should list requests: {stdout}");
    assert!(stdout.contains("Verified"), "should show Verified: {stdout}");
    assert!(stdout.contains("Checked"), "should show Checked: {stdout}");
    assert!(stdout.contains("Summary"), "should show summary: {stdout}");
}

// ── 13. --ffi-stdlib-only blocks third-party imports ─────────────

#[test]
fn check_ffi_stdlib_only_rejects_requests() {
    // Write a temporary .asty file that imports requests.
    let dir = workspace_root().join("target/test-stdlib-only");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("requests_import.asty");
    std::fs::write(&src, "from python import requests\npub fn main() { 42 }\n").unwrap();

    let output = asatsuyu()
        .args(["check", "--ffi-stdlib-only", &src.display().to_string()])
        .output()
        .unwrap();

    assert!(!output.status.success(), "should fail with --ffi-stdlib-only");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("E0208"), "should contain E0208: {stderr}");
    assert!(stderr.contains("requests"), "should mention requests: {stderr}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_ffi_stdlib_only_allows_stdlib() {
    let output = asatsuyu()
        .args(["check", "--ffi-stdlib-only", &example("ffi_pathlib.asty")])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stdlib imports should pass with --ffi-stdlib-only\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// ── 14. --no-emit-package emits module only ──────────────────────

#[test]
fn build_no_emit_package_emits_module_only() {
    let dir = workspace_root().join("target/test-no-pkg");
    let dir_str = dir.display().to_string();
    let _ = std::fs::remove_dir_all(&dir);

    let output = asatsuyu()
        .args(["build", &example("hello.asty"), "-o", &dir_str, "--no-emit-package"])
        .output()
        .unwrap();

    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));

    // Should have a single .py file, no pyproject.toml.
    let py_path = dir.join("hello.py");
    assert!(py_path.exists(), "hello.py should exist");
    let content = std::fs::read_to_string(&py_path).unwrap();
    assert!(content.contains("def main()"), "output: {content}");

    let pyproject = dir.join("pyproject.toml");
    assert!(!pyproject.exists(), "pyproject.toml should NOT exist with --no-emit-package");

    let _ = std::fs::remove_dir_all(&dir);
}

// ── 15. --ffi-runtime off suppresses Cargo.toml ─────────────────

#[test]
fn build_ffi_runtime_off_no_cargo_toml() {
    // Write a source that would normally trigger Checked FFI.
    let dir = workspace_root().join("target/test-ffi-rt-off");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("checked.asty");
    std::fs::write(
        &src,
        "from python import requests\npub fn main() -> Int {\n  let response = requests.get(\"https://example.test\")\n  response.status_code\n}\n",
    )
    .unwrap();
    let out_dir = dir.join("out");

    let output = asatsuyu()
        .args([
            "build",
            &src.display().to_string(),
            "-o",
            &out_dir.display().to_string(),
            "--ffi-runtime",
            "off",
        ])
        .output()
        .unwrap();

    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));

    // Cargo.toml should NOT be present with --ffi-runtime off.
    let cargo_toml = out_dir.join("Cargo.toml");
    assert!(!cargo_toml.exists(), "Cargo.toml should NOT exist with --ffi-runtime off");

    let _ = std::fs::remove_dir_all(&dir);
}

// ── 16. help text shows FFI flags ────────────────────────────────

#[test]
fn help_shows_ffi_flags() {
    let output = asatsuyu().args(["build", "--help"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stdout.contains("--ffi-runtime"), "help should mention --ffi-runtime: {stdout}");
    assert!(
        stdout.contains("--ffi-stdlib-only"),
        "help should mention --ffi-stdlib-only: {stdout}"
    );
    assert!(stdout.contains("--ffi-stub-path"), "help should mention --ffi-stub-path: {stdout}");
    assert!(
        stdout.contains("--no-emit-package"),
        "help should mention --no-emit-package: {stdout}"
    );
}

// ── 17. --error-format json ───────────────────────────────────────

#[test]
fn check_error_format_json_produces_ndjson() {
    let dir = workspace_root().join("target/test-json-output");
    std::fs::create_dir_all(&dir).unwrap();
    let bad_file = dir.join("bad.asty");
    std::fs::write(&bad_file, "fn f() -> Int { \"hello\" }").unwrap();

    let bad_file_str = bad_file.display().to_string();
    let output =
        asatsuyu().args(["check", "--error-format", "json", &bad_file_str]).output().unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty(), "stdout should be empty for check");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let lines: Vec<&str> = stderr.lines().collect();

    // At least one diagnostic line + one summary line.
    assert!(lines.len() >= 2, "expected at least 2 NDJSON lines, got: {stderr}");

    // Every line must be valid JSON.
    for line in &lines {
        let _: serde_json::Value =
            serde_json::from_str(line).unwrap_or_else(|e| panic!("invalid JSON line: {e}\n{line}"));
    }

    // First line should be a diagnostic.
    let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(first["type"], "diagnostic");
    assert_eq!(first["severity"], "error");

    // Last line should be the summary.
    let last: serde_json::Value = serde_json::from_str(lines[lines.len() - 1]).unwrap();
    assert_eq!(last["type"], "summary");
    assert!(last["error_count"].as_u64().unwrap() > 0);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_error_format_json_schema_fields() {
    let dir = workspace_root().join("target/test-json-schema");
    std::fs::create_dir_all(&dir).unwrap();
    let bad_file = dir.join("schema.asty");
    std::fs::write(&bad_file, "fn f() -> Int { \"hello\" }").unwrap();

    let bad_file_str = bad_file.display().to_string();
    let output =
        asatsuyu().args(["check", "--error-format", "json", &bad_file_str]).output().unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    let first_line = stderr.lines().next().unwrap();
    let diag: serde_json::Value = serde_json::from_str(first_line).unwrap();

    // Required fields exist.
    assert!(diag["code"].is_string(), "code should be a string: {diag}");
    assert!(diag["message"].is_string(), "message should be a string: {diag}");
    assert!(diag["file"].is_string(), "file should be a string: {diag}");

    // Span positions are 1-based.
    let start_line = diag["span"]["start"]["line"].as_u64().unwrap();
    let start_col = diag["span"]["start"]["column"].as_u64().unwrap();
    assert!(start_line >= 1, "line should be 1-based: {start_line}");
    assert!(start_col >= 1, "column should be 1-based: {start_col}");

    // Offset is present.
    assert!(diag["span"]["start"]["offset"].is_u64(), "offset should be present");

    // Labels array exists.
    assert!(diag["labels"].is_array(), "labels should be an array");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_success_json_emits_summary_only() {
    let output = asatsuyu()
        .args(["check", "--error-format", "json", &example("hello.asty")])
        .output()
        .unwrap();

    assert!(output.status.success());

    let stderr = String::from_utf8_lossy(&output.stderr);
    let lines: Vec<&str> = stderr.lines().collect();

    // Success: exactly one summary line (no diagnostics).
    assert_eq!(lines.len(), 1, "expected exactly 1 line (summary), got: {stderr}");

    let summary: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(summary["type"], "summary");
    assert_eq!(summary["error_count"], 0);
    assert_eq!(summary["warning_count"], 0);
    assert_eq!(summary["note_count"], 0);
}

#[test]
fn build_error_format_json() {
    let dir = workspace_root().join("target/test-json-build");
    std::fs::create_dir_all(&dir).unwrap();
    let bad_file = dir.join("bad.asty");
    std::fs::write(&bad_file, "fn f() -> Int { \"hello\" }").unwrap();

    let bad_file_str = bad_file.display().to_string();
    let output = asatsuyu()
        .args([
            "build",
            "--error-format",
            "json",
            &bad_file_str,
            "-o",
            &dir.join("out").display().to_string(),
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());

    let stderr = String::from_utf8_lossy(&output.stderr);
    let lines: Vec<&str> = stderr.lines().collect();
    assert!(lines.len() >= 2, "expected NDJSON output: {stderr}");

    // Same schema as check.
    let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(first["type"], "diagnostic");

    let last: serde_json::Value = serde_json::from_str(lines[lines.len() - 1]).unwrap();
    assert_eq!(last["type"], "summary");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_success_json_emits_summary_only() {
    let dir = workspace_root().join("target/test-json-build-success");
    let _ = std::fs::remove_dir_all(&dir);

    let output = asatsuyu()
        .args([
            "build",
            "--error-format",
            "json",
            &example("hello.asty"),
            "-o",
            &dir.display().to_string(),
        ])
        .output()
        .unwrap();

    assert!(output.status.success(), "build should succeed");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let lines: Vec<&str> = stderr.lines().collect();
    assert_eq!(lines.len(), 1, "expected exactly 1 summary line, got: {stderr}");

    let summary: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(summary["type"], "summary");
    assert_eq!(summary["error_count"], 0);
    assert_eq!(summary["warning_count"], 0);
    assert_eq!(summary["note_count"], 0);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_success_json_emits_summary_only() {
    let output = asatsuyu()
        .args(["run", "--error-format", "json", &example("hello.asty")])
        .output()
        .unwrap();

    assert!(output.status.success(), "run should succeed");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let lines: Vec<&str> = stderr.lines().collect();
    assert_eq!(lines.len(), 1, "expected exactly 1 summary line, got: {stderr}");

    let summary: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(summary["type"], "summary");
    assert_eq!(summary["error_count"], 0);
    assert_eq!(summary["warning_count"], 0);
    assert_eq!(summary["note_count"], 0);
}

#[test]
fn check_error_format_human_is_default() {
    // Omitting --error-format should produce miette output, not JSON.
    let dir = workspace_root().join("target/test-default-human");
    std::fs::create_dir_all(&dir).unwrap();
    let bad_file = dir.join("bad.asty");
    std::fs::write(&bad_file, "fn f() -> Int { \"hello\" }").unwrap();

    let bad_file_str = bad_file.display().to_string();
    let output = asatsuyu().args(["check", &bad_file_str]).output().unwrap();

    assert!(!output.status.success());

    let stderr = String::from_utf8_lossy(&output.stderr);
    // Human output should NOT be valid JSON.
    assert!(
        serde_json::from_str::<serde_json::Value>(stderr.lines().next().unwrap_or("")).is_err(),
        "default output should be human-readable, not JSON: {stderr}",
    );
    // Should contain miette-style output markers.
    assert!(stderr.contains("type mismatch"), "stderr: {stderr}");

    let _ = std::fs::remove_dir_all(&dir);
}

// ── 17c. project discovery ────────────────────────────────────────

#[test]
fn check_no_args_in_project() {
    let dir = workspace_root().join("target/test-check-no-args");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // Create a project.
    let output = asatsuyu().current_dir(&dir).args(["new", "myproj"]).output().unwrap();
    assert!(output.status.success(), "new: {}", String::from_utf8_lossy(&output.stderr));

    // Run check with no file args from inside the project.
    let project_dir = dir.join("myproj");
    let output = asatsuyu().current_dir(&project_dir).args(["check"]).output().unwrap();
    assert!(
        output.status.success(),
        "check (no args) should succeed in project:\nstderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_no_args_outside_project() {
    let dir = workspace_root().join("target/test-check-no-project");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // Run check with no file args in a directory without asatsuyu.toml.
    let output = asatsuyu().current_dir(&dir).args(["check"]).output().unwrap();
    assert!(!output.status.success(), "should fail without project");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("asatsuyu.toml"), "error should mention asatsuyu.toml: {stderr}",);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_watch_flag_in_help() {
    let output = asatsuyu().args(["check", "--help"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--watch"), "help should mention --watch: {stdout}");
}

// ── 17d. --ffi-stub-path must point at an existing directory ────

#[test]
fn check_rejects_missing_ffi_stub_path() {
    let missing = workspace_root().join("target/does-not-exist-stubs");
    let _ = std::fs::remove_dir_all(&missing);

    let output = asatsuyu()
        .args(["check", &example("hello.asty"), "--ffi-stub-path", &missing.display().to_string()])
        .output()
        .unwrap();

    assert!(!output.status.success(), "should fail with missing --ffi-stub-path");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("invalid --ffi-stub-path"),
        "stderr should mention invalid stub path: {stderr}"
    );
    assert!(stderr.contains("does not exist"), "stderr should explain failure: {stderr}");
}

// ── 18. check and build produce the same diagnostics ──────────────

#[test]
fn check_and_build_produce_same_diagnostic_codes() {
    let dir = workspace_root().join("target/test-check-build-contract");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("contract.asty");
    std::fs::write(&src, "fn f() -> Int { \"hello\" }\n").unwrap();
    let src_str = src.display().to_string();

    let check_output = asatsuyu().args(["check", &src_str]).output().unwrap();
    let build_output = asatsuyu()
        .args(["build", &src_str, "-o", &dir.join("out").display().to_string()])
        .output()
        .unwrap();

    let check_stderr = String::from_utf8_lossy(&check_output.stderr);
    let build_stderr = String::from_utf8_lossy(&build_output.stderr);

    // Both should fail with the same diagnostic code.
    assert!(!check_output.status.success(), "check should fail");
    assert!(!build_output.status.success(), "build should fail");
    assert!(check_stderr.contains("E0200"), "check should contain E0200: {check_stderr}");
    assert!(build_stderr.contains("E0200"), "build should contain E0200: {build_stderr}");

    let _ = std::fs::remove_dir_all(&dir);
}

// ── 19. exit code contract tests ────────────────────────────────

/// Compile errors produce exit code 1.
#[test]
fn exit_code_compile_error_is_1() {
    let dir = workspace_root().join("target/test-exit-code-1");
    std::fs::create_dir_all(&dir).unwrap();
    let bad_file = dir.join("bad.asty");
    std::fs::write(&bad_file, "fn f() -> Int { \"hello\" }").unwrap();

    let bad_file_str = bad_file.display().to_string();
    let output = asatsuyu().args(["check", &bad_file_str]).output().unwrap();
    assert_eq!(output.status.code(), Some(1), "compile error should exit with code 1");

    let _ = std::fs::remove_dir_all(&dir);
}

/// Config/usage errors produce exit code 2.
#[test]
fn exit_code_missing_file_is_2() {
    let output = asatsuyu().args(["check", "nonexistent-file.asty"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2), "missing file should exit with code 2");
}

/// Invalid --ffi-stub-path produces exit code 2.
#[test]
fn exit_code_invalid_ffi_stub_path_is_2() {
    let output = asatsuyu()
        .args(["check", &example("hello.asty"), "--ffi-stub-path", "/nonexistent-dir"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "invalid --ffi-stub-path should exit with code 2");
}

/// `new` validation errors produce exit code 2.
#[test]
fn exit_code_new_validation_is_2() {
    let output = asatsuyu().args(["new", "bad name!"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2), "invalid project name should exit with code 2");
}

// ── 20. summary contract tests ──────────────────────────────────

/// `run` with compile error + JSON still emits summary.
#[test]
fn run_compile_error_json_emits_summary() {
    let dir = workspace_root().join("target/test-run-json-err");
    std::fs::create_dir_all(&dir).unwrap();
    let bad_file = dir.join("bad.asty");
    std::fs::write(&bad_file, "fn f() -> Int { \"hello\" }").unwrap();

    let bad_file_str = bad_file.display().to_string();
    let output =
        asatsuyu().args(["run", "--error-format", "json", &bad_file_str]).output().unwrap();

    assert_eq!(output.status.code(), Some(1), "compile error should exit with code 1");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let lines: Vec<&str> = stderr.lines().collect();
    assert!(lines.len() >= 2, "should have diagnostics + summary: {stderr}");

    let last: serde_json::Value = serde_json::from_str(lines[lines.len() - 1]).unwrap();
    assert_eq!(last["type"], "summary", "last line should be summary: {stderr}");
    assert!(last["error_count"].as_u64().unwrap() > 0);

    let _ = std::fs::remove_dir_all(&dir);
}

// ── 21. stdout/stderr contract tests ────────────────────────────

/// `check` success produces empty stdout.
#[test]
fn check_success_stdout_is_empty() {
    let output = asatsuyu().args(["check", &example("hello.asty")]).output().unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty(), "check stdout should be empty on success");
}

/// `build` success produces output path on stdout.
#[test]
fn build_success_stdout_has_path() {
    let dir = workspace_root().join("target/test-stdout-path");
    let dir_str = dir.display().to_string();
    let output =
        asatsuyu().args(["build", &example("hello.asty"), "-o", &dir_str]).output().unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.trim().is_empty(), "build stdout should contain output path: {stdout}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// `new` produces empty stdout (messages go to stderr).
#[test]
fn new_stdout_is_empty() {
    let dir = workspace_root().join("target/test-new-stdout");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let output = asatsuyu().current_dir(&dir).args(["new", "check_stdout"]).output().unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty(), "new stdout should be empty");

    let _ = std::fs::remove_dir_all(&dir);
}

// ── 22. asatsuyu.toml schema contract tests ─────────────────────

/// Unknown keys in asatsuyu.toml produce a clear error.
#[test]
fn check_rejects_unknown_toml_key() {
    let dir = workspace_root().join("target/test-unknown-toml-key");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/main.asty"), "pub fn main() { 42 }").unwrap();
    // Write an asatsuyu.toml with an unknown key.
    std::fs::write(
        dir.join("asatsuyu.toml"),
        "[project]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[unknown]\nfoo = 1\n",
    )
    .unwrap();

    let output = asatsuyu().current_dir(&dir).args(["check"]).output().unwrap();
    assert!(!output.status.success(), "should fail with unknown section");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown field"), "error should mention unknown field: {stderr}");

    let _ = std::fs::remove_dir_all(&dir);
}

// ── 23. Python dependency model tests ──────────────────────────────

/// Invalid PEP 440 specifier in [python-dependencies] exits with code 2.
#[test]
fn check_invalid_pep440_specifier_exits_2() {
    let dir = workspace_root().join("target/test-invalid-pep440");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/main.asty"), "pub fn main() { 42 }").unwrap();
    std::fs::write(
        dir.join("asatsuyu.toml"),
        "[project]\nname = \"demo\"\n\n[python-dependencies]\nrequests = \"not a version\"\n",
    )
    .unwrap();

    let output = asatsuyu().current_dir(&dir).args(["check"]).output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(2),
        "invalid PEP 440 specifier should exit 2\nstderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("invalid PEP 440 specifier"), "error should mention PEP 440: {stderr}",);

    let _ = std::fs::remove_dir_all(&dir);
}

/// Valid PEP 440 specifiers in [python-dependencies] parse without config error.
#[test]
fn check_valid_pep440_config_parses() {
    let dir = workspace_root().join("target/test-valid-pep440");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/main.asty"), "pub fn main() { 42 }").unwrap();
    std::fs::write(
        dir.join("asatsuyu.toml"),
        "[project]\nname = \"demo\"\n\n[python-dependencies]\nrequests = \">=2.31\"\nflask = \">=3.0,<4.0\"\n",
    )
    .unwrap();

    let output = asatsuyu().current_dir(&dir).args(["check"]).output().unwrap();
    // Should not fail with exit code 2 (config error).
    // It may still succeed or warn about missing packages, but not exit 2 for config.
    assert_ne!(
        output.status.code(),
        Some(2),
        "valid PEP 440 specifiers should not cause config error\nstderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Explicit `check` paths still honor project dependency checking.
#[test]
fn check_explicit_path_runs_dependency_check() {
    let dir = workspace_root().join("target/test-check-explicit-path-deps");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/main.asty"), "pub fn main() { 42 }").unwrap();
    std::fs::write(
        dir.join("asatsuyu.toml"),
        "[project]\nname = \"demo\"\n\n[python-dependencies]\ndefinitely-missing-package = \">=1.0\"\n",
    )
    .unwrap();

    let output = asatsuyu().current_dir(&dir).args(["check", "src/main.asty"]).output().unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("definitely-missing-package")
            || stderr.contains("dependency check skipped"),
        "explicit path check should still run dependency discovery:\nstderr: {stderr}",
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// [python] path field is accepted in asatsuyu.toml.
#[test]
fn check_python_path_config_field_accepted() {
    let dir = workspace_root().join("target/test-python-path-config");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/main.asty"), "pub fn main() { 42 }").unwrap();
    std::fs::write(
        dir.join("asatsuyu.toml"),
        "[project]\nname = \"demo\"\n\n[python]\nversion = \">=3.12\"\npath = \"/usr/bin/python3\"\n",
    )
    .unwrap();

    let output = asatsuyu().current_dir(&dir).args(["check"]).output().unwrap();
    // The path field should be parsed without causing an "unknown field" error.
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unknown field"),
        "python.path should not be rejected as unknown: {stderr}",
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// --python-path flag is accepted by check command.
#[test]
fn check_python_path_cli_flag_accepted() {
    let output = asatsuyu()
        .args(["check", "--python-path", "/usr/bin/python3", &example("hello.asty")])
        .output()
        .unwrap();

    // The flag should be accepted without error. The compilation itself
    // should still succeed since hello.asty has no Python dependencies.
    assert!(
        output.status.success(),
        "check with --python-path should succeed:\nstderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );
}

/// Invalid [python] version specifier is rejected.
#[test]
fn check_invalid_python_version_exits_2() {
    let dir = workspace_root().join("target/test-invalid-py-ver");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/main.asty"), "pub fn main() { 42 }").unwrap();
    std::fs::write(
        dir.join("asatsuyu.toml"),
        "[project]\nname = \"demo\"\n\n[python]\nversion = \"bad\"\n",
    )
    .unwrap();

    let output = asatsuyu().current_dir(&dir).args(["check"]).output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(2),
        "invalid python version should exit 2\nstderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("python.version"), "error should mention python.version: {stderr}",);

    let _ = std::fs::remove_dir_all(&dir);
}

// ── 24. pyproject.toml generation tests ────────────────────────────

/// build inside a project uses asatsuyu.toml name/version in pyproject.toml.
#[test]
fn build_uses_project_name_and_version() {
    let dir = workspace_root().join("target/test-build-project-meta");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/main.asty"), "pub fn main() { 42 }").unwrap();
    std::fs::write(
        dir.join("asatsuyu.toml"),
        "schema_version = 1\n\n[project]\nname = \"my-tool\"\nversion = \"1.2.3\"\n\n[python]\nversion = \">=3.13\"\n",
    )
    .unwrap();

    let out_dir = dir.join("dist");
    let out_dir_str = out_dir.display().to_string();
    let main_path = dir.join("src/main.asty");
    let main_path_str = main_path.display().to_string();

    let output = asatsuyu().args(["build", &main_path_str, "-o", &out_dir_str]).output().unwrap();

    assert!(
        output.status.success(),
        "build should succeed:\nstderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );

    let pyproject = std::fs::read_to_string(out_dir.join("pyproject.toml"))
        .expect("pyproject.toml should exist");
    assert!(pyproject.contains("name = \"my-tool\""), "should use project name: {pyproject}",);
    assert!(pyproject.contains("version = \"1.2.3\""), "should use project version: {pyproject}",);
    assert!(
        pyproject.contains("requires-python = \">=3.13\""),
        "should use python version: {pyproject}",
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ── lock command ─────────────────────────────────────────────────

/// `asatsuyu lock` outside a project fails with exit code 2.
#[test]
fn lock_without_project_fails() {
    let dir = workspace_root().join("target/test-lock-no-project");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let output = {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_asatsuyu-cli"));
        cmd.current_dir(&dir);
        cmd.args(["lock"]).output().unwrap()
    };

    assert_eq!(
        output.status.code(),
        Some(2),
        "lock outside project should exit 2\nstderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// `asatsuyu lock` with no python-dependencies succeeds silently.
#[test]
fn lock_empty_deps_succeeds() {
    let dir = workspace_root().join("target/test-lock-empty-deps");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/main.asty"), "pub fn main() { 42 }").unwrap();
    std::fs::write(
        dir.join("asatsuyu.toml"),
        "schema_version = 1\n\n[project]\nname = \"empty\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();

    let output = {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_asatsuyu-cli"));
        cmd.current_dir(&dir);
        cmd.args(["lock"]).output().unwrap()
    };

    assert!(
        output.status.success(),
        "lock with no deps should succeed\nstderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    // No pylock.toml should be created.
    assert!(!dir.join("pylock.toml").exists(), "pylock.toml should not be created when no deps");

    let _ = std::fs::remove_dir_all(&dir);
}

/// `asatsuyu lock --help` includes the lock subcommand.
#[test]
fn lock_help_shows_subcommand() {
    let output = asatsuyu().args(["lock", "--help"]).output().unwrap();
    assert!(output.status.success(), "lock --help should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("lockfile") || stdout.contains("pylock"),
        "help should describe lockfile: {stdout}",
    );
}

/// `asatsuyu lock --error-format json` reports errors as JSON.
#[test]
fn lock_json_error_output() {
    let dir = workspace_root().join("target/test-lock-json-error");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let output = {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_asatsuyu-cli"));
        cmd.current_dir(&dir);
        cmd.args(["lock", "--error-format", "json"]).output().unwrap()
    };

    assert_eq!(output.status.code(), Some(2), "lock json error should exit 2");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"type\":\"lockfile\""), "should emit lockfile json: {stdout}");
    assert!(stdout.contains("\"status\":\"error\""), "should emit error status: {stdout}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// `check --error-format json` reports missing lockfile as a JSON warning.
#[test]
fn check_json_emits_lockfile_warning() {
    let dir = workspace_root().join("target/test-check-json-lock-warning");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/main.asty"), "pub fn main() { 42 }").unwrap();
    std::fs::write(
        dir.join("asatsuyu.toml"),
        "[project]\nname = \"demo\"\n\n[python-dependencies]\nrequests = \">=2.31\"\n",
    )
    .unwrap();

    let output = {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_asatsuyu-cli"));
        cmd.current_dir(&dir);
        cmd.args(["check", "--error-format", "json"]).output().unwrap()
    };

    assert!(
        output.status.success(),
        "check should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"type\":\"lockfile\""),
        "should emit lockfile json warning: {stdout}"
    );
    assert!(stdout.contains("\"status\":\"missing\""), "should emit missing status: {stdout}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// Full integration: `asatsuyu lock` generates pylock.toml (requires uv or pip >= 25.1).
#[test]
#[ignore = "requires uv >= 0.6.15 or pip >= 25.1 on PATH"]
fn lock_generates_pylock_toml() {
    let dir = workspace_root().join("target/test-lock-generate");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/main.asty"), "pub fn main() { 42 }").unwrap();
    std::fs::write(
        dir.join("asatsuyu.toml"),
        "[project]\nname = \"locktest\"\nversion = \"0.1.0\"\n\n[python]\nversion = \">=3.12\"\n\n[python-dependencies]\nrequests = \">=2.31\"\n",
    )
    .unwrap();

    let output = {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_asatsuyu-cli"));
        cmd.current_dir(&dir);
        cmd.args(["lock"]).output().unwrap()
    };

    assert!(
        output.status.success(),
        "lock should succeed\nstderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );

    let pylock_path = dir.join("pylock.toml");
    assert!(pylock_path.exists(), "pylock.toml should be created");

    let content = std::fs::read_to_string(&pylock_path).unwrap();
    assert!(content.contains("lock-version"), "should contain lock-version: {content}");
    assert!(content.contains("requests"), "should contain requests package: {content}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// Smoke test: `pip install -r pylock.toml` works in a venv (requires lock + pip).
#[test]
#[ignore = "requires uv >= 0.6.15 or pip >= 25.1 and network access"]
fn lock_install_from_pylock() {
    let dir = workspace_root().join("target/test-lock-install");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/main.asty"), "pub fn main() { 42 }").unwrap();
    std::fs::write(
        dir.join("asatsuyu.toml"),
        "[project]\nname = \"installtest\"\nversion = \"0.1.0\"\n\n[python]\nversion = \">=3.12\"\n\n[python-dependencies]\nsix = \">=1.16\"\n",
    )
    .unwrap();

    // Generate lockfile.
    let lock_out = {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_asatsuyu-cli"));
        cmd.current_dir(&dir);
        cmd.args(["lock"]).output().unwrap()
    };
    assert!(
        lock_out.status.success(),
        "lock failed: {}",
        String::from_utf8_lossy(&lock_out.stderr)
    );

    // Create venv and install from lockfile using uv (pip >= 25.1 also works).
    let venv_dir = dir.join(".venv");
    let venv_out = Command::new("uv").args(["venv", &venv_dir.to_string_lossy()]).output().unwrap();
    assert!(venv_out.status.success(), "venv creation failed");

    let install_out = Command::new("uv")
        .args([
            "pip",
            "install",
            "-r",
            &dir.join("pylock.toml").to_string_lossy(),
            "--python",
            &venv_dir.join("bin/python").to_string_lossy(),
        ])
        .output()
        .unwrap();
    assert!(
        install_out.status.success(),
        "uv pip install from pylock.toml failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&install_out.stdout),
        String::from_utf8_lossy(&install_out.stderr),
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// build inside a project includes python-dependencies in pyproject.toml.
#[test]
fn build_includes_dependencies_in_pyproject() {
    let dir = workspace_root().join("target/test-build-deps");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/main.asty"), "pub fn main() { 42 }").unwrap();
    std::fs::write(
        dir.join("asatsuyu.toml"),
        "[project]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[python-dependencies]\nrequests = \">=2.31\"\nflask = \">=3.0\"\n",
    )
    .unwrap();

    let out_dir = dir.join("dist");
    let out_dir_str = out_dir.display().to_string();
    let main_path = dir.join("src/main.asty");
    let main_path_str = main_path.display().to_string();

    let output = asatsuyu().args(["build", &main_path_str, "-o", &out_dir_str]).output().unwrap();

    assert!(
        output.status.success(),
        "build should succeed:\nstderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );

    let pyproject = std::fs::read_to_string(out_dir.join("pyproject.toml"))
        .expect("pyproject.toml should exist");
    assert!(pyproject.contains("\"requests>=2.31\""), "should include requests dep: {pyproject}",);
    assert!(pyproject.contains("\"flask>=3.0\""), "should include flask dep: {pyproject}",);

    let _ = std::fs::remove_dir_all(&dir);
}

// ── add / remove / sync ─────────────────────────────────────────

/// `asatsuyu add` outside a project fails with exit code 2.
#[test]
fn add_without_project_fails() {
    let dir = workspace_root().join("target/test-add-no-project");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let output = {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_asatsuyu-cli"));
        cmd.current_dir(&dir);
        cmd.args(["add", "requests"]).output().unwrap()
    };
    assert_eq!(output.status.code(), Some(2), "add outside project should exit 2");

    let _ = std::fs::remove_dir_all(&dir);
}

/// `asatsuyu add` with an invalid PEP 440 specifier fails.
#[test]
fn add_rejects_invalid_specifier() {
    let dir = workspace_root().join("target/test-add-bad-spec");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/main.asty"), "pub fn main() { 42 }").unwrap();
    std::fs::write(dir.join("asatsuyu.toml"), "schema_version = 1\n\n[project]\nname = \"test\"\n")
        .unwrap();

    let output = {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_asatsuyu-cli"));
        cmd.current_dir(&dir);
        cmd.args(["add", "requests", "not-a-version"]).output().unwrap()
    };
    assert_eq!(
        output.status.code(),
        Some(2),
        "add with bad specifier should exit 2\nstderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// `asatsuyu add` writes to asatsuyu.toml.
#[test]
fn add_dependency_to_project() {
    let dir = workspace_root().join("target/test-add-dep");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/main.asty"), "pub fn main() { 42 }").unwrap();
    std::fs::write(
        dir.join("asatsuyu.toml"),
        "schema_version = 1\n\n[project]\nname = \"addtest\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();

    let output = {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_asatsuyu-cli"));
        cmd.current_dir(&dir);
        cmd.args(["add", "requests", ">=2.31"]).output().unwrap()
    };
    assert!(
        output.status.success(),
        "add should succeed\nstderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );

    let toml = std::fs::read_to_string(dir.join("asatsuyu.toml")).unwrap();
    assert!(toml.contains("requests = \">=2.31\""), "should contain requests dep:\n{toml}",);

    let _ = std::fs::remove_dir_all(&dir);
}

/// `asatsuyu remove` removes a dependency from asatsuyu.toml.
#[test]
fn remove_existing_dependency() {
    let dir = workspace_root().join("target/test-remove-dep");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/main.asty"), "pub fn main() { 42 }").unwrap();
    std::fs::write(
        dir.join("asatsuyu.toml"),
        "schema_version = 1\n\n[project]\nname = \"rmtest\"\n\n[python-dependencies]\nrequests = \">=2.31\"\nflask = \">=3.0\"\n",
    )
    .unwrap();

    let output = {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_asatsuyu-cli"));
        cmd.current_dir(&dir);
        cmd.args(["remove", "requests"]).output().unwrap()
    };
    assert!(
        output.status.success(),
        "remove should succeed\nstderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );

    let toml = std::fs::read_to_string(dir.join("asatsuyu.toml")).unwrap();
    assert!(!toml.contains("requests"), "should not contain requests:\n{toml}");
    assert!(toml.contains("flask = \">=3.0\""), "should still contain flask:\n{toml}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// `asatsuyu remove` on a nonexistent dep fails with exit 2.
#[test]
fn remove_nonexistent_fails() {
    let dir = workspace_root().join("target/test-remove-nonexistent");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/main.asty"), "pub fn main() { 42 }").unwrap();
    std::fs::write(dir.join("asatsuyu.toml"), "schema_version = 1\n\n[project]\nname = \"test\"\n")
        .unwrap();

    let output = {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_asatsuyu-cli"));
        cmd.current_dir(&dir);
        cmd.args(["remove", "nonexistent"]).output().unwrap()
    };
    assert_eq!(output.status.code(), Some(2), "remove nonexistent should exit 2");

    let _ = std::fs::remove_dir_all(&dir);
}

/// `asatsuyu sync` without pylock.toml fails with exit 2.
#[test]
fn sync_requires_pylock() {
    let dir = workspace_root().join("target/test-sync-no-lock");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/main.asty"), "pub fn main() { 42 }").unwrap();
    std::fs::write(
        dir.join("asatsuyu.toml"),
        "schema_version = 1\n\n[project]\nname = \"test\"\n\n[python-dependencies]\nrequests = \">=2.31\"\n",
    )
    .unwrap();

    let output = {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_asatsuyu-cli"));
        cmd.current_dir(&dir);
        cmd.args(["sync"]).output().unwrap()
    };
    assert_eq!(
        output.status.code(),
        Some(2),
        "sync without pylock.toml should exit 2\nstderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("pylock.toml"), "should mention pylock.toml: {stderr}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// `asatsuyu sync --help` works.
#[test]
fn sync_help_shows_subcommand() {
    let output = asatsuyu().args(["sync", "--help"]).output().unwrap();
    assert!(output.status.success(), "sync --help should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("pylock") || stdout.contains("Sync") || stdout.contains("environment"),
        "help should describe sync: {stdout}",
    );
}

/// `asatsuyu sync` honors `[python] path` from asatsuyu.toml.
#[test]
fn sync_uses_python_path_from_config() {
    let dir = workspace_root().join("target/test-sync-python-path-config");
    let _ = std::fs::remove_dir_all(&dir);
    let site_packages = dir.join("fake-site-packages");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(&site_packages).unwrap();
    std::fs::write(dir.join("src/main.asty"), "pub fn main() { 42 }").unwrap();

    let fake_python = dir.join("fake-python");
    std::fs::write(
        &fake_python,
        format!(
            "#!/bin/sh\nif [ \"$1\" = \"-c\" ]; then\n  printf '{}\\n'\n  exit 0\nfi\nexit 1\n",
            site_packages.display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&fake_python).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake_python, perms).unwrap();
    }

    std::fs::write(
        dir.join("asatsuyu.toml"),
        format!(
            "[project]\nname = \"test\"\n\n[python]\npath = \"{}\"\n\n[python-dependencies]\nrequests = \">=2.31\"\n",
            fake_python.display()
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("pylock.toml"),
        "lock-version = \"1.0\"\ncreated-by = \"test\"\n[[packages]]\nname = \"requests\"\nversion = \"2.31.0\"\n",
    )
    .unwrap();

    let output = {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_asatsuyu-cli"));
        cmd.current_dir(&dir);
        cmd.env_clear();
        cmd.args(["sync"]).output().unwrap()
    };
    assert_eq!(
        output.status.code(),
        Some(2),
        "sync without tools should still exit 2\nstderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("no Python environment found"),
        "sync should honor [python] path before PATH discovery:\n{stderr}",
    );
    // After honoring [python] path, sync proceeds past environment discovery.
    // On some CI runners pip is globally installed, so tool discovery may
    // succeed and then pip install fails (because fake-python exits 1 for
    // `-m pip`).  Either outcome proves the config path was used.
    assert!(
        stderr.contains("no sync tool found") || stderr.contains("pip install"),
        "sync should reach tool discovery after resolving configured python:\n{stderr}",
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Full workflow: add → sync → remove (requires uv).
#[test]
#[ignore = "requires uv >= 0.6.15 and network access"]
fn add_remove_sync_full_workflow() {
    let dir = workspace_root().join("target/test-full-workflow");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/main.asty"), "pub fn main() { 42 }").unwrap();
    std::fs::write(
        dir.join("asatsuyu.toml"),
        "schema_version = 1\n\n[project]\nname = \"wftest\"\nversion = \"0.1.0\"\n\n[python]\nversion = \">=3.12\"\n",
    )
    .unwrap();

    // Add dependency.
    let add_out = {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_asatsuyu-cli"));
        cmd.current_dir(&dir);
        cmd.args(["add", "six", ">=1.16"]).output().unwrap()
    };
    assert!(add_out.status.success(), "add failed: {}", String::from_utf8_lossy(&add_out.stderr));
    assert!(dir.join("pylock.toml").exists(), "pylock.toml should exist after add");

    // Create venv and sync.
    let venv_out =
        Command::new("uv").args(["venv", &dir.join(".venv").to_string_lossy()]).output().unwrap();
    assert!(venv_out.status.success(), "venv creation failed");

    let sync_out = {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_asatsuyu-cli"));
        cmd.current_dir(&dir);
        cmd.args(["sync"]).output().unwrap()
    };
    assert!(
        sync_out.status.success(),
        "sync failed: {}",
        String::from_utf8_lossy(&sync_out.stderr)
    );

    // Remove dependency.
    let rm_out = {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_asatsuyu-cli"));
        cmd.current_dir(&dir);
        cmd.args(["remove", "six"]).output().unwrap()
    };
    assert!(rm_out.status.success(), "remove failed: {}", String::from_utf8_lossy(&rm_out.stderr));
    assert!(!dir.join("pylock.toml").exists(), "pylock.toml should be removed when no deps");

    let _ = std::fs::remove_dir_all(&dir);
}
