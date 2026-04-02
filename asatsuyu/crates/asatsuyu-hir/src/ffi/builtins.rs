//! Hand-crafted FFI signatures for Phase 1 stdlib modules.
//!
//! These cover the minimum surface needed for Verified FFI:
//! `pathlib`, `json`, `os`, `sys`.
//!
//! Each function returns an [`FfiModule`] with the symbols Asatsuyu
//! can safely use. The signatures are intentionally conservative —
//! they only expose what the MVP requires.

use smol_str::SmolStr;

use super::model::{
    FfiClass, FfiModule, FfiParam, FfiSignature, FfiSource, FfiSymbol, FfiSymbolKind,
    FfiTrustLevel, FfiType,
};

// ── Helpers ────────────────────────────────────────────────────────

fn param(name: &str, ty: FfiType) -> FfiParam {
    FfiParam { name: SmolStr::from(name), ty, has_default: false }
}

fn param_with_default(name: &str, ty: FfiType) -> FfiParam {
    FfiParam { name: SmolStr::from(name), ty, has_default: true }
}

fn sig(params: Vec<FfiParam>, return_ty: FfiType) -> FfiSignature {
    FfiSignature { params, return_ty, is_async: false }
}

fn async_sig(params: Vec<FfiParam>, return_ty: FfiType) -> FfiSignature {
    FfiSignature { params, return_ty, is_async: true }
}

fn func(name: &str, signature: FfiSignature) -> FfiSymbol {
    FfiSymbol {
        name: SmolStr::from(name),
        kind: FfiSymbolKind::Function(signature),
        trust_level: None,
    }
}

fn constant(name: &str, ty: FfiType) -> FfiSymbol {
    FfiSymbol { name: SmolStr::from(name), kind: FfiSymbolKind::Constant(ty), trust_level: None }
}

fn path_type() -> FfiType {
    FfiType::Named { module: SmolStr::from("pathlib"), name: SmolStr::from("Path") }
}

// ── pathlib ────────────────────────────────────────────────────────

/// Minimal surface for `pathlib` (Verified FFI).
///
/// Exposes `Path` class with constructor, common methods, and properties.
#[must_use]
pub fn pathlib_module() -> FfiModule {
    let path_class = FfiClass {
        name: SmolStr::from("Path"),
        constructor: Some(sig(vec![param_with_default("path", FfiType::Str)], path_type())),
        methods: vec![
            (
                SmolStr::from("read_text"),
                sig(
                    vec![param_with_default("encoding", FfiType::Optional(Box::new(FfiType::Str)))],
                    FfiType::Str,
                ),
            ),
            (
                SmolStr::from("write_text"),
                sig(
                    vec![
                        param("data", FfiType::Str),
                        param_with_default("encoding", FfiType::Optional(Box::new(FfiType::Str))),
                    ],
                    FfiType::Int,
                ),
            ),
            (SmolStr::from("exists"), sig(vec![], FfiType::Bool)),
            (SmolStr::from("is_file"), sig(vec![], FfiType::Bool)),
            (SmolStr::from("is_dir"), sig(vec![], FfiType::Bool)),
            (SmolStr::from("joinpath"), sig(vec![param("other", FfiType::Str)], path_type())),
            (
                SmolStr::from("mkdir"),
                sig(
                    vec![
                        param_with_default("mode", FfiType::Int),
                        param_with_default("parents", FfiType::Bool),
                        param_with_default("exist_ok", FfiType::Bool),
                    ],
                    FfiType::NoneType,
                ),
            ),
        ],
        properties: vec![
            (SmolStr::from("name"), FfiType::Str),
            (SmolStr::from("stem"), FfiType::Str),
            (SmolStr::from("suffix"), FfiType::Str),
            (SmolStr::from("parent"), path_type()),
            (SmolStr::from("parts"), FfiType::Tuple(vec![FfiType::Str])),
        ],
    };

    FfiModule {
        name: SmolStr::from("pathlib"),
        source: FfiSource::Builtin,
        trust_level: FfiTrustLevel::Verified,
        symbols: vec![FfiSymbol {
            name: SmolStr::from("Path"),
            kind: FfiSymbolKind::Class(path_class),
            trust_level: None,
        }],
    }
}

// ── json ───────────────────────────────────────────────────────────

