"""Runtime tests for requests Checked FFI using stub module.

Tests verify that _asatsuyu_runtime correctly dispatches calls to Python
modules. Uses a stub requests module (no real HTTP calls).
"""

from __future__ import annotations

import pathlib

import pytest


def test_import_stub_requests(
    requests_stub_module: pathlib.Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Verify _asatsuyu_runtime can import a stub requests module."""
    import _asatsuyu_runtime

    monkeypatch.syspath_prepend(str(requests_stub_module.parent))
    mod = _asatsuyu_runtime.import_module("requests")
    assert hasattr(mod, "get")
    assert hasattr(mod, "Response")


def test_call_get(
    requests_stub_module: pathlib.Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Verify call_function routes requests.get through runtime."""
    import _asatsuyu_runtime

    monkeypatch.syspath_prepend(str(requests_stub_module.parent))
    mod = _asatsuyu_runtime.import_module("requests")
    response = _asatsuyu_runtime.call_function(mod, "get", "https://example.test")
    assert hasattr(response, "status_code")
    assert response.status_code == 200
    assert "stub:" in response.text


def test_call_method_json(
    requests_stub_module: pathlib.Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Verify call_method routes response.json() through runtime."""
    import _asatsuyu_runtime

    monkeypatch.syspath_prepend(str(requests_stub_module.parent))
    mod = _asatsuyu_runtime.import_module("requests")
    response = _asatsuyu_runtime.call_function(mod, "get", "https://example.test")
    data = _asatsuyu_runtime.call_method(response, "json")
    assert isinstance(data, dict)
    assert data["ok"] is True


def test_call_method_raise_for_status(
    requests_stub_module: pathlib.Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Verify call_method can call raise_for_status on a 200 response."""
    import _asatsuyu_runtime

    monkeypatch.syspath_prepend(str(requests_stub_module.parent))
    mod = _asatsuyu_runtime.import_module("requests")
    response = _asatsuyu_runtime.call_function(mod, "get", "https://example.test")
    # Should not raise for 200
    _asatsuyu_runtime.call_method(response, "raise_for_status")


def test_call_post_put_delete(
    requests_stub_module: pathlib.Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Verify all HTTP methods work through runtime."""
    import _asatsuyu_runtime

    monkeypatch.syspath_prepend(str(requests_stub_module.parent))
    mod = _asatsuyu_runtime.import_module("requests")
    for method in ["post", "put", "delete"]:
        response = _asatsuyu_runtime.call_function(mod, method, "https://example.test")
        assert hasattr(response, "status_code"), f"{method} should return Response"
        assert hasattr(response, "text"), f"{method} should return Response"
