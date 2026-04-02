//! Parse Python `.pyi` stub files into [`PythonModuleInfo`].
//!
//! Uses `libcst` to parse the full Python CST, then extracts the public API
//! surface: functions, classes, methods, properties, constants, and overloads.
//!
//! This parser intentionally does NOT attempt full semantic analysis of Python.
//! It extracts only what is needed for FFI-aware completion and type checking.
//! Unsupported constructs are silently skipped per symbol, never failing the
//! entire module.

use smol_str::SmolStr;

use super::index::{
    PythonClassInfo, PythonFunctionInfo, PythonMethodInfo, PythonModuleInfo, PythonSymbolInfo,
    PythonSymbolKind,
};
use super::model::{FfiParam, FfiSignature, FfiType};
use super::source::{ResolvedTypeSource, TypeSourceKind};

// ── Public API ───────────────────────────────────────────────────

/// Parse a `.pyi` stub file into a [`PythonModuleInfo`].
///
/// Returns `Ok` even when individual symbols cannot be parsed — those are
/// silently skipped. Returns `Err` only when libcst fails to parse the file
/// at all (syntax error severe enough to prevent tree construction).
pub fn parse_stub(
    module_name: &str,
    source: &str,
    source_kind: TypeSourceKind,
) -> Result<PythonModuleInfo, StubParseError> {
    let resolved_source = ResolvedTypeSource {
        module_name: SmolStr::from(module_name),
        source_kind,
        paths: Vec::new(),
        is_partial: false,
    };

    parse_type_source(&resolved_source, source)
}

/// Parse any resolved Python type source into a [`PythonModuleInfo`].
///
/// This supports both `.pyi` stubs and typed `.py` runtime sources. The
/// original resolution metadata is preserved on the returned module info.
pub fn parse_type_source(
    resolved_source: &ResolvedTypeSource,
    source: &str,
) -> Result<PythonModuleInfo, StubParseError> {
    let module = libcst_native::parse_module(source, None)
        .map_err(|e| StubParseError::SyntaxError(format!("{e}")))?;

    let mut info =
        PythonModuleInfo::new(resolved_source.module_name.clone(), resolved_source.clone());

    // Extract __all__ if present, to filter public symbols later.
    let all_names = extract_all_names(&module);

    for stmt in &module.body {
        extract_statement(stmt, &mut info, resolved_source.source_kind);
    }

    // If __all__ is defined, filter to only those names.
    if let Some(ref names) = all_names {
        info.symbols.retain(|sym| names.contains(&sym.name.to_string()));
    }

    Ok(info)
}

/// Error from parsing a `.pyi` stub file.
#[derive(Debug)]
pub enum StubParseError {
    /// The file could not be parsed at all.
    SyntaxError(String),
}

impl std::fmt::Display for StubParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SyntaxError(msg) => write!(f, "stub parse error: {msg}"),
        }
    }
}

// ── Statement extraction ─────────────────────────────────────────

fn extract_statement(
    stmt: &libcst_native::Statement<'_>,
    info: &mut PythonModuleInfo,
    provenance: TypeSourceKind,
) {
    match stmt {
        libcst_native::Statement::Simple(simple) => {
            for small in &simple.body {
                extract_small_statement(small, info, provenance);
            }
        }
        libcst_native::Statement::Compound(compound) => {
            extract_compound_statement(compound, info, provenance);
        }
    }
}

fn extract_small_statement(
    stmt: &libcst_native::SmallStatement<'_>,
    info: &mut PythonModuleInfo,
    provenance: TypeSourceKind,
) {
    match stmt {
        libcst_native::SmallStatement::AnnAssign(ann) => {
            // Module-level annotated assignment: `x: int` or `x: int = ...`
            if let Some(name) = extract_name_from_assign_target(&ann.target)
                && !name.starts_with('_')
                && let Some(ty) = annotation_to_ffi_type(&ann.annotation)
            {
                info.add_symbol(PythonSymbolInfo {
                    name: SmolStr::from(name),
                    kind: PythonSymbolKind::Constant(ty),
                    doc: None,
                    is_async: false,
                    provenance,
                });
            }
        }
        libcst_native::SmallStatement::Assign(_assign) => {
            // Simple assignment: `__all__ = [...]` or `x = value`
            // __all__ is handled separately; skip other assignments for now
            // as they're less common in stubs.
        }
        _ => {}
    }
}

