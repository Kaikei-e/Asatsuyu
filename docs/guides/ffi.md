# FFI Guide — Python Interop in Asatsuyu

Asatsuyu compiles to Python 3.12+ and can call Python libraries directly.
This guide explains the FFI trust model, compiler flags, generated package
layouts, and common failure modes.

---

## 1. FFI Trust Model

Asatsuyu classifies every Python module import into one of three trust levels:

| Level | Meaning | Runtime cost | Examples |
|---|---|---|---|
| **Verified** | Fully typed, no `Any` leaks | None — direct Python calls | `pathlib`, `os`, `sys` |
| **Checked** | Types present but contain `Any` or partial coverage | Runtime wrappers validate args/returns, exceptions become `Result` | `requests`, `json` |
| **Unsafe** | Dynamic surface, untyped | Opaque values only — no field access or pattern matching | *(future: pandas, torch)* |

### How trust is determined

1. The compiler resolves module type information (currently from hand-crafted
   builtin signatures; future: `.pyi` / typeshed / PEP 561).
2. An **admissibility checker** inspects every exported symbol for `Any` presence.
3. Symbols containing `Any` are downgraded from Verified to Checked.
4. The module trust level is the **minimum** across all its symbols.

Run `asatsuyu verify-ffi` to see the trust report for all known modules.

---

## 2. Verified FFI

Verified modules flow into Asatsuyu's type system as normal types with zero
runtime overhead.

```
from python import pathlib

pub fn read_config(path: String) -> String {
  let p = pathlib.Path(path)
  p.read_text()
}
```

Generated Python:

```python
import pathlib

def read_config(path: str) -> str:
    p = pathlib.Path(path)
    return p.read_text()
```

Currently supported Verified modules: `pathlib`, `os`, `sys`.

---

## 3. Checked FFI

Checked modules are wrapped with runtime validation. The compiler generates
`_asatsuyu_runtime` dispatch calls and exception-to-`Result` conversion.

```
from python import requests

pub fn fetch(url: String) -> Int {
  let response = requests.get(url)
  response.status_code
}
```

Generated Python uses `_asatsuyu_runtime.call_function()` with type validation
and exception normalization.

### The `try` expression

Use `try` at the Python boundary to convert exceptions into `Result`:

```
from python import requests

pub fn safe_fetch(url: String) -> Result(Int, PyException) {
  let response = try requests.get(url)
  Ok(response.status_code)
}
```

`try` wraps the call in `try/except`, returning `Error(PyException(...))` on
failure. The `PyException` type carries: `kind`, `exception_type`, `message`,
`module`, and `traceback_summary`.

Exception kinds follow a 9-category taxonomy:
`IoError`, `ValueError`, `TypeError`, `KeyError`, `AttributeError`,
`ImportError`, `ArithmeticError`, `RuntimeError`, `Other`.

Currently supported Checked modules: `requests`, `json`.

---

## 4. Unsafe / Opaque FFI

*(Not yet fully implemented.)*

Unsafe modules produce `PyOpaque` values that cannot be inspected, pattern-matched,
or implicitly converted. They can only be passed to other foreign calls or
explicitly converted through a checked boundary function.

---

## 5. Compiler Flags

### `--ffi-stdlib-only`

Restrict FFI resolution to standard library modules only (`pathlib`, `json`,
`os`, `sys`). Third-party modules like `requests` are rejected at compile time.

Available on: `check`, `build`, `run`.

```bash
asatsuyu check --ffi-stdlib-only src/main.asty
asatsuyu build --ffi-stdlib-only src/main.asty
```

### `--ffi-runtime on|off|auto`

Control whether the PyO3 native runtime extension (`_asatsuyu_runtime`) is
included in the generated package.

| Value | Behavior |
|---|---|
| `auto` (default) | Include only when Checked FFI calls are detected |
| `on` | Always include the runtime (forces maturin mixed layout) |
| `off` | Never include the runtime (pure Python prelude shim only) |

Available on: `build`, `run`.

```bash
asatsuyu build --ffi-runtime off src/main.asty
```

### `--no-emit-package`

Skip full package generation. Emit only the `.py` module file without
`pyproject.toml`, `__init__.py`, `py.typed`, or any packaging structure.

Useful for embedding Asatsuyu output into an existing Python project.

Available on: `build`.

```bash
asatsuyu build --no-emit-package -o lib/ src/main.asty
# Produces: lib/main.py
```

### `--ffi-stub-path <DIR>`

Specify additional directories for `.pyi` stub file lookup.
Reserved for future use — no stub-file resolver is implemented yet.

Available on: `check`, `build`, `run`.

---

## 6. Generated Package Layouts

### Pure Python (setuptools)

When only Verified FFI is used, the output is a standard Python package:

```
dist/
├── python/
│   └── my_app/
│       ├── __init__.py
│       ├── my_app.py
│       ├── py.typed
│       └── __main__.py      # if main() exists
└── pyproject.toml            # setuptools backend
```

Install and run:

```bash
asatsuyu build src/main.asty
cd dist && pip install -e . && python -m my_app
```

### Mixed layout (maturin)

When Checked FFI is used (and `--ffi-runtime` is not `off`), the output
includes a Rust extension module built with maturin:

```
dist/
├── python/
│   └── my_app/
│       ├── __init__.py
│       ├── my_app.py
│       ├── py.typed
│       ├── asatsuyu_prelude.py
│       ├── _asatsuyu_runtime.py   # pure-Python fallback
│       ├── _asatsuyu_runtime.pyi  # type stubs
│       └── __main__.py
├── src/
│   └── lib.rs                     # maturin wrapper
├── Cargo.toml                     # maturin build config
└── pyproject.toml                 # maturin backend
```

Install and run:

```bash
asatsuyu build src/main.asty
cd dist && maturin develop && python -m my_app
```

The pure-Python fallback (`_asatsuyu_runtime.py`) works without building the
native extension, but the native version provides better error messages and
performance.

### Module only (`--no-emit-package`)

```
dist/
└── main.py
```

No packaging structure. Suitable for embedding into existing projects.

---

## 7. Failure Modes

### E0208: Unknown Python module

```
error[E0208]: unknown Python module `numpy`
 --> src/main.asty:1:1
  | from python import numpy
  | ^^^^^^^^^^^^^^^^^^^^^^^^ not found in FFI registry
```

The module is not in the FFI registry. Currently only `pathlib`, `json`, `os`,
`sys`, and `requests` are supported.

### `--ffi-stdlib-only` rejection

```
error[E0208]: unknown Python module `requests`
```

When `--ffi-stdlib-only` is active, third-party modules are rejected. Remove the
flag or remove the import.

### Runtime not available

```
AsatsuyuError: _asatsuyu_runtime is not available...
```

The Checked FFI runtime extension is not installed. Either:
- Run `maturin develop` in the output directory, or
- Use `--ffi-runtime off` to use the pure-Python fallback.

### Type mismatch at FFI boundary

```
AsatsuyuError: requests.get returned unexpected type: NoneType
```

A Checked FFI call returned a value that doesn't match the expected type.
This indicates a bug in the stub definition or an unexpected runtime behavior.

### Stub path not found

The `--ffi-stub-path` directory does not exist. Verify the path is correct.
