"""Shared pytest fixtures for Asatsuyu Python tests."""

from __future__ import annotations

import pathlib
import subprocess
import sys

import pytest

PROJECT_ROOT = pathlib.Path(__file__).parent.parent
ASATSUYU_DIR = PROJECT_ROOT / "asatsuyu"


@pytest.fixture(scope="session")
def cli_binary() -> pathlib.Path:
    """Build the asatsuyu-cli crate and return the path to its `asatsuyu` binary."""
    result = subprocess.run(
        ["cargo", "build", "-p", "asatsuyu-cli", "--release"],
        cwd=ASATSUYU_DIR,
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, f"cargo build failed: {result.stderr}"
    binary = ASATSUYU_DIR / "target" / "release" / "asatsuyu"
    assert binary.exists(), f"binary not found: {binary}"
    return binary


@pytest.fixture()
def tmp_venv(tmp_path: pathlib.Path) -> pathlib.Path:
    """Create a temporary virtualenv and return its path."""
    venv_dir = tmp_path / "venv"
    subprocess.run(
        [sys.executable, "-m", "venv", str(venv_dir)],
        check=True,
    )
    return venv_dir


REQUESTS_STUB_SOURCE = """\
class Response:
    def __init__(self, status_code: int, text: str):
        self.status_code = status_code
        self.text = text
        self.ok = status_code < 400
        self.url = ""
        self.content = text.encode()
        self.encoding = "utf-8"

    def json(self):
        return {"ok": True}

    def raise_for_status(self):
        if self.status_code >= 400:
            raise Exception(f"HTTP {self.status_code}")


def get(url: str) -> Response:
    return Response(200, f"stub:{url}")


def post(url: str) -> Response:
    return Response(201, f"stub:{url}")


def put(url: str) -> Response:
    return Response(200, f"stub:{url}")


def delete(url: str) -> Response:
    return Response(204, f"stub:{url}")
"""


@pytest.fixture()
def requests_stub_module(
    tmp_path: pathlib.Path, monkeypatch: pytest.MonkeyPatch
) -> pathlib.Path:
    """Write a stub requests.py into tmp_path and return the file path.

    Prepending to `sys.path` is not enough on its own: an already-imported
    `requests` is served from `sys.modules`, so on a machine that has the real
    library these tests would reach the network instead of the stub.
    """
    stub = tmp_path / "requests.py"
    stub.write_text(REQUESTS_STUB_SOURCE)
    monkeypatch.delitem(sys.modules, "requests", raising=False)
    return stub
