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
    assert!(stdout.contains("hello.py"), "stdout should contain output path: {stdout}");

    // Verify the file was created and contains valid Python.
    let py_path = dir.join("hello.py");
    let content = std::fs::read_to_string(&py_path).expect("generated .py should exist");
    assert!(content.contains("def main()"), "generated Python: {content}");
    assert!(content.contains("return 42"), "generated Python: {content}");

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

    let content =
        std::fs::read_to_string(dir.join("greet.py")).expect("generated .py should exist");
    assert!(content.contains("def greet(name: str) -> str:"), "content: {content}");
    assert!(content.contains("def add(x: int, y: int) -> int:"), "content: {content}");

    let _ = std::fs::remove_dir_all(&dir);
}