/// Minimal surface for `json`.
///
/// Note: `loads` returns `Any` and `dumps` accepts `Any`, so these symbols
/// are classified as `Checked` by the admissibility checker.
#[must_use]
pub fn json_module() -> FfiModule {
    FfiModule {
        name: SmolStr::from("json"),
        source: FfiSource::Builtin,
        trust_level: FfiTrustLevel::Verified,
        symbols: vec![
            func("loads", sig(vec![param("s", FfiType::Str)], FfiType::Any)),
            func(
                "dumps",
                sig(
                    vec![
                        param("obj", FfiType::Any),
                        param_with_default("indent", FfiType::Optional(Box::new(FfiType::Int))),
                    ],
                    FfiType::Str,
                ),
            ),
        ],
    }
}

// ── os ─────────────────────────────────────────────────────────────

/// Minimal surface for `os` (Verified FFI).
#[must_use]
pub fn os_module() -> FfiModule {
    FfiModule {
        name: SmolStr::from("os"),
        source: FfiSource::Builtin,
        trust_level: FfiTrustLevel::Verified,
        symbols: vec![
            func(
                "getenv",
                sig(
                    vec![
                        param("key", FfiType::Str),
                        param_with_default("default", FfiType::Optional(Box::new(FfiType::Str))),
                    ],
                    FfiType::Optional(Box::new(FfiType::Str)),
                ),
            ),
            func("getcwd", sig(vec![], FfiType::Str)),
            constant("environ", FfiType::Dict(Box::new(FfiType::Str), Box::new(FfiType::Str))),
            constant("sep", FfiType::Str),
            constant("linesep", FfiType::Str),
        ],
    }
}

// ── sys ────────────────────────────────────────────────────────────

/// Minimal surface for `sys` (Verified FFI).
#[must_use]
pub fn sys_module() -> FfiModule {
    FfiModule {
        name: SmolStr::from("sys"),
        source: FfiSource::Builtin,
        trust_level: FfiTrustLevel::Verified,
        symbols: vec![
            constant("argv", FfiType::List(Box::new(FfiType::Str))),
            func("exit", sig(vec![param_with_default("code", FfiType::Int)], FfiType::NoneType)),
            constant("platform", FfiType::Str),
            constant("version", FfiType::Str),
        ],
    }
}

// ── requests ──────────────────────────────────────────────────────

fn response_type() -> FfiType {
    FfiType::Named { module: SmolStr::from("requests"), name: SmolStr::from("Response") }
}

/// Minimal surface for `requests` (Checked FFI).
///
/// `requests` does not ship `py.typed`; type info comes from `types-requests`
/// (typeshed stubs). MVP exposes `get`/`post`/`put`/`delete` with `url: str`
/// only. `Response.json()` returns `Any`, making the module `Checked`.
#[must_use]
pub fn requests_module() -> FfiModule {
    let response_class = FfiClass {
        name: SmolStr::from("Response"),
        constructor: None, // not directly constructed by users
        methods: vec![
            (SmolStr::from("json"), sig(vec![], FfiType::Any)),
            (SmolStr::from("raise_for_status"), sig(vec![], FfiType::NoneType)),
        ],
        properties: vec![
            (SmolStr::from("text"), FfiType::Str),
            (SmolStr::from("status_code"), FfiType::Int),
            (SmolStr::from("ok"), FfiType::Bool),
            (SmolStr::from("url"), FfiType::Str),
            (SmolStr::from("content"), FfiType::Bytes),
            (SmolStr::from("encoding"), FfiType::Optional(Box::new(FfiType::Str))),
        ],
    };

    FfiModule {
        name: SmolStr::from("requests"),
        source: FfiSource::Builtin,
        trust_level: FfiTrustLevel::Checked,
        symbols: vec![
            func("get", sig(vec![param("url", FfiType::Str)], response_type())),
            func("post", sig(vec![param("url", FfiType::Str)], response_type())),
            func("put", sig(vec![param("url", FfiType::Str)], response_type())),
            func("delete", sig(vec![param("url", FfiType::Str)], response_type())),
            FfiSymbol {
                name: SmolStr::from("Response"),
                kind: FfiSymbolKind::Class(response_class),
                trust_level: None,
            },
        ],
    }
}