fn extract_compound_statement(
    stmt: &libcst_native::CompoundStatement<'_>,
    info: &mut PythonModuleInfo,
    provenance: TypeSourceKind,
) {
    match stmt {
        libcst_native::CompoundStatement::FunctionDef(func) => {
            extract_function(func, info, provenance);
        }
        libcst_native::CompoundStatement::ClassDef(cls) => {
            extract_class(cls, info, provenance);
        }
        libcst_native::CompoundStatement::If(if_stmt) => {
            // Handle `if sys.version_info >= (3, 12):` blocks.
            // For now, include all branches (we target 3.12+).
            extract_if_body(if_stmt, info, provenance);
        }
        _ => {}
    }
}

// ── Function extraction ──────────────────────────────────────────

fn extract_function(
    func: &libcst_native::FunctionDef<'_>,
    info: &mut PythonModuleInfo,
    provenance: TypeSourceKind,
) {
    let name = func.name.value;
    if name.starts_with('_') && !name.starts_with("__") {
        return; // Skip private functions (but keep dunder)
    }

    let is_overload = has_decorator(&func.decorators, "overload");
    let is_async = func.asynchronous.is_some();
    let sig = extract_signature(func, is_async);

    // Check if we already have this function (overload case)
    if is_overload
        && let Some(existing) = info.symbols.iter_mut().find(|s| s.name == name)
        && let PythonSymbolKind::Function(ref mut func_info) = existing.kind
    {
        func_info.add_overload(sig);
        return;
    }

    // If not overloaded or first occurrence, add as new symbol
    info.add_symbol(PythonSymbolInfo {
        name: SmolStr::from(name),
        kind: PythonSymbolKind::Function(PythonFunctionInfo::single(sig)),
        doc: None,
        is_async,
        provenance,
    });
}

fn extract_signature(func: &libcst_native::FunctionDef<'_>, is_async: bool) -> FfiSignature {
    let params = extract_params(&func.params);
    let return_ty = func
        .returns
        .as_ref()
        .and_then(|ann| expr_to_ffi_type(&ann.annotation))
        .unwrap_or(FfiType::NoneType);

    FfiSignature { params, return_ty, is_async }
}

fn extract_params(params: &libcst_native::Parameters<'_>) -> Vec<FfiParam> {
    let mut result = Vec::new();

    for param in &params.params {
        // Skip `self` and `cls` parameters
        let name = param.name.value;
        if name == "self" || name == "cls" {
            continue;
        }

        let ty = param
            .annotation
            .as_ref()
            .and_then(|ann| expr_to_ffi_type(&ann.annotation))
            .unwrap_or(FfiType::Any);

        let has_default = param.default.is_some();

        result.push(FfiParam { name: SmolStr::from(name), ty, has_default });
    }

    // Also include keyword-only params (after *)
    if let Some(ref kwonly) = params.star_kwarg {
        // star_kwarg is **kwargs, skip it
        let _ = kwonly;
    }

    for param in &params.kwonly_params {
        let name = param.name.value;
        let ty = param
            .annotation
            .as_ref()
            .and_then(|ann| expr_to_ffi_type(&ann.annotation))
            .unwrap_or(FfiType::Any);
        let has_default = param.default.is_some();
        result.push(FfiParam { name: SmolStr::from(name), ty, has_default });
    }

    result
}

// ── Class extraction ─────────────────────────────────────────────

