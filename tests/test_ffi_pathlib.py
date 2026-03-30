"""Execution tests for pathlib Verified FFI.

Compiles .asty files using the CLI binary and executes the generated Python
to verify that Verified FFI calls work end-to-end.
"""

from __future__ import annotations

import pathlib
import subprocess

ASATSUYU_DIR = pathlib.Path(__file__).parent.parent / "asatsuyu"


def _run_asty(cli_binary: pathlib.Path, source: str, tmp_path: pathlib.Path) -> subprocess.CompletedProcess:
    """Write an .asty file and run it through the compiler."""
    src_file = tmp_path / "test.asty"
    src_file.write_text(source)
    return subprocess.run(
        [str(cli_binary), "run", str(src_file)],
        capture_output=True,
        text=True,
        cwd=ASATSUYU_DIR,
        timeout=30,
    )


# ── pathlib tests ────────────────────────────────────────────────


def test_pathlib_exists(cli_binary: pathlib.Path, tmp_path: pathlib.Path) -> None:
    """pathlib.Path('.').exists() should compile and run."""
    source = """\
from python import pathlib

pub fn main() -> Bool {
  let p = pathlib.Path(".")
  p.exists()
}
"""
    result = _run_asty(cli_binary, source, tmp_path)
    assert result.returncode == 0, f"stderr: {result.stderr}"


def test_pathlib_name_property(cli_binary: pathlib.Path, tmp_path: pathlib.Path) -> None:
    """pathlib.Path('foo/bar.txt').name should compile and run."""
    source = """\
from python import pathlib

pub fn main() -> String {
  let p = pathlib.Path("foo/bar.txt")
  p.name
}
"""
    result = _run_asty(cli_binary, source, tmp_path)
    assert result.returncode == 0, f"stderr: {result.stderr}"


def test_pathlib_joinpath(cli_binary: pathlib.Path, tmp_path: pathlib.Path) -> None:
    """pathlib.Path('.').joinpath('sub') should compile and run."""
    source = """\
from python import pathlib

pub fn main() -> Bool {
  let p = pathlib.Path(".")
  let q = p.joinpath("sub")
  q.is_dir()
}
"""
    result = _run_asty(cli_binary, source, tmp_path)
    assert result.returncode == 0, f"stderr: {result.stderr}"


def test_pathlib_is_file(cli_binary: pathlib.Path, tmp_path: pathlib.Path) -> None:
    """pathlib.Path.is_file() should compile and run."""
    source = """\
from python import pathlib

pub fn main() -> Bool {
  let p = pathlib.Path(".")
  p.is_file()
}
"""
    result = _run_asty(cli_binary, source, tmp_path)
    assert result.returncode == 0, f"stderr: {result.stderr}"


def test_pathlib_is_dir(cli_binary: pathlib.Path, tmp_path: pathlib.Path) -> None:
    """pathlib.Path.is_dir() should compile and run."""
    source = """\
from python import pathlib

pub fn main() -> Bool {
  let p = pathlib.Path(".")
  p.is_dir()
}
"""
    result = _run_asty(cli_binary, source, tmp_path)
    assert result.returncode == 0, f"stderr: {result.stderr}"
