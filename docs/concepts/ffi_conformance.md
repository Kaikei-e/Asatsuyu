# FFI Conformance Model

## Overview

Asatsuyu uses a three-tier trust model for Python FFI, as defined in the [language charter](principles.md) §4.1:

| Tier | Meaning | CI Gate |
|---|---|---|
| **Verified** | No `Any` leaks. Flows into THIR as normal types | Trust level snapshot + runtime drift test |
| **Checked** | Static types present but `Any` somewhere. Requires runtime wrappers | Wrapper test + trust level assertion |
| **Unsafe** | No usable type info. Opaque value isolation | Not yet covered (MVP scope) |

## Module Coverage

### Verified Modules

| Module | Symbols | Status |
|---|---|---|
| `pathlib` | `Path` class (7 methods, 5 properties, constructor) | Verified |
| `os` | `getenv`, `getcwd`, `environ`, `sep`, `linesep` | Verified |
| `sys` | `argv`, `exit`, `platform`, `version` | Verified |

### Checked Modules

| Module | Symbols | Reason for Checked |
|---|---|---|
| `json` | `loads`, `dumps` | `loads` returns `Any`; `dumps` accepts `Any` |
| `requests` | `get`, `post`, `put`, `delete`, `Response` | `Response.json()` returns `Any` |

## CI Gates

### Layer 1: Rust — FFI Surface Snapshot (`asatsuyu-hir`)

**File**: `crates/asatsuyu-hir/tests/ffi_conformance.rs`

- **Surface snapshot**: `insta` snapshot of `verify_all()` output. Any change to the FFI surface forces explicit review via `cargo insta review`.
- **Trust invariants**: Asserts that Verified modules (pathlib, os, sys) remain Verified and all their symbols are Verified.
- **Symbol count guards**: Prevents silent removal of symbols.
- **Any tracking**: Verifies that `Any` only appears in known Checked symbols.

### Layer 2: Python — Stub/Runtime Drift Detection

**File**: `tests/test_ffi_conformance.py`

Introspects Python 3.12+ runtime at test time and verifies that every symbol Asatsuyu claims to map actually exists. Tests:

- `pathlib.Path` methods and properties exist and are callable
- `os` functions (`getenv`, `getcwd`) and constants (`environ`, `sep`, `linesep`) exist
- `sys` constants and functions exist with correct types
- `json.loads` and `json.dumps` signatures match Asatsuyu's expectations
- `requests` functions and `Response` class members exist (requires `requests` installed)

### Layer 3: E2E — CLI Trust Summary Gate

**File**: `crates/asatsuyu-cli/tests/e2e.rs`

- Asserts exact trust summary: `3 Verified, 2 Checked, 0 Unsafe`
- Asserts each module's trust level in the CLI output

## Known Omissions

Asatsuyu intentionally does not expose the full surface of each module. The following are tracked omissions, not bugs:

### pathlib.Path

`__fspath__`, `resolve`, `absolute`, `unlink`, `rmdir`, `rename`, `replace`, `stat`, `lstat`, `chmod`, `glob`, `rglob`, `iterdir`, `open`, `read_bytes`, `write_bytes`, `touch`, `symlink_to`, `hardlink_to`, `home`, `cwd`, `expanduser`, `match`, `relative_to`, `is_relative_to`, `is_absolute`, `is_symlink`, `with_name`, `with_stem`, `with_suffix`, `as_posix`, `as_uri`, `anchor`, `drive`, `root`, `suffixes`

### os

Most of the `os` module (`listdir`, `remove`, `path.*`, `walk`, etc.) is omitted. MVP covers only `getenv`, `getcwd`, `environ`, `sep`, `linesep`.

### sys

Most of `sys` is omitted. MVP covers `argv`, `exit`, `platform`, `version`.

## Adding a New Module

1. Add the module definition in `asatsuyu-hir/src/ffi/builtins.rs`
2. Register it in `resolver.rs` (`BuiltinResolver::resolve` and `KNOWN_MODULES`)
3. Run `cargo test -p asatsuyu-hir --test ffi_conformance` — snapshot will fail
4. Review and accept: `cargo insta review`
5. Add Python drift tests in `tests/test_ffi_conformance.py`
6. Update this document's coverage tables
7. Run `cargo test --workspace` to verify all gates pass