fn extract_class(
    cls: &libcst_native::ClassDef<'_>,
    info: &mut PythonModuleInfo,
    provenance: TypeSourceKind,
) {
    let name = cls.name.value;
    if name.starts_with('_') && !name.starts_with("__") {
        return; // Skip private classes
    }

    let mut class_info = PythonClassInfo::new(SmolStr::from(name));

    // Walk the class body
    match &cls.body {
        libcst_native::Suite::IndentedBlock(block) => {
            for stmt in &block.body {
                extract_class_member(stmt, &mut class_info);
            }
        }
        libcst_native::Suite::SimpleStatementSuite(simple) => {
            for small in &simple.body {
                extract_class_small_statement(small, &mut class_info);
            }
        }
    }

    // Consolidate overloaded methods
    consolidate_method_overloads(&mut class_info);

    info.add_symbol(PythonSymbolInfo {
        name: SmolStr::from(name),
        kind: PythonSymbolKind::Class(class_info),
        doc: None,
        is_async: false,
        provenance,
    });
}

fn extract_class_member(stmt: &libcst_native::Statement<'_>, class_info: &mut PythonClassInfo) {
    match stmt {
        libcst_native::Statement::Simple(simple) => {
            for small in &simple.body {
                extract_class_small_statement(small, class_info);
            }
        }
        libcst_native::Statement::Compound(compound) => match compound {
            libcst_native::CompoundStatement::FunctionDef(func) => {
                extract_class_method(func, class_info);
            }
            libcst_native::CompoundStatement::If(if_stmt) => {
                // Handle version conditionals inside class bodies
                extract_class_if_body(if_stmt, class_info);
            }
            _ => {}
        },
    }
}

fn extract_class_small_statement(
    stmt: &libcst_native::SmallStatement<'_>,
    class_info: &mut PythonClassInfo,
) {
    if let libcst_native::SmallStatement::AnnAssign(ann) = stmt {
        // Class-level annotation: `name: str` → property
        if let Some(name) = extract_name_from_assign_target(&ann.target)
            && !name.starts_with('_')
            && let Some(ty) = annotation_to_ffi_type(&ann.annotation)
        {
            class_info.properties.push((SmolStr::from(name), ty));
        }
    }
}

fn extract_class_method(func: &libcst_native::FunctionDef<'_>, class_info: &mut PythonClassInfo) {
    let name = func.name.value;
    let is_async = func.asynchronous.is_some();
    let is_property = has_decorator(&func.decorators, "property");
    let is_classmethod = has_decorator(&func.decorators, "classmethod");
    let is_staticmethod = has_decorator(&func.decorators, "staticmethod");
    let is_overload = has_decorator(&func.decorators, "overload");

    if name == "__init__" {
        // Constructor
        let sig = extract_signature(func, false);
        class_info.constructor = Some(sig);
        return;
    }

    // Skip private methods (but keep dunder)
    if name.starts_with('_') && !name.starts_with("__") {
        return;
    }

    if is_property {
        // Property: extract return type as property type
        let ty = func
            .returns
            .as_ref()
            .and_then(|ann| expr_to_ffi_type(&ann.annotation))
            .unwrap_or(FfiType::Any);
        class_info.properties.push((SmolStr::from(name), ty));
        return;
    }

    let sig = extract_signature(func, is_async);
    let method_info =
        PythonMethodInfo { name: SmolStr::from(name), signatures: vec![sig], is_async };

    if is_classmethod {
        if is_overload
            && let Some(existing) = class_info.class_methods.iter_mut().find(|m| m.name == name)
        {
            existing.signatures.extend(method_info.signatures);
            return;
        }
        class_info.class_methods.push(method_info);
    } else if is_staticmethod {
        if is_overload
            && let Some(existing) = class_info.static_methods.iter_mut().find(|m| m.name == name)
        {
            existing.signatures.extend(method_info.signatures);
            return;
        }
        class_info.static_methods.push(method_info);
    } else {
        if is_overload
            && let Some(existing) = class_info.methods.iter_mut().find(|m| m.name == name)
        {
            existing.signatures.extend(method_info.signatures);
            return;
        }
        class_info.methods.push(method_info);
    }
}

