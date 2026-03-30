#!/usr/bin/env python3
"""Smoke test for the native _asatsuyu_runtime extension.

Verifies that the compiled PyO3 module can be imported and its core
functions work correctly. Run after `maturin develop` or after
installing a wheel built by `maturin build`.

Usage:
    python scripts/smoke_test.py
"""
from __future__ import annotations

import sys


def main() -> None:
    # 1. Import check
    try:
        import _asatsuyu_runtime
    except ImportError:
        print("FAIL: _asatsuyu_runtime not installed")
        print("  Run: cd asatsuyu/crates/asatsuyu-runtime-python && maturin develop")
        sys.exit(1)

    # 2. Capability check
    assert _asatsuyu_runtime.ffi_available(), "ffi_available() should return True"

    # 3. Module import
    mod = _asatsuyu_runtime.import_module("json")
    assert mod is not None, "import_module('json') should succeed"

    # 4. Function call
    result = _asatsuyu_runtime.call_function(mod, "dumps", [1, 2, 3])
    assert result == "[1, 2, 3]", f"call_function(json, dumps, [1,2,3]) = {result!r}"

    # 5. Method call
    import pathlib

    p = pathlib.Path(".")
    exists = _asatsuyu_runtime.call_method(p, "exists")
    assert isinstance(exists, bool), f"call_method(Path, exists) = {exists!r}"

    # 6. Exception normalization
    try:
        raise ValueError("test error")
    except Exception as e:
        info = _asatsuyu_runtime.normalize_exception(e)
        assert info["kind"] == "ValueError", f"wrong kind: {info}"
        assert info["message"] == "test error", f"wrong message: {info}"

    # 7. AsatsuyuError is a RuntimeError subclass
    assert issubclass(
        _asatsuyu_runtime.AsatsuyuError, RuntimeError
    ), "AsatsuyuError should extend RuntimeError"

    print("OK: all smoke tests passed")


if __name__ == "__main__":
    main()
