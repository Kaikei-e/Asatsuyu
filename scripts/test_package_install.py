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
EXAMPLES = WORKSPACE / "examples"

# Only Verified-FFI examples (setuptools-based, no native extension needed).
# Note: ffi_json.asty is excluded because json.dumps accepts Any (Checked FFI).
VERIFIED_EXAMPLES = ["hello.asty", "greet.asty", "ffi_pathlib.asty"]


def build_cli() -> Path:
    """Build asatsuyu-cli and return the binary path."""
    print("Building asatsuyu-cli...")
    subprocess.run(
        ["cargo", "build", "-p", "asatsuyu-cli", "--release"],
        cwd=WORKSPACE,
        check=True,
    )
    binary = WORKSPACE / "target" / "release" / "asatsuyu-cli"
    assert binary.exists(), f"binary not found: {binary}"
    return binary


def test_package(cli_path: Path, example_name: str, tmp_dir: Path) -> None:
    """Build, install, and import a generated package."""
    source = EXAMPLES / example_name
    stem = source.stem
    out_dir = tmp_dir / f"dist-{stem}"

    # 1. Build package
    result = subprocess.run(
        [str(cli_path), "build", str(source), "-o", str(out_dir)],
        capture_output=True,
        text=True,
        cwd=WORKSPACE,
    )
    assert result.returncode == 0, f"build failed for {example_name}: {result.stderr}"

    # 2. Create venv
    venv_dir = tmp_dir / f"venv-{stem}"
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

    # 3. Ensure the local backend is importable inside the venv. On some CI
    # images setuptools is absent even with --system-site-packages.
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
    result = subprocess.run(
        [str(pip), "install", "--no-build-isolation", "--no-deps", str(out_dir)],
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, f"pip install failed for {stem}: {result.stderr}"

    # 5. Import test
    result = subprocess.run(
        [str(python), "-c", f"import {stem}; print('OK: {stem}')"],
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, f"import failed for {stem}: {result.stderr}"
    print(f"  PASS: {example_name} ({stem})")


def main() -> None:
    cli = build_cli()
    with tempfile.TemporaryDirectory() as tmp:
        tmp_path = Path(tmp)
        for example in VERIFIED_EXAMPLES:
            test_package(cli, example, tmp_path)
    print(f"\nOK: all {len(VERIFIED_EXAMPLES)} package install tests passed")


if __name__ == "__main__":
    main()