/// Merge overloaded methods that were added as separate entries.
fn consolidate_method_overloads(class_info: &mut PythonClassInfo) {
    dedup_methods(&mut class_info.methods);
    dedup_methods(&mut class_info.class_methods);
    dedup_methods(&mut class_info.static_methods);
}

fn dedup_methods(methods: &mut Vec<PythonMethodInfo>) {
    let mut i = 0;
    while i < methods.len() {
        let mut j = i + 1;
        while j < methods.len() {
            if methods[j].name == methods[i].name {
                let sigs = std::mem::take(&mut methods[j].signatures);
                methods[i].signatures.extend(sigs);
                methods.remove(j);
            } else {
                j += 1;
            }
        }
        i += 1;
    }
}

// ── If statement extraction (version conditionals) ───────────────

fn extract_class_if_body(if_stmt: &libcst_native::If<'_>, class_info: &mut PythonClassInfo) {
    // Include the if body (we target 3.12+)
    extract_class_suite(&if_stmt.body, class_info);
    // Also include else/elif
    if let Some(ref orelse) = if_stmt.orelse {
        match orelse.as_ref() {
            libcst_native::OrElse::Elif(elif) => {
                extract_class_if_body(elif, class_info);
            }
            libcst_native::OrElse::Else(else_clause) => {
                extract_class_suite(&else_clause.body, class_info);
            }
        }
    }
}

fn extract_class_suite(suite: &libcst_native::Suite<'_>, class_info: &mut PythonClassInfo) {
    match suite {
        libcst_native::Suite::IndentedBlock(block) => {
            for stmt in &block.body {
                extract_class_member(stmt, class_info);
            }
        }
        libcst_native::Suite::SimpleStatementSuite(simple) => {
            for small in &simple.body {
                extract_class_small_statement(small, class_info);
            }
        }
    }
}

fn extract_if_body(
    if_stmt: &libcst_native::If<'_>,
    info: &mut PythonModuleInfo,
    provenance: TypeSourceKind,
) {
    // Include the if body (we're targeting 3.12+, so most version guards pass)
    extract_suite(&if_stmt.body, info, provenance);

    // Also include else/elif bodies
    if let Some(ref orelse) = if_stmt.orelse {
        match orelse.as_ref() {
            libcst_native::OrElse::Elif(elif) => {
                extract_if_body(elif, info, provenance);
            }
            libcst_native::OrElse::Else(else_clause) => {
                extract_suite(&else_clause.body, info, provenance);
            }
        }
    }
}

fn extract_suite(
    suite: &libcst_native::Suite<'_>,
    info: &mut PythonModuleInfo,
    provenance: TypeSourceKind,
) {
    match suite {
        libcst_native::Suite::IndentedBlock(block) => {
            for stmt in &block.body {
                extract_statement(stmt, info, provenance);
            }
        }
        libcst_native::Suite::SimpleStatementSuite(simple) => {
            for small in &simple.body {
                extract_small_statement(small, info, provenance);
            }
        }
    }
}

// ── Type annotation → FfiType conversion ─────────────────────────

fn annotation_to_ffi_type(annotation: &libcst_native::Annotation<'_>) -> Option<FfiType> {
    expr_to_ffi_type(&annotation.annotation)
}

