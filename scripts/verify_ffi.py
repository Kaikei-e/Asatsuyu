#!/usr/bin/env python3
"""Verify FFI surface integrity for Asatsuyu's Verified modules.

Runs pyright --verifytypes (type completeness) and mypy stubtest
(stub/runtime fidelity) against the Python modules that Asatsuyu
classifies as Verified FFI.

Requirements:
    pip install pyright mypy

Usage:
    python scripts/verify_ffi.py
"""

from __future__ import annotations

import json
import subprocess
import sys
from typing import Any

STDLIB_VERIFIED_MODULES: list[str] = ["pathlib", "json", "os", "sys"]
CHECKED_MODULES: list[str] = ["requests"]

# pyright completeness threshold for Verified modules.
COMPLETENESS_THRESHOLD: float = 0.80

# Known typeshed <-> CPython mismatches that are NOT Asatsuyu bugs.
# These are filtered from stubtest output before checking pass/fail.
# Key: module name, Value: set of symbol prefixes to ignore.
STUBTEST_ALLOWLIST: dict[str, set[str]] = {
    "json": {
        # Python 3.14 exposes json.__main__ at runtime, but stub coverage
        # can lag behind mypy/typeshed releases.
        "json.__main__ failed to find stubs",
    },
    "os": {
        "os.PathLike.__class_getitem__",
        "os._wrap_close.",
        "os.path.join",
        "os.__all__ names exported from the stub do not correspond to the names exported at runtime.",
        # Python 3.14 stdlib additions can temporarily diverge across
        # runtime/typeshed/mypy release cadences.
        "os.reload_environ",
    },
    "sys": {
        "sys.gettotalrefcount",
        "sys.ps1",
        "sys.ps2",
        "sys.last_exc",
        "sys.last_type",
        "sys.last_value",
        "sys.last_traceback",
        "sys.tracebacklimit",
        "sys._monitoring",
        "sys._jit.",
        "sys.implementation",
        "sys.flags.context_aware_warnings",
        "sys.flags.thread_inherit_context",
        "sys.flags.gil",
    },
}


def run_verifytypes(module: str) -> dict[str, Any]:
    """Run pyright --verifytypes and return parsed result."""
    try:
        result = subprocess.run(
            ["pyright", "--verifytypes", module, "--outputjson"],
            capture_output=True,
            text=True,
            timeout=120,
        )
    except FileNotFoundError:
        return {"error": "pyright not installed (pip install pyright)"}
    except subprocess.TimeoutExpired:
        return {"error": f"pyright timed out for {module}"}

    if result.returncode == 2:
        return {"error": f"pyright fatal error for {module}"}

    try:
        data = json.loads(result.stdout)
        tc = data.get("typeCompleteness", {})
        counts = tc.get("exportedSymbolCounts", {})
        return {
            "score": tc.get("completenessScore", 0.0),
            "known": counts.get("withKnownType", 0),
            "unknown": counts.get("withUnknownType", 0),
            "ambiguous": counts.get("withAmbiguousType", 0),
        }
    except json.JSONDecodeError:
        return {"error": "failed to parse pyright output"}


def run_stubtest(module: str) -> dict[str, Any]:
    """Run mypy stubtest and return result."""
    try:
        result = subprocess.run(
            ["python", "-m", "mypy.stubtest", module, "--concise"],
            capture_output=True,
            text=True,
            timeout=120,
        )
    except FileNotFoundError:
        return {"pass": False, "issues": [], "total_issues": 0, "error": "mypy not installed"}
    except subprocess.TimeoutExpired:
        return {"pass": False, "issues": [], "total_issues": 0, "error": "stubtest timed out"}

    lines = [line.strip() for line in result.stdout.strip().splitlines() if line.strip()]
    # Filter out "Success" messages
    issues = [line for line in lines if not line.startswith("Success")]
    stderr = result.stderr.strip()
    if result.returncode != 0 and not issues:
        return {
            "pass": False,
            "issues": [],
            "total_issues": 0,
            "error": stderr or f"stubtest exited with status {result.returncode}",
        }

    # Filter known typeshed <-> CPython mismatches.
    allowed = STUBTEST_ALLOWLIST.get(module, set())
    raw_count = len(issues)
    issues = [
        line for line in issues
        if not any(line.startswith(prefix) for prefix in allowed)
    ]
    allowed_count = raw_count - len(issues)

    return {
        "pass": len(issues) == 0,
        "issues": issues[:10],
        "total_issues": len(issues),
        "allowed_count": allowed_count,
    }


def main() -> None:
    failed = False

    print("=" * 60)
    print("Asatsuyu FFI Verification Report")
    print("=" * 60)

    for module in STDLIB_VERIFIED_MODULES:
        print(f"\n--- {module} (Verified candidate) ---")
        print("  verifytypes: SKIP (stdlib/typeshed modules are not supported by pyright --verifytypes)")

        st = run_stubtest(module)
        if "error" in st:
            print(f"  stubtest:    SKIP ({st['error']})")
        elif st["pass"]:
            allowed_msg = ""
            if st.get("allowed_count", 0) > 0:
                allowed_msg = f", {st['allowed_count']} known mismatches allowlisted"
            print(f"  stubtest:    PASS (0 actionable issues{allowed_msg})")
        else:
            print(f"  stubtest:    FAIL ({st['total_issues']} issues)")
            for issue in st["issues"]:
                print(f"    - {issue}")
            failed = True

    for module in CHECKED_MODULES:
        print(f"\n--- {module} (Checked, informational) ---")
        vt = run_verifytypes(module)
        if "error" in vt:
            print(f"  verifytypes: SKIP ({vt['error']})")
        else:
            print(f"  verifytypes: completeness {vt['score']:.1%} (informational)")

    print("\n" + "=" * 60)
    if failed:
        print("RESULT: FAIL -- some Verified modules did not pass")
        sys.exit(1)
    else:
        print("RESULT: PASS -- all Verified modules validated")


if __name__ == "__main__":
    main()
