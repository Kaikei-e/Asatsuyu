#!/usr/bin/env python3
"""Docs sync check — verifies documentation matches implementation state.

Checks:
  1. DiagnosticCode range consistency (doc comment vs enum variants)
  2. FFI module Verified/Checked classification (docs vs test assertions)
  3. Fixture project README completeness (directories vs table rows)
  4. Snapshot count floor (prevents accidental mass deletion)
  5. CLI FFI flags documented in FFI guide

Exit 0 if all checks pass, exit 1 if any fail.
"""

from __future__ import annotations

import os
import re
import sys
from pathlib import Path

# ── Paths (relative to repo root) ────────────────────────────────────

REPO_ROOT = Path(__file__).resolve().parent.parent
CARGO_DIR = REPO_ROOT / "asatsuyu"
DIAGNOSTIC_RS = (
    CARGO_DIR / "crates" / "asatsuyu-syntax" / "src" / "diagnostic.rs"
)
FFI_CONFORMANCE_RS = (
    CARGO_DIR / "crates" / "asatsuyu-hir" / "tests" / "ffi_conformance.rs"
)
FFI_CONFORMANCE_MD = REPO_ROOT / "docs" / "concepts" / "ffi_conformance.md"
FFI_GUIDE_MD = REPO_ROOT / "docs" / "guides" / "ffi.md"
FIXTURES_DIR = CARGO_DIR / "fixtures" / "projects"
FIXTURES_README = FIXTURES_DIR / "README.md"

SNAPSHOT_FLOOR = 350  # current count ~361; catches mass deletion

FFI_FLAGS = [
    "--ffi-stdlib-only",
    "--ffi-runtime",
    "--no-emit-package",
    "--ffi-stub-path",
]

# ── Utilities ─────────────────────────────────────────────────────────

passed = 0
failed = 0


def check(name: str, ok: bool, detail: str = "") -> None:
    global passed, failed
    status = "PASS" if ok else "FAIL"
    suffix = f"  ({detail})" if detail else ""
    print(f"  [{status}] {name}{suffix}")
    if ok:
        passed += 1
    else:
        failed += 1


# ── Check 1: DiagnosticCode range consistency ─────────────────────────


def check_diagnostic_codes() -> None:
    """Verify every DiagnosticCode variant falls within a documented range."""
    print("\n== Check 1: DiagnosticCode ranges ==")
    src = DIAGNOSTIC_RS.read_text()

    # Parse documented ranges from doc comment.
    # Pattern: "- E0001–E0049: ..." or "- E0001-E0049: ..."
    range_pat = re.compile(r"E(\d{4})[–\-]E(\d{4}):\s*(.+)")
    ranges: list[tuple[int, int, bool]] = []  # (lo, hi, reserved)
    for m in range_pat.finditer(src):
        lo, hi = int(m.group(1)), int(m.group(2))
        reserved = "reserved" in m.group(3).lower()
        ranges.append((lo, hi, reserved))

    check("documented ranges found", len(ranges) > 0, f"{len(ranges)} ranges")

    # Parse enum variants: E0001 = 1, E0050 = 50, ...
    variant_pat = re.compile(r"E(\d{4})\s*=\s*(\d+)")
    codes: list[int] = []
    for m in variant_pat.finditer(src):
        name_num = int(m.group(1))
        disc_num = int(m.group(2))
        # Sanity: the name number should match the discriminant
        check_ok = name_num == disc_num
        if not check_ok:
            check(
                f"E{name_num:04d} name matches discriminant",
                False,
                f"name=E{name_num:04d}, disc={disc_num}",
            )
        codes.append(name_num)

    check("enum variants found", len(codes) > 0, f"{len(codes)} codes")

    # Check that every code falls within at least one documented range.
    out_of_range: list[int] = []
    for code in codes:
        in_range = any(lo <= code <= hi for lo, hi, _reserved in ranges)
        if not in_range:
            out_of_range.append(code)

    check(
        "all codes within documented ranges",
        len(out_of_range) == 0,
        f"out-of-range: {['E' + str(c).zfill(4) for c in out_of_range]}"
        if out_of_range
        else "",
    )

    # Check that no non-reserved documented range is empty.
    empty_ranges: list[str] = []
    for lo, hi, reserved in ranges:
        if reserved:
            continue
        has_code = any(lo <= c <= hi for c in codes)
        if not has_code:
            empty_ranges.append(f"E{lo:04d}–E{hi:04d}")

    check(
        "no empty documented ranges",
        len(empty_ranges) == 0,
        f"empty: {empty_ranges}" if empty_ranges else "",
    )


# ── Check 2: FFI module classification sync ───────────────────────────