fn expr_to_ffi_type(expr: &libcst_native::Expression<'_>) -> Option<FfiType> {
    match expr {
        libcst_native::Expression::Name(name) => Some(name_to_ffi_type(name.value)),
        libcst_native::Expression::Attribute(attr) => {
            // e.g., `pathlib.Path`, `os.PathLike`
            let module = extract_dotted_name_from_expr(&attr.value)?;
            let name = attr.attr.value;
            Some(FfiType::Named { module: SmolStr::from(module), name: SmolStr::from(name) })
        }
        libcst_native::Expression::Subscript(sub) => {
            // e.g., `list[int]`, `dict[str, int]`, `Optional[str]`
            subscript_to_ffi_type(sub)
        }
        libcst_native::Expression::BinaryOperation(binop) => {
            // e.g., `int | str` (PEP 604 union syntax)
            if matches!(binop.operator, libcst_native::BinaryOp::BitOr { .. }) {
                let left = expr_to_ffi_type(&binop.left)?;
                let right = expr_to_ffi_type(&binop.right)?;

                // Special case: `T | None` → Optional(T)
                if right == FfiType::NoneType {
                    return Some(FfiType::Optional(Box::new(left)));
                }
                if left == FfiType::NoneType {
                    return Some(FfiType::Optional(Box::new(right)));
                }

                Some(FfiType::Union(vec![left, right]))
            } else {
                None
            }
        }
        libcst_native::Expression::Ellipsis(_) => {
            // `...` used as type (rare but valid)
            Some(FfiType::Any)
        }
        _ => {
            // Unsupported expression → treat as Any (graceful degradation)
            Some(FfiType::Any)
        }
    }
}

fn name_to_ffi_type(name: &str) -> FfiType {
    match name {
        "int" => FfiType::Int,
        "float" | "SupportsFloat" => FfiType::Float,
        "str" | "LiteralString" => FfiType::Str,
        "bool" | "SupportsInt" => FfiType::Bool,
        "None" | "NoneType" => FfiType::NoneType,
        "bytes" | "bytearray" | "ReadableBuffer" | "WriteableBuffer" => FfiType::Bytes,
        "Any" | "object" | "NoReturn" | "Never" => FfiType::Any,
        // Named types from other modules
        other => FfiType::Named { module: SmolStr::default(), name: SmolStr::from(other) },
    }
}

fn subscript_to_ffi_type(sub: &libcst_native::Subscript<'_>) -> Option<FfiType> {
    let base_name = extract_name_from_expr(&sub.value)?;
    let args = extract_subscript_args(&sub.slice);

    match base_name.as_str() {
        "list" | "List" | "Sequence" | "Iterable" | "Iterator" | "Set" | "set" | "FrozenSet"
        | "frozenset" => {
            let inner = args.first().cloned().unwrap_or(FfiType::Any);
            Some(FfiType::List(Box::new(inner)))
        }
        "dict" | "Dict" => {
            let key = args.first().cloned().unwrap_or(FfiType::Any);
            let val = args.get(1).cloned().unwrap_or(FfiType::Any);
            Some(FfiType::Dict(Box::new(key), Box::new(val)))
        }
        "tuple" | "Tuple" => Some(FfiType::Tuple(args)),
        "Optional" => {
            let inner = args.first().cloned().unwrap_or(FfiType::Any);
            Some(FfiType::Optional(Box::new(inner)))
        }
        "Union" => Some(FfiType::Union(args)),
        "ClassVar" | "Final" => args.first().cloned().or(Some(FfiType::Any)),
        "Callable" => Some(FfiType::Any),
        "Type" | "type" => Some(args.first().cloned().unwrap_or(FfiType::Any)),
        _ => Some(FfiType::Named { module: SmolStr::default(), name: SmolStr::from(base_name) }),
    }
}

