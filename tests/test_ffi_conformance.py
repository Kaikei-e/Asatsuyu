"""FFI stub/runtime drift detection tests.

Verifies that Asatsuyu's hand-crafted FFI builtin definitions (builtins.rs)
match the actual Python 3.12+ runtime. Catches drift when Python adds,
renames, or removes symbols that Asatsuyu maps.

Direction: tests only verify "Asatsuyu claims X exists" → "X actually exists
in Python". Python symbols not mapped by Asatsuyu are intentionally omitted
(tracked in KNOWN_OMISSIONS per module).
"""

from __future__ import annotations

import inspect
import json
import os
import pathlib
import sys

import pytest


# ── pathlib.Path ─────────────────────────────────────────────────────

# Methods Asatsuyu maps for pathlib.Path (from builtins.rs)
PATHLIB_PATH_METHODS = [
    "read_text",
    "write_text",
    "exists",
    "is_file",
    "is_dir",
    "joinpath",
    "mkdir",
]

# Properties Asatsuyu maps for pathlib.Path
PATHLIB_PATH_PROPERTIES = [
    "name",
    "stem",
    "suffix",
    "parent",
    "parts",
]

# pathlib.Path members Python has but Asatsuyu intentionally omits (MVP).
PATHLIB_KNOWN_OMISSIONS = [
    "__fspath__",
    "resolve",
    "absolute",
    "unlink",
    "rmdir",
    "rename",
    "replace",
    "stat",
    "lstat",
    "chmod",
    "glob",
    "rglob",
    "iterdir",
    "open",
    "read_bytes",
    "write_bytes",
    "touch",
    "symlink_to",
    "hardlink_to",
    "link_to",
    "home",
    "cwd",
    "expanduser",
    "match",
    "relative_to",
    "is_relative_to",
    "is_absolute",
    "is_symlink",
    "is_mount",
    "is_block_device",
    "is_char_device",
    "is_fifo",
    "is_socket",
    "samefile",
    "with_name",
    "with_stem",
    "with_suffix",
    "as_posix",
    "as_uri",
    "anchor",
    "drive",
    "root",
    "suffixes",
]


@pytest.mark.parametrize("method", PATHLIB_PATH_METHODS)
def test_pathlib_path_method_exists(method: str) -> None:
    """Verify that each method Asatsuyu maps on pathlib.Path actually exists."""
    assert hasattr(pathlib.Path, method), f"pathlib.Path.{method} does not exist in Python runtime"
    attr = getattr(pathlib.Path, method)
    assert callable(attr), f"pathlib.Path.{method} is not callable"


@pytest.mark.parametrize("prop", PATHLIB_PATH_PROPERTIES)
def test_pathlib_path_property_exists(prop: str) -> None:
    """Verify that each property Asatsuyu maps on pathlib.Path actually exists."""
    assert hasattr(pathlib.Path, prop), f"pathlib.Path.{prop} does not exist in Python runtime"


def test_pathlib_path_constructor() -> None:
    """Verify pathlib.Path can be constructed with a string argument."""
    p = pathlib.Path(".")
    assert isinstance(p, pathlib.Path)


# ── os ───────────────────────────────────────────────────────────────

# Functions Asatsuyu maps for os
OS_FUNCTIONS = ["getenv", "getcwd"]

# Constants Asatsuyu maps for os
OS_CONSTANTS = ["environ", "sep", "linesep"]


@pytest.mark.parametrize("func", OS_FUNCTIONS)
def test_os_function_exists(func: str) -> None:
    """Verify each os function Asatsuyu maps actually exists."""
    assert hasattr(os, func), f"os.{func} does not exist"
    assert callable(getattr(os, func)), f"os.{func} is not callable"


@pytest.mark.parametrize("const", OS_CONSTANTS)
def test_os_constant_exists(const: str) -> None:
    """Verify each os constant Asatsuyu maps actually exists."""
    assert hasattr(os, const), f"os.{const} does not exist"


