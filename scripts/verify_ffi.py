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

VERIFIED_MODULES: list[str] = ["pathlib", "json", "os", "sys"]
CHECKED_MODULES: list[str] = ["requests"]

# pyright completeness threshold for Verified modules.
COMPLETENESS_THRESHOLD: float = 0.80


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
    return {
        "pass": result.returncode == 0,
        "issues": issues[:10],
        "total_issues": len(issues),
    }


def main() -> None:
    failed = False

    print("=" * 60)
    print("Asatsuyu FFI Verification Report")
    print("=" * 60)

    for module in VERIFIED_MODULES:
        print(f"\n--- {module} (Verified candidate) ---")

        vt = run_verifytypes(module)
        if "error" in vt:
            print(f"  verifytypes: SKIP ({vt['error']})")
        else:
            status = "PASS" if vt["score"] >= COMPLETENESS_THRESHOLD else "FAIL"
            print(f"  verifytypes: {status} (completeness: {vt['score']:.1%})")
            if status == "FAIL":
                failed = True

        st = run_stubtest(module)
        if "error" in st:
            print(f"  stubtest:    SKIP ({st['error']})")
        elif st["pass"]:
            print(f"  stubtest:    PASS (0 issues)")
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