fn extract_subscript_args(slice: &[libcst_native::SubscriptElement<'_>]) -> Vec<FfiType> {
    slice
        .iter()
        .filter_map(|el| match &el.slice {
            libcst_native::BaseSlice::Index(idx) => expr_to_ffi_type(&idx.value),
            libcst_native::BaseSlice::Slice(_) => None,
        })
        .collect()
}

// ── Helper functions ─────────────────────────────────────────────

fn has_decorator(decorators: &[libcst_native::Decorator<'_>], name: &str) -> bool {
    decorators.iter().any(|dec| {
        match &dec.decorator {
            libcst_native::Expression::Name(n) => n.value == name,
            libcst_native::Expression::Attribute(attr) => {
                // e.g., `typing.overload`
                attr.attr.value == name
            }
            _ => false,
        }
    })
}

fn extract_name_from_expr(expr: &libcst_native::Expression<'_>) -> Option<String> {
    match expr {
        libcst_native::Expression::Name(name) => Some(name.value.to_string()),
        libcst_native::Expression::Attribute(attr) => {
            let prefix = extract_name_from_expr(&attr.value)?;
            Some(format!("{prefix}.{}", attr.attr.value))
        }
        _ => None,
    }
}

fn extract_dotted_name_from_expr(expr: &libcst_native::Expression<'_>) -> Option<String> {
    extract_name_from_expr(expr)
}

fn extract_name_from_assign_target<'a>(
    target: &'a libcst_native::AssignTargetExpression<'a>,
) -> Option<&'a str> {
    match target {
        libcst_native::AssignTargetExpression::Name(name) => Some(name.value),
        _ => None,
    }
}

/// Extract `__all__` list from the module body.
fn extract_all_names(module: &libcst_native::Module<'_>) -> Option<Vec<String>> {
    for stmt in &module.body {
        if let libcst_native::Statement::Simple(simple) = stmt {
            for small in &simple.body {
                if let libcst_native::SmallStatement::Assign(assign) = small {
                    for target in &assign.targets {
                        if let libcst_native::AssignTargetExpression::Name(name) = &target.target
                            && name.value == "__all__"
                        {
                            return extract_string_list(&assign.value);
                        }
                    }
                }
            }
        }
    }
    None
}

fn extract_string_list(expr: &libcst_native::Expression<'_>) -> Option<Vec<String>> {
    match expr {
        libcst_native::Expression::List(list) => {
            let names: Vec<String> = list
                .elements
                .iter()
                .filter_map(|el| {
                    if let libcst_native::Element::Simple { value, .. } = el {
                        extract_string_value(value)
                    } else {
                        None
                    }
                })
                .collect();
            Some(names)
        }
        libcst_native::Expression::Tuple(tuple) => {
            let names: Vec<String> = tuple
                .elements
                .iter()
                .filter_map(|el| {
                    if let libcst_native::Element::Simple { value, .. } = el {
                        extract_string_value(value)
                    } else {
                        None
                    }
                })
                .collect();
            Some(names)
        }
        _ => None,
    }
}

