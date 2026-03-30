//! Exception classification mirroring the prelude's `_classify()` function.
//!
//! Maintains the same `isinstance` check order and category names as
//! `asatsuyu-backend-python/src/prelude.rs` to ensure consistency.

use pyo3::prelude::*;

/// Exception category strings, matching the prelude's `_classify()` output.
pub(crate) const IO_ERROR: &str = "IoError";
pub(crate) const VALUE_ERROR: &str = "ValueError";
pub(crate) const TYPE_ERROR: &str = "TypeError";
pub(crate) const KEY_ERROR: &str = "KeyError";
pub(crate) const ATTRIBUTE_ERROR: &str = "AttributeError";
pub(crate) const IMPORT_ERROR: &str = "ImportError";
pub(crate) const ARITHMETIC_ERROR: &str = "ArithmeticError";
pub(crate) const RUNTIME_ERROR: &str = "RuntimeError";
pub(crate) const OTHER: &str = "Other";

/// Classify a Python exception into one of the 9 Asatsuyu exception categories.
///
/// The `isinstance` check order matches `_classify()` in the generated prelude
/// exactly, so that Checked FFI (runtime) and Verified FFI (pure Python) produce
/// identical category strings for the same exception.
pub(crate) fn classify_exception(exc: &Bound<'_, PyAny>) -> &'static str {
    if exc.is_instance_of::<pyo3::exceptions::PyOSError>() {
        return IO_ERROR;
    }
    if exc.is_instance_of::<pyo3::exceptions::PyValueError>() {
        return VALUE_ERROR;
    }
    if exc.is_instance_of::<pyo3::exceptions::PyTypeError>() {
        return TYPE_ERROR;
    }
    if exc.is_instance_of::<pyo3::exceptions::PyKeyError>() {
        return KEY_ERROR;
    }
    if exc.is_instance_of::<pyo3::exceptions::PyAttributeError>() {
        return ATTRIBUTE_ERROR;
    }
    if exc.is_instance_of::<pyo3::exceptions::PyImportError>() {
        return IMPORT_ERROR;
    }
    if exc.is_instance_of::<pyo3::exceptions::PyArithmeticError>() {
        return ARITHMETIC_ERROR;
    }
    if exc.is_instance_of::<pyo3::exceptions::PyRuntimeError>() {
        return RUNTIME_ERROR;
    }
    OTHER
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_strings_match_prelude() {
        assert_eq!(IO_ERROR, "IoError");
        assert_eq!(VALUE_ERROR, "ValueError");
        assert_eq!(TYPE_ERROR, "TypeError");
        assert_eq!(KEY_ERROR, "KeyError");
        assert_eq!(ATTRIBUTE_ERROR, "AttributeError");
        assert_eq!(IMPORT_ERROR, "ImportError");
        assert_eq!(ARITHMETIC_ERROR, "ArithmeticError");
        assert_eq!(RUNTIME_ERROR, "RuntimeError");
        assert_eq!(OTHER, "Other");
    }
}
