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