fn extract_string_value(expr: &libcst_native::Expression<'_>) -> Option<String> {
    if let libcst_native::Expression::SimpleString(s) = expr {
        // Strip quotes: "foo" → foo, 'foo' → foo
        let raw = s.value;
        if (raw.starts_with('"') && raw.ends_with('"'))
            || (raw.starts_with('\'') && raw.ends_with('\''))
        {
            return Some(raw[1..raw.len() - 1].to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_function() {
        let source = "def hello(name: str) -> str: ...";
        let info = parse_stub("test", source, TypeSourceKind::Typeshed).expect("parse");
        assert_eq!(info.symbols.len(), 1);
        assert_eq!(info.symbols[0].name.as_str(), "hello");
        match &info.symbols[0].kind {
            PythonSymbolKind::Function(f) => {
                assert_eq!(f.signatures.len(), 1);
                assert_eq!(f.signatures[0].params.len(), 1);
                assert_eq!(f.signatures[0].params[0].name.as_str(), "name");
                assert_eq!(f.signatures[0].params[0].ty, FfiType::Str);
                assert_eq!(f.signatures[0].return_ty, FfiType::Str);
            }
            other => panic!("expected Function, got {other:?}"),
        }
    }

    #[test]
    fn parse_class_with_init_and_method() {
        let source = r"
class Path:
    def __init__(self, path: str = ...) -> None: ...
    def exists(self) -> bool: ...
    name: str
";
        let info = parse_stub("pathlib", source, TypeSourceKind::Typeshed).expect("parse");
        assert_eq!(info.symbols.len(), 1);
        match &info.symbols[0].kind {
            PythonSymbolKind::Class(cls) => {
                assert!(cls.constructor.is_some());
                let ctor = cls.constructor.as_ref().unwrap();
                assert_eq!(ctor.params.len(), 1); // self is skipped
                assert_eq!(ctor.params[0].name.as_str(), "path");
                assert!(ctor.params[0].has_default);

                assert_eq!(cls.methods.len(), 1);
                assert_eq!(cls.methods[0].name.as_str(), "exists");
                assert_eq!(cls.methods[0].signatures[0].return_ty, FfiType::Bool);

                assert_eq!(cls.properties.len(), 1);
                assert_eq!(cls.properties[0].0.as_str(), "name");
                assert_eq!(cls.properties[0].1, FfiType::Str);
            }
            other => panic!("expected Class, got {other:?}"),
        }
    }

    #[test]
    fn parse_property_decorator() {
        let source = r"
class Foo:
    @property
    def value(self) -> int: ...
";
        let info = parse_stub("test", source, TypeSourceKind::Typeshed).expect("parse");
        match &info.symbols[0].kind {
            PythonSymbolKind::Class(cls) => {
                // property should be in properties, not methods
                assert!(cls.methods.is_empty());
                assert_eq!(cls.properties.len(), 1);
                assert_eq!(cls.properties[0].0.as_str(), "value");
                assert_eq!(cls.properties[0].1, FfiType::Int);
            }
            other => panic!("expected Class, got {other:?}"),
        }
    }

    #[test]
    fn parse_overloaded_function() {
        let source = r"
from typing import overload

@overload
def parse(x: int) -> int: ...
@overload
def parse(x: str) -> str: ...
";
        let info = parse_stub("test", source, TypeSourceKind::Typeshed).expect("parse");
        assert_eq!(info.symbols.len(), 1);
        match &info.symbols[0].kind {
            PythonSymbolKind::Function(f) => {
                assert_eq!(f.signatures.len(), 2);
                assert_eq!(f.signatures[0].params[0].ty, FfiType::Int);
                assert_eq!(f.signatures[1].params[0].ty, FfiType::Str);
            }
            other => panic!("expected Function, got {other:?}"),
        }
    }

    #[test]
    fn parse_union_type() {
        let source = "def f(x: int | str) -> int | None: ...";
        let info = parse_stub("test", source, TypeSourceKind::Typeshed).expect("parse");
        match &info.symbols[0].kind {
            PythonSymbolKind::Function(f) => {
                assert_eq!(
                    f.signatures[0].params[0].ty,
                    FfiType::Union(vec![FfiType::Int, FfiType::Str])
                );
                // int | None becomes Optional(int)
                assert_eq!(f.signatures[0].return_ty, FfiType::Optional(Box::new(FfiType::Int)));
            }
            other => panic!("expected Function, got {other:?}"),
        }
    }

    #[test]
    fn parse_generic_types() {
        let source = "def f(x: list[int], y: dict[str, float]) -> tuple[int, str]: ...";
        let info = parse_stub("test", source, TypeSourceKind::Typeshed).expect("parse");
        match &info.symbols[0].kind {
            PythonSymbolKind::Function(f) => {
                assert_eq!(f.signatures[0].params[0].ty, FfiType::List(Box::new(FfiType::Int)));
                assert_eq!(
                    f.signatures[0].params[1].ty,
                    FfiType::Dict(Box::new(FfiType::Str), Box::new(FfiType::Float))
                );
                assert_eq!(
                    f.signatures[0].return_ty,
                    FfiType::Tuple(vec![FfiType::Int, FfiType::Str])
                );
            }
            other => panic!("expected Function, got {other:?}"),
        }
    }

    #[test]
    fn parse_module_constant() {
        let source = "sep: str\nlinesep: str\n";
        let info = parse_stub("os", source, TypeSourceKind::Typeshed).expect("parse");
        assert_eq!(info.symbols.len(), 2);
        assert_eq!(info.symbols[0].name.as_str(), "sep");
        match &info.symbols[0].kind {
            PythonSymbolKind::Constant(ty) => assert_eq!(*ty, FfiType::Str),
            other => panic!("expected Constant, got {other:?}"),
        }
    }

    #[test]
    fn skip_private_symbols() {
        let source = r"
def public() -> int: ...
def _private() -> int: ...
def __dunder__() -> int: ...
";
        let info = parse_stub("test", source, TypeSourceKind::Typeshed).expect("parse");
        let names: Vec<&str> = info.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"public"));
        assert!(!names.contains(&"_private"));
        assert!(names.contains(&"__dunder__"));
    }

    #[test]
    fn parse_async_function() {
        let source = "async def sleep(delay: float) -> None: ...";
        let info = parse_stub("asyncio", source, TypeSourceKind::Typeshed).expect("parse");
        assert!(info.symbols[0].is_async);
        match &info.symbols[0].kind {
            PythonSymbolKind::Function(f) => {
                assert!(f.signatures[0].is_async);
            }
            other => panic!("expected Function, got {other:?}"),
        }
    }

    #[test]
    fn parse_all_filter() {
        let source = r#"
__all__ = ["public_one", "public_two"]

def public_one() -> int: ...
def public_two() -> str: ...
def not_in_all() -> bool: ...
"#;
        let info = parse_stub("test", source, TypeSourceKind::Typeshed).expect("parse");
        let names: Vec<&str> = info.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"public_one"));
        assert!(names.contains(&"public_two"));
        assert!(!names.contains(&"not_in_all"));
    }

    #[test]
    fn parse_real_pathlib_stub() {
        // Parse the actual vendored pathlib stub
        let source = include_str!("../../../../vendor/typeshed/stdlib/pathlib/__init__.pyi");
        let info = parse_stub("pathlib", source, TypeSourceKind::Typeshed).expect("parse pathlib");

        // Should find Path class
        let path_sym = info.symbols.iter().find(|s| s.name == "Path").expect("should have Path");
        match &path_sym.kind {
            PythonSymbolKind::Class(cls) => {
                // Should have methods
                assert!(!cls.methods.is_empty(), "Path should have methods, got none");
                // Should have exists
                assert!(
                    cls.methods.iter().any(|m| m.name == "exists"),
                    "Path should have exists method"
                );
            }
            other => panic!("Path should be a Class, got {other:?}"),
        }
    }

    #[test]
    fn parse_real_os_stub() {
        let source = include_str!("../../../../vendor/typeshed/stdlib/os/__init__.pyi");
        let info = parse_stub("os", source, TypeSourceKind::Typeshed).expect("parse os");
        // os should have getcwd
        let has_getcwd = info.symbols.iter().any(|s| s.name == "getcwd");
        assert!(has_getcwd, "os should have getcwd");
    }

    #[test]
    fn parse_real_sys_stub() {
        let source = include_str!("../../../../vendor/typeshed/stdlib/sys/__init__.pyi");
        let info = parse_stub("sys", source, TypeSourceKind::Typeshed).expect("parse sys");
        // sys should have argv
        let has_argv = info.symbols.iter().any(|s| s.name == "argv");
        assert!(has_argv, "sys should have argv");
    }

    #[test]
    fn unsupported_construct_does_not_crash() {
        // A stub with complex constructs that we don't fully support
        let source = r#"
from typing import TypeVar, Protocol

_T = TypeVar("_T")

class Comparable(Protocol):
    def __lt__(self, other: object) -> bool: ...

def sort(items: list[_T]) -> list[_T]: ...
"#;
        let info = parse_stub("test", source, TypeSourceKind::Typeshed).expect("should not crash");
        // Should have extracted something, even if types are approximate
        assert!(!info.symbols.is_empty());
    }
}