def check_ffi_classification() -> None:
    """Verify Verified/Checked module lists match between docs and tests."""
    print("\n== Check 2: FFI module classification ==")

    # Extract from ffi_conformance.rs
    rs_src = FFI_CONFORMANCE_RS.read_text()

    # Verified: the array in verified_modules_stay_verified
    verified_rs_match = re.search(
        r'for\s+name\s+in\s+&\[([^\]]+)\]', rs_src
    )
    verified_rs: set[str] = set()
    if verified_rs_match:
        verified_rs = set(re.findall(r'"(\w+)"', verified_rs_match.group(1)))

    # Checked: json_is_checked and requests_is_checked test functions
    checked_rs: set[str] = set()
    for m in re.finditer(r'fn\s+(\w+)_is_checked\b', rs_src):
        checked_rs.add(m.group(1))

    check(
        "test file has Verified modules",
        len(verified_rs) > 0,
        f"{sorted(verified_rs)}",
    )
    check(
        "test file has Checked modules",
        len(checked_rs) > 0,
        f"{sorted(checked_rs)}",
    )

    # Extract from ffi_conformance.md
    md_src = FFI_CONFORMANCE_MD.read_text()

    # Find Verified modules section table rows: | `pathlib` | ...  | Verified |
    verified_md: set[str] = set()
    checked_md: set[str] = set()

    in_verified = False
    in_checked = False
    for line in md_src.splitlines():
        if "### Verified Modules" in line:
            in_verified = True
            in_checked = False
            continue
        if "### Checked Modules" in line:
            in_checked = True
            in_verified = False
            continue
        if line.startswith("##") and "Modules" not in line:
            in_verified = False
            in_checked = False
            continue

        # Parse table row: | `module` | ... |
        row_match = re.match(r'\|\s*`(\w+)`\s*\|', line)
        if row_match:
            mod_name = row_match.group(1)
            if in_verified:
                verified_md.add(mod_name)
            elif in_checked:
                checked_md.add(mod_name)

    check(
        "Verified modules match (docs vs tests)",
        verified_md == verified_rs,
        f"docs={sorted(verified_md)}, tests={sorted(verified_rs)}"
        if verified_md != verified_rs
        else f"{sorted(verified_rs)}",
    )
    check(
        "Checked modules match (docs vs tests)",
        checked_md == checked_rs,
        f"docs={sorted(checked_md)}, tests={sorted(checked_rs)}"
        if checked_md != checked_rs
        else f"{sorted(checked_rs)}",
    )


# ── Check 3: Fixture project README sync ──────────────────────────────


def check_fixture_readme() -> None:
    """Verify fixture project directories match README table rows."""
    print("\n== Check 3: Fixture project README ==")

    # Get actual directories
    dirs = sorted(
        d.name
        for d in FIXTURES_DIR.iterdir()
        if d.is_dir() and not d.name.startswith(".")
    )
    check("fixture directories found", len(dirs) > 0, f"{len(dirs)} dirs")

    # Get README table rows: | `hello_cli` | ... |
    readme_src = FIXTURES_README.read_text()
    readme_fixtures: list[str] = []
    for m in re.finditer(r'\|\s*`(\w+)`\s*\|', readme_src):
        readme_fixtures.append(m.group(1))

    readme_set = set(readme_fixtures)
    dirs_set = set(dirs)

    missing_from_readme = dirs_set - readme_set
    extra_in_readme = readme_set - dirs_set

    check(
        "all directories listed in README",
        len(missing_from_readme) == 0,
        f"missing: {sorted(missing_from_readme)}" if missing_from_readme else "",
    )
    check(
        "no stale entries in README",
        len(extra_in_readme) == 0,
        f"stale: {sorted(extra_in_readme)}" if extra_in_readme else "",
    )


# ── Check 4: Snapshot count floor ─────────────────────────────────────


def check_snapshot_floor() -> None:
    """Verify snapshot file count hasn't dropped below the floor."""
    print("\n== Check 4: Snapshot count floor ==")

    count = 0
    for root, _dirs, files in os.walk(CARGO_DIR):
        for f in files:
            if f.endswith(".snap"):
                count += 1

    check(
        f"snapshot count >= {SNAPSHOT_FLOOR}",
        count >= SNAPSHOT_FLOOR,
        f"found {count}",
    )


# ── Check 5: CLI FFI flags in guide ──────────────────────────────────


def check_ffi_flags_documented() -> None:
    """Verify all FFI CLI flags are mentioned in the FFI guide."""
    print("\n== Check 5: CLI FFI flags in guide ==")

    guide_src = FFI_GUIDE_MD.read_text()

    for flag in FFI_FLAGS:
        check(
            f"'{flag}' in FFI guide",
            flag in guide_src,
        )


# ── Main ──────────────────────────────────────────────────────────────


def main() -> int:
    print("Docs sync check")
    print(f"  repo root: {REPO_ROOT}")

    check_diagnostic_codes()
    check_ffi_classification()
    check_fixture_readme()
    check_snapshot_floor()
    check_ffi_flags_documented()

    print(f"\n{'=' * 40}")
    print(f"  {passed} passed, {failed} failed")

    if failed > 0:
        print("  DOCS SYNC: FAIL")
        return 1

    print("  DOCS SYNC: PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
