"""Integration tests for the _asatsuyu_runtime extension module."""

import pytest


def test_import():
    import _asatsuyu_runtime

    assert hasattr(_asatsuyu_runtime, "ffi_available")
    assert hasattr(_asatsuyu_runtime, "import_module")
    assert hasattr(_asatsuyu_runtime, "call_function")
    assert hasattr(_asatsuyu_runtime, "call_method")
    assert hasattr(_asatsuyu_runtime, "normalize_exception")
    assert hasattr(_asatsuyu_runtime, "AsatsuyuError")


def test_ffi_available():
    from _asatsuyu_runtime import ffi_available

    assert ffi_available() is True


def test_import_module():
    from _asatsuyu_runtime import import_module

    json_mod = import_module("json")
    assert hasattr(json_mod, "dumps")


def test_import_module_failure():
    from _asatsuyu_runtime import import_module

    with pytest.raises(ModuleNotFoundError):
        import_module("nonexistent_module_xyz")


def test_call_function():
    from _asatsuyu_runtime import call_function, import_module

    json_mod = import_module("json")
    result = call_function(json_mod, "dumps", [1, 2, 3])
    assert result == "[1, 2, 3]"


def test_call_method():
    from _asatsuyu_runtime import call_method, import_module

    pathlib = import_module("pathlib")
    p = pathlib.Path(".")
    result = call_method(p, "exists")
    assert result is True


def test_normalize_exception_value_error():
    from _asatsuyu_runtime import normalize_exception

    try:
        raise ValueError("test error")
    except Exception as e:
        result = normalize_exception(e)

    assert result["kind"] == "ValueError"
    assert result["exception_type"] == "ValueError"
    assert result["message"] == "test error"
    assert result["module"] == "builtins"
    assert "ValueError: test error" in result["traceback_summary"]


def test_normalize_exception_io_error():
    from _asatsuyu_runtime import normalize_exception

    try:
        raise FileNotFoundError("no such file")
    except Exception as e:
        result = normalize_exception(e)

    # FileNotFoundError is a subclass of OSError -> classified as IoError
    assert result["kind"] == "IoError"
    assert result["exception_type"] == "FileNotFoundError"


def test_normalize_exception_type_error():
    from _asatsuyu_runtime import normalize_exception

    try:
        raise TypeError("bad type")
    except Exception as e:
        result = normalize_exception(e)

    assert result["kind"] == "TypeError"


def test_normalize_exception_key_error():
    from _asatsuyu_runtime import normalize_exception

    try:
        raise KeyError("missing")
    except Exception as e:
        result = normalize_exception(e)

    assert result["kind"] == "KeyError"


def test_normalize_exception_other():
    from _asatsuyu_runtime import normalize_exception

    try:
        raise StopIteration("done")
    except Exception as e:
        result = normalize_exception(e)

    assert result["kind"] == "Other"


def test_asatsuyu_error():
    from _asatsuyu_runtime import AsatsuyuError

    assert issubclass(AsatsuyuError, RuntimeError)

    with pytest.raises(AsatsuyuError):
        raise AsatsuyuError("test")


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
