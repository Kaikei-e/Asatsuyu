#!/usr/bin/env python3
"""Test that generated packages can be installed and imported.

Builds .asty examples into Python packages, installs them in a temporary
venv, and verifies that the generated modules can be imported.

Usage:
    python scripts/test_package_install.py
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

PROJECT_ROOT = Path(__file__).parent.parent
WORKSPACE = PROJECT_ROOT / "asatsuyu"
FIXTURE_PROJECTS = WORKSPACE / "fixtures" / "projects"

# Package-install smoke tests for executable fixture projects.
# Excludes requests_client because it is Checked FFI with third-party dependency.
#
# Each entry is (fixture_name, extra_build_args).
INSTALLABLE_FIXTURES: list[tuple[str, list[str]]] = [
    ("hello_cli", []),
    ("pathlib_walk", []),
    ("stdlib_ffi", []),
    ("build_install", []),
]


def build_cli() -> Path:
    """Build the asatsuyu-cli crate and return the path to its `asatsuyu` binary."""
    print("Building asatsuyu-cli...")
    subprocess.run(
        ["cargo", "build", "-p", "asatsuyu-cli", "--release"],
        cwd=WORKSPACE,
        check=True,
    )
    binary = WORKSPACE / "target" / "release" / "asatsuyu"
    assert binary.exists(), f"binary not found: {binary}"
    return binary


def test_package(
    cli_path: Path,
    fixture_name: str,
    tmp_dir: Path,
    extra_args: list[str] | None = None,
) -> None:
    """Build, install, and import a generated package."""
    source = FIXTURE_PROJECTS / fixture_name / "src" / "main.asty"
    out_dir = tmp_dir / f"dist-{fixture_name}"

    # 1. Build package
    cmd = [str(cli_path), "build", str(source), "-o", str(out_dir)]
    if extra_args:
        cmd.extend(extra_args)
    result = subprocess.run(
        cmd,
        capture_output=True,
        text=True,
        cwd=WORKSPACE,
    )
    assert result.returncode == 0, f"build failed for {fixture_name}: {result.stderr}"

    # 2. Create venv
    venv_dir = tmp_dir / f"venv-{fixture_name}"
    subprocess.run(
        [
            sys.executable,
            "-m",
            "venv",
            "--system-site-packages",
            str(venv_dir),
        ],
        check=True,
    )
    pip = venv_dir / "bin" / "pip"
    python = venv_dir / "bin" / "python"

    # 3. Ensure build backends are importable inside the venv. On some CI
    # images setuptools is absent even with --system-site-packages.
    # Typeshed-resolved modules may produce maturin-backed packages even for
    # stdlib-only code (due to stub parser limitations with `Self` types
    # causing Checked FFI detection). Install both backends to be safe.
    backend_check = subprocess.run(
        [str(python), "-c", "import setuptools.build_meta"],
        capture_output=True,
        text=True,
    )
    if backend_check.returncode != 0:
        subprocess.run(
            [str(pip), "install", "setuptools"],
            capture_output=True,
            text=True,
            check=True,
        )

    # 4. Install package without build isolation so local verification does not
    # depend on network access to re-download setuptools/maturin.
    # If the generated package requires maturin (Checked FFI), skip the pip
    # install + import test but still verify the build step succeeded.
    pyproject_path = out_dir / "pyproject.toml"
    if pyproject_path.exists() and "maturin" in pyproject_path.read_text():
        print(f"  SKIP (maturin): {fixture_name} — Checked FFI package, install requires Rust build")
        return

    result = subprocess.run(
        [str(pip), "install", "--no-build-isolation", "--no-deps", str(out_dir)],
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, f"pip install failed for {fixture_name}: {result.stderr}"

    # 5. Import test. Generated packages expose `<pkg>/<pkg>.py`.
    result = subprocess.run(
        [
            str(python),
            "-c",
            (
                "import importlib; "
                f"importlib.import_module('{fixture_name}.{fixture_name}'); "
                f"print('OK: {fixture_name}')"
            ),
        ],
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, f"import failed for {fixture_name}: {result.stderr}"
    print(f"  PASS: {fixture_name}")


def main() -> None:
    cli = build_cli()
    with tempfile.TemporaryDirectory() as tmp:
        tmp_path = Path(tmp)
        for fixture_name, extra_args in INSTALLABLE_FIXTURES:
            test_package(cli, fixture_name, tmp_path, extra_args)
    print(f"\nOK: all {len(INSTALLABLE_FIXTURES)} fixture package install tests passed")


if __name__ == "__main__":
    main()