// ── asyncio ──────────────────────────────────────────────────────

/// Minimal surface for `asyncio` (Verified FFI).
///
/// Exposes `sleep` as the primary async function for MVP testing.
/// `run` is synchronous (it blocks until the coroutine completes).
#[must_use]
pub fn asyncio_module() -> FfiModule {
    FfiModule {
        name: SmolStr::from("asyncio"),
        source: FfiSource::Builtin,
        trust_level: FfiTrustLevel::Verified,
        symbols: vec![
            func("sleep", async_sig(vec![param("delay", FfiType::Float)], FfiType::NoneType)),
            func("run", sig(vec![param("main", FfiType::Any)], FfiType::Any)),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pathlib_has_path_class() {
        let module = pathlib_module();
        let path_sym =
            module.symbols.iter().find(|s| s.name == "Path").expect("pathlib should have Path");
        match &path_sym.kind {
            FfiSymbolKind::Class(cls) => {
                assert!(cls.constructor.is_some());
                assert!(!cls.methods.is_empty());
                assert!(!cls.properties.is_empty());
                // Check specific methods exist
                let method_names: Vec<&str> = cls.methods.iter().map(|(n, _)| n.as_str()).collect();
                assert!(method_names.contains(&"read_text"));
                assert!(method_names.contains(&"exists"));
                assert!(method_names.contains(&"joinpath"));
            }
            other => panic!("expected Class, got {other:?}"),
        }
    }

    #[test]
    fn json_has_loads_and_dumps() {
        let module = json_module();
        let names: Vec<&str> = module.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"loads"));
        assert!(names.contains(&"dumps"));
    }

    #[test]
    fn os_has_getenv_and_environ() {
        let module = os_module();
        let names: Vec<&str> = module.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"getenv"));
        assert!(names.contains(&"environ"));
    }

    #[test]
    fn sys_has_argv_and_exit() {
        let module = sys_module();
        let names: Vec<&str> = module.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"argv"));
        assert!(names.contains(&"exit"));
    }

    #[test]
    fn requests_has_get_post_and_response() {
        let module = requests_module();
        let names: Vec<&str> = module.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"get"));
        assert!(names.contains(&"post"));
        assert!(names.contains(&"put"));
        assert!(names.contains(&"delete"));
        assert!(names.contains(&"Response"));
    }

    #[test]
    fn requests_response_has_json_and_text() {
        let module = requests_module();
        let resp_sym = module
            .symbols
            .iter()
            .find(|s| s.name == "Response")
            .expect("requests should have Response");
        match &resp_sym.kind {
            FfiSymbolKind::Class(cls) => {
                assert!(cls.constructor.is_none());
                let method_names: Vec<&str> = cls.methods.iter().map(|(n, _)| n.as_str()).collect();
                assert!(method_names.contains(&"json"));
                assert!(method_names.contains(&"raise_for_status"));
                let prop_names: Vec<&str> =
                    cls.properties.iter().map(|(n, _)| n.as_str()).collect();
                assert!(prop_names.contains(&"text"));
                assert!(prop_names.contains(&"status_code"));
                assert!(prop_names.contains(&"ok"));
            }
            other => panic!("expected Class, got {other:?}"),
        }
    }

    #[test]
    fn asyncio_has_sleep_and_run() {
        let module = asyncio_module();
        let names: Vec<&str> = module.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"sleep"));
        assert!(names.contains(&"run"));
    }

    #[test]
    fn asyncio_sleep_is_async() {
        let module = asyncio_module();
        let sleep = module.symbols.iter().find(|s| s.name == "sleep").unwrap();
        match &sleep.kind {
            FfiSymbolKind::Function(sig) => {
                assert!(sig.is_async, "asyncio.sleep should be async");
                assert_eq!(sig.return_ty, FfiType::NoneType);
            }
            other => panic!("expected Function, got {other:?}"),
        }
    }

    #[test]
    fn asyncio_run_is_sync() {
        let module = asyncio_module();
        let run = module.symbols.iter().find(|s| s.name == "run").unwrap();
        match &run.kind {
            FfiSymbolKind::Function(sig) => {
                assert!(!sig.is_async, "asyncio.run should be sync");
            }
            other => panic!("expected Function, got {other:?}"),
        }
    }
}
