//! Thin `PyO3` runtime boundary for Asatsuyu Checked FFI.
//!
//! Provides `_asatsuyu_runtime`, a Python extension module used by
//! Checked FFI generated code. Verified FFI does NOT use this module.
//!
//! # Exported functions
//!
//! - [`ffi_available`] — capability check (always returns `True`)
//! - [`import_module`] — import a Python module by name
//! - [`call_function`] — call a module-level function
//! - [`call_method`] — call a method on an object
//! - [`normalize_exception`] — classify an exception into Asatsuyu's taxonomy
//!
//! # Custom exception
//!
//! - `AsatsuyuError` — base exception for runtime FFI errors (subclass of `RuntimeError`)

mod classify;

use pyo3::prelude::*;
use pyo3::types::{PyAnyMethods, PyDict, PyTuple};

// ── Custom exception ──────────────────────────────────────────────

pyo3::create_exception!(
    _asatsuyu_runtime,
    AsatsuyuError,
    pyo3::exceptions::PyRuntimeError,
    "Base exception for Asatsuyu runtime errors."
);

// ── Module init ───────────────────────────────────────────────────

#[pymodule]
fn _asatsuyu_runtime(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(ffi_available, m)?)?;
    m.add_function(wrap_pyfunction!(import_module, m)?)?;
    m.add_function(wrap_pyfunction!(call_function, m)?)?;
    m.add_function(wrap_pyfunction!(call_method, m)?)?;
    m.add_function(wrap_pyfunction!(normalize_exception, m)?)?;
    m.add("AsatsuyuError", py.get_type::<AsatsuyuError>())?;
    Ok(())
}

// ── Capability check ──────────────────────────────────────────────

/// Returns `True` to confirm the native runtime extension is loaded.
#[pyfunction]
fn ffi_available() -> bool {
    true
}

// ── Module import ─────────────────────────────────────────────────

/// Import a Python module by name.
#[pyfunction]
fn import_module(py: Python<'_>, module_name: &str) -> PyResult<Py<PyAny>> {
    let module = py.import(module_name)?;
    Ok(module.into_any().unbind())
}

// ── Function call ─────────────────────────────────────────────────

/// Call a named function in a module with positional arguments.
#[pyfunction]
#[pyo3(signature = (module, func_name, *args))]
fn call_function(
    module: &Bound<'_, PyAny>,
    func_name: &str,
    args: &Bound<'_, PyTuple>,
) -> PyResult<Py<PyAny>> {
    let func = module.getattr(func_name)?;
    let result = func.call1(args)?;
    Ok(result.unbind())
}

// ── Method call ───────────────────────────────────────────────────

/// Call a named method on an object with positional arguments.
#[pyfunction]
#[pyo3(signature = (obj, method_name, *args))]
fn call_method(
    obj: &Bound<'_, PyAny>,
    method_name: &str,
    args: &Bound<'_, PyTuple>,
) -> PyResult<Py<PyAny>> {
    let result = obj.call_method1(method_name, args)?;
    Ok(result.unbind())
}

// ── Exception normalization ───────────────────────────────────────

/// Classify and normalize a Python exception into Asatsuyu's 5-field format.
///
/// Returns a dict with keys: `kind`, `exception_type`, `message`,
/// `module`, `traceback_summary`. The classification order and category
/// names match `_classify()` in `asatsuyu_prelude.py` exactly.
#[pyfunction]
fn normalize_exception(py: Python<'_>, exc: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let kind = classify::classify_exception(exc);

    let exc_type = exc.get_type();
    let exception_type: String = exc_type.getattr("__name__")?.extract()?;
    let message: String = exc.str()?.to_string();
    let module: String = exc_type
        .getattr("__module__")
        .and_then(|m| m.extract())
        .unwrap_or_else(|_| "builtins".to_string());

    let tb_mod = py.import("traceback")?;
    let formatted = tb_mod.call_method1("format_exception", (exc,))?;
    let parts: Vec<String> = formatted.extract()?;
    let traceback_summary = parts.join("");

    let dict = PyDict::new(py);
    dict.set_item("kind", kind)?;
    dict.set_item("exception_type", exception_type)?;
    dict.set_item("message", message)?;
    dict.set_item("module", module)?;
    dict.set_item("traceback_summary", traceback_summary)?;
    Ok(dict.into_any().unbind())
}
