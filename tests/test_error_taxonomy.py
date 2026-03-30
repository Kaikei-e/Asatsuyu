"""Test 9-category exception taxonomy consistency across native runtime and prelude.

The Asatsuyu exception taxonomy classifies Python exceptions into 9 categories.
Both the native PyO3 runtime (classify.rs) and the pure-Python prelude (_classify)
must produce identical results for the same exception.

Categories: IoError, ValueError, TypeError, KeyError, AttributeError,
ImportError, ArithmeticError, RuntimeError, Other.
"""

from __future__ import annotations

import pytest

# Parametrize: (exception_class, constructor_args, expected_kind)
TAXONOMY_CASES = [
    # 1. IoError (OSError hierarchy)
    (FileNotFoundError, ("no such file",), "IoError"),
    (PermissionError, ("denied",), "IoError"),
    (OSError, ("generic os",), "IoError"),
    (IsADirectoryError, ("is dir",), "IoError"),
    (ConnectionError, ("conn refused",), "IoError"),
    (ConnectionRefusedError, ("refused",), "IoError"),
    # 2. ValueError
    (ValueError, ("bad value",), "ValueError"),
    # 3. TypeError
    (TypeError, ("bad type",), "TypeError"),
    # 4. KeyError
    (KeyError, ("missing",), "KeyError"),
    # 5. AttributeError
    (AttributeError, ("no attr",), "AttributeError"),
    # 6. ImportError
    (ImportError, ("no module",), "ImportError"),
    (ModuleNotFoundError, ("not found",), "ImportError"),
    # 7. ArithmeticError
    (ZeroDivisionError, ("div by zero",), "ArithmeticError"),
    (OverflowError, ("overflow",), "ArithmeticError"),
    # 8. RuntimeError
    (RuntimeError, ("runtime",), "RuntimeError"),
    (NotImplementedError, ("not impl",), "RuntimeError"),
    # 9. Other
    (StopIteration, ("done",), "Other"),
    (StopAsyncIteration, ("done async",), "Other"),
    (LookupError, ("lookup",), "Other"),
]


def _classify_prelude(e: Exception) -> str:
    """Pure-Python classification matching asatsuyu_prelude.py _classify().

    IMPORTANT: This must be kept in sync with:
    - asatsuyu-backend-python/src/prelude.rs (PRELUDE_PY)
    - asatsuyu-runtime-python/src/classify.rs (classify_exception)
    """
    if isinstance(e, OSError):
        return "IoError"
    if isinstance(e, ValueError):
        return "ValueError"
    if isinstance(e, TypeError):
        return "TypeError"
    if isinstance(e, KeyError):
        return "KeyError"
    if isinstance(e, AttributeError):
        return "AttributeError"
    if isinstance(e, ImportError):
        return "ImportError"
    if isinstance(e, ArithmeticError):
        return "ArithmeticError"
    if isinstance(e, RuntimeError):
        return "RuntimeError"
    return "Other"


@pytest.mark.parametrize(
    "exc_class,args,expected_kind",
    TAXONOMY_CASES,
    ids=[f"{c[0].__name__}->{c[2]}" for c in TAXONOMY_CASES],
)
def test_native_taxonomy(exc_class: type, args: tuple, expected_kind: str) -> None:
    """Verify native PyO3 runtime classifies exceptions correctly."""
    from _asatsuyu_runtime import normalize_exception

    try:
        raise exc_class(*args)
    except Exception as e:
        info = normalize_exception(e)
        assert info["kind"] == expected_kind, (
            f"native: {exc_class.__name__} classified as {info['kind']}, "
            f"expected {expected_kind}"
        )


@pytest.mark.parametrize(
    "exc_class,args,expected_kind",
    TAXONOMY_CASES,
    ids=[f"{c[0].__name__}->{c[2]}" for c in TAXONOMY_CASES],
)
def test_prelude_taxonomy(exc_class: type, args: tuple, expected_kind: str) -> None:
    """Verify pure-Python prelude classifies exceptions identically."""
    try:
        raise exc_class(*args)
    except Exception as e:
        result = _classify_prelude(e)
        assert result == expected_kind, (
            f"prelude: {exc_class.__name__} classified as {result}, "
            f"expected {expected_kind}"
        )


@pytest.mark.parametrize(
    "exc_class,args,expected_kind",
    TAXONOMY_CASES,
    ids=[f"{c[0].__name__}->{c[2]}" for c in TAXONOMY_CASES],
)
def test_native_prelude_consistency(exc_class: type, args: tuple, expected_kind: str) -> None:
    """Verify native and prelude produce identical results."""
    from _asatsuyu_runtime import normalize_exception

    try:
        raise exc_class(*args)
    except Exception as e:
        native_kind = normalize_exception(e)["kind"]
        prelude_kind = _classify_prelude(e)
        assert native_kind == prelude_kind, (
            f"{exc_class.__name__}: native={native_kind}, prelude={prelude_kind}"
        )
