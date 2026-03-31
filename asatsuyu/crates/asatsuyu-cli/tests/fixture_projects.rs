//! Integration tests for executable fixture projects.
//!
//! Each fixture is a complete Asatsuyu project under `fixtures/projects/`.
//! Tests exercise the full CLI pipeline: check, build, and run.

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

/// Path to a fixture project's main source file.
fn fixture_src(project: &str) -> String {
    format!("fixtures/projects/{project}/src/main.asty")
}

// ── check tests ─────────────────────────────────────────────────────

#[test]
fn check_hello_cli() {
    let output = asatsuyu().args(["check", &fixture_src("hello_cli")]).output().unwrap();
    assert!(
        output.status.success(),
        "check hello_cli failed:\nstderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn check_pathlib_walk() {
    let output = asatsuyu().args(["check", &fixture_src("pathlib_walk")]).output().unwrap();
    assert!(
        output.status.success(),
        "check pathlib_walk failed:\nstderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn check_stdlib_ffi() {
    let output = asatsuyu().args(["check", &fixture_src("stdlib_ffi")]).output().unwrap();
    assert!(
        output.status.success(),
        "check stdlib_ffi failed:\nstderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn check_requests_client() {
    let output = asatsuyu().args(["check", &fixture_src("requests_client")]).output().unwrap();
    assert!(
        output.status.success(),
        "check requests_client failed:\nstderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn check_build_install() {
    let output = asatsuyu().args(["check", &fixture_src("build_install")]).output().unwrap();
    assert!(
        output.status.success(),
        "check build_install failed:\nstderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );
}

// ── build tests ─────────────────────────────────────────────────────

#[test]
fn build_hello_cli() {
    let dir = workspace_root().join("target/test-fixture-build-hello-cli");
    let dir_str = dir.display().to_string();
    let output =
        asatsuyu().args(["build", &fixture_src("hello_cli"), "-o", &dir_str]).output().unwrap();

    assert!(
        output.status.success(),
        "build hello_cli failed:\nstderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );

    // Verify generated Python files.
    let py_path = dir.join("python/hello_cli/hello_cli.py");
    let content = std::fs::read_to_string(&py_path).expect("generated .py should exist");
    assert!(content.contains("def main()"), "main function missing: {content}");
    assert!(content.contains("def greet("), "greet function missing: {content}");
    assert!(content.contains("class Red"), "ADT variant missing: {content}");

    assert!(dir.join("pyproject.toml").exists(), "pyproject.toml should exist");
    assert!(dir.join("python/hello_cli/__init__.py").exists(), "__init__.py should exist");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_pathlib_walk() {
    let dir = workspace_root().join("target/test-fixture-build-pathlib-walk");
    let dir_str = dir.display().to_string();
    let output =
        asatsuyu().args(["build", &fixture_src("pathlib_walk"), "-o", &dir_str]).output().unwrap();

    assert!(
        output.status.success(),
        "build pathlib_walk failed:\nstderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );

    let py_path = dir.join("python/pathlib_walk/pathlib_walk.py");
    let content = std::fs::read_to_string(&py_path).expect("generated .py should exist");
    assert!(content.contains("import pathlib"), "pathlib import missing: {content}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_stdlib_ffi() {
    let dir = workspace_root().join("target/test-fixture-build-stdlib-ffi");
    let dir_str = dir.display().to_string();
    let output =
        asatsuyu().args(["build", &fixture_src("stdlib_ffi"), "-o", &dir_str]).output().unwrap();

    assert!(
        output.status.success(),
        "build stdlib_ffi failed:\nstderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );

    let py_path = dir.join("python/stdlib_ffi/stdlib_ffi.py");
    let content = std::fs::read_to_string(&py_path).expect("generated .py should exist");
    assert!(content.contains("import os"), "os import missing: {content}");
    assert!(content.contains("import sys"), "sys import missing: {content}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_requests_client() {
    let dir = workspace_root().join("target/test-fixture-build-requests-client");
    let dir_str = dir.display().to_string();
    let output = asatsuyu()
        .args(["build", &fixture_src("requests_client"), "-o", &dir_str])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "build requests_client failed:\nstderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );

    let py_path = dir.join("python/requests_client/requests_client.py");
    let content = std::fs::read_to_string(&py_path).expect("generated .py should exist");
    assert!(content.contains("import requests"), "requests import missing: {content}");

    // Verify pyproject.toml includes requests dependency.
    let pyproject =
        std::fs::read_to_string(dir.join("pyproject.toml")).expect("pyproject.toml should exist");
    assert!(pyproject.contains("requests"), "requests dependency missing: {pyproject}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_build_install() {
    let dir = workspace_root().join("target/test-fixture-build-build-install");
    let dir_str = dir.display().to_string();
    let output =
        asatsuyu().args(["build", &fixture_src("build_install"), "-o", &dir_str]).output().unwrap();

    assert!(
        output.status.success(),
        "build build_install failed:\nstderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );

    // Verify package metadata.
    let pyproject =
        std::fs::read_to_string(dir.join("pyproject.toml")).expect("pyproject.toml should exist");
    assert!(pyproject.contains("name = \"build_install\""), "project name missing: {pyproject}",);
    assert!(pyproject.contains("version = \"1.0.0\""), "project version missing: {pyproject}",);

    let _ = std::fs::remove_dir_all(&dir);
}

// ── run tests ───────────────────────────────────────────────────────

#[test]
fn run_hello_cli() {
    let output = asatsuyu().args(["run", &fixture_src("hello_cli")]).output().unwrap();
    assert!(
        output.status.success(),
        "run hello_cli failed:\nstderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn run_pathlib_walk() {
    let output = asatsuyu().args(["run", &fixture_src("pathlib_walk")]).output().unwrap();
    assert!(
        output.status.success(),
        "run pathlib_walk failed:\nstderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn run_stdlib_ffi() {
    let output = asatsuyu().args(["run", &fixture_src("stdlib_ffi")]).output().unwrap();
    assert!(
        output.status.success(),
        "run stdlib_ffi failed:\nstderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn run_build_install() {
    let output = asatsuyu().args(["run", &fixture_src("build_install")]).output().unwrap();
    assert!(
        output.status.success(),
        "run build_install failed:\nstderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
#[ignore = "requires network access and requests installed"]
fn run_requests_client() {
    let output = asatsuyu().args(["run", &fixture_src("requests_client")]).output().unwrap();
    assert!(
        output.status.success(),
        "run requests_client failed:\nstderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );
}

// ── new → populate → build integration ──────────────────────────────

#[test]
fn new_then_populate_then_build() {
    let dir = workspace_root().join("target/test-fixture-new-populate-build");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // Create a new project.
    let output = asatsuyu().current_dir(&dir).args(["new", "myapp"]).output().unwrap();
    assert!(output.status.success(), "new failed: {}", String::from_utf8_lossy(&output.stderr));

    // Populate with a non-trivial program (hello_cli style).
    let main_content = r#"type Shape {
  Circle(radius: Float)
  Rect(w: Float, h: Float)
}

pub fn describe(s: Shape) -> String {
  match s {
    Circle(_) -> "circle"
    Rect(_, _) -> "rectangle"
  }
}

pub fn main() -> String {
  describe(Circle(3.14))
}
"#;
    std::fs::write(dir.join("myapp/src/main.asty"), main_content).unwrap();

    // Build the populated project.
    let main_path = dir.join("myapp/src/main.asty");
    let main_path_str = main_path.display().to_string();
    let out_dir = dir.join("myapp/dist");
    let out_dir_str = out_dir.display().to_string();
    let output = asatsuyu().args(["build", &main_path_str, "-o", &out_dir_str]).output().unwrap();

    assert!(
        output.status.success(),
        "build after new failed:\nstderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );

    // Verify output.
    assert!(out_dir.join("pyproject.toml").exists(), "pyproject.toml should exist");
    let py_path = out_dir.join("python/myapp/myapp.py");
    let content = std::fs::read_to_string(&py_path).expect("generated .py should exist");
    assert!(content.contains("class Circle"), "Circle ADT missing: {content}");
    assert!(content.contains("def describe("), "describe function missing: {content}");

    // Run it too.
    let output = asatsuyu().args(["run", &main_path_str]).output().unwrap();
    assert!(
        output.status.success(),
        "run after new failed:\nstderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ── build → pip install ─────────────────────────────────────────────

#[test]
#[ignore = "requires venv and pip"]
fn build_install_and_import() {
    let dir = workspace_root().join("target/test-fixture-install");
    let _ = std::fs::remove_dir_all(&dir);

    let out_dir = dir.join("dist");
    let out_dir_str = out_dir.display().to_string();
    let output = asatsuyu()
        .args(["build", &fixture_src("build_install"), "-o", &out_dir_str])
        .output()
        .unwrap();
    assert!(output.status.success());

    // Create venv and install.
    let venv_dir = dir.join("venv");
    let status = Command::new("python3")
        .args(["-m", "venv", &venv_dir.display().to_string()])
        .status()
        .unwrap();
    assert!(status.success(), "venv creation failed");

    let pip = venv_dir.join("bin/pip");
    let status = Command::new(&pip).args(["install", &out_dir_str]).status().unwrap();
    assert!(status.success(), "pip install failed");

    // Verify import works.
    let python = venv_dir.join("bin/python");
    let output = Command::new(&python)
        .args(["-c", "from build_install.build_install import greeting; print(greeting())"])
        .output()
        .unwrap();
    assert!(output.status.success(), "import failed: {}", String::from_utf8_lossy(&output.stderr));

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Hello from build_install!"), "unexpected output: {stdout}");

    let _ = std::fs::remove_dir_all(&dir);
}