def test_os_getenv_signature() -> None:
    """Verify os.getenv accepts (key, default=None) as Asatsuyu maps it."""
    sig = inspect.signature(os.getenv)
    params = list(sig.parameters.keys())
    assert "key" in params, f"os.getenv params: {params}"
    assert "default" in params, f"os.getenv params: {params}"


def test_os_getcwd_returns_str() -> None:
    """Verify os.getcwd() returns a string."""
    result = os.getcwd()
    assert isinstance(result, str)


# ── sys ──────────────────────────────────────────────────────────────

SYS_CONSTANTS = ["argv", "platform", "version"]
SYS_FUNCTIONS = ["exit"]


@pytest.mark.parametrize("const", SYS_CONSTANTS)
def test_sys_constant_exists(const: str) -> None:
    """Verify each sys constant Asatsuyu maps actually exists."""
    assert hasattr(sys, const), f"sys.{const} does not exist"


@pytest.mark.parametrize("func", SYS_FUNCTIONS)
def test_sys_function_exists(func: str) -> None:
    """Verify each sys function Asatsuyu maps actually exists."""
    assert hasattr(sys, func), f"sys.{func} does not exist"
    assert callable(getattr(sys, func)), f"sys.{func} is not callable"


def test_sys_argv_is_list() -> None:
    """Verify sys.argv is a list of strings."""
    assert isinstance(sys.argv, list)


def test_sys_platform_is_str() -> None:
    """Verify sys.platform is a string."""
    assert isinstance(sys.platform, str)


# ── json ─────────────────────────────────────────────────────────────

JSON_FUNCTIONS = ["loads", "dumps"]


@pytest.mark.parametrize("func", JSON_FUNCTIONS)
def test_json_function_exists(func: str) -> None:
    """Verify each json function Asatsuyu maps actually exists."""
    assert hasattr(json, func), f"json.{func} does not exist"
    assert callable(getattr(json, func)), f"json.{func} is not callable"


def test_json_loads_signature() -> None:
    """Verify json.loads accepts a string argument."""
    sig = inspect.signature(json.loads)
    params = list(sig.parameters.keys())
    assert "s" in params, f"json.loads params: {params}"


def test_json_dumps_signature() -> None:
    """Verify json.dumps accepts obj and indent arguments."""
    sig = inspect.signature(json.dumps)
    params = list(sig.parameters.keys())
    assert "obj" in params, f"json.dumps params: {params}"
    assert "indent" in params, f"json.dumps params: {params}"


# ── requests (optional) ─────────────────────────────────────────────

REQUESTS_FUNCTIONS = ["get", "post", "put", "delete"]

REQUESTS_RESPONSE_METHODS = ["json", "raise_for_status"]

REQUESTS_RESPONSE_PROPERTIES = [
    "text",
    "status_code",
    "ok",
    "url",
    "content",
    "encoding",
]


@pytest.mark.parametrize("func", REQUESTS_FUNCTIONS)
def test_requests_function_exists(func: str) -> None:
    """Verify each requests function Asatsuyu maps actually exists."""
    requests = pytest.importorskip("requests")
    assert hasattr(requests, func), f"requests.{func} does not exist"
    assert callable(getattr(requests, func)), f"requests.{func} is not callable"


@pytest.mark.parametrize("method", REQUESTS_RESPONSE_METHODS)
def test_requests_response_method_exists(method: str) -> None:
    """Verify each Response method Asatsuyu maps actually exists."""
    requests = pytest.importorskip("requests")
    assert hasattr(requests.Response, method), f"Response.{method} does not exist"


@pytest.mark.parametrize("prop", REQUESTS_RESPONSE_PROPERTIES)
def test_requests_response_property_exists(prop: str) -> None:
    """Verify each Response property Asatsuyu maps actually exists."""
    requests = pytest.importorskip("requests")
    # Create a minimal Response instance. Some attributes (status_code, url,
    # encoding) are set in __init__, not as class descriptors. Using an
    # instance with status_code=200 ensures `ok` property doesn't raise.
    resp = requests.Response()
    resp.status_code = 200
    assert hasattr(resp, prop), f"Response.{prop} does not exist"
