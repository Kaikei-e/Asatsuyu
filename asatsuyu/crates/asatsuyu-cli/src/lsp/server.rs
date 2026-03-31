//! Tower-LSP server implementation for Asatsuyu.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use asatsuyu_hir::DefKind;
use asatsuyu_hir::ffi::FfiResolverConfig;
use asatsuyu_syntax::{FileId, LineIndex};
use asatsuyu_ty::ThirModule;
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionOptions, CompletionParams, CompletionResponse,
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DidSaveTextDocumentParams, DocumentFormattingParams, DocumentSymbol, DocumentSymbolParams,
    DocumentSymbolResponse, GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverContents,
    HoverParams, HoverProviderCapability, InitializeParams, InitializeResult, InitializedParams,
    Location, MarkupContent, MarkupKind, MessageType, OneOf, Position, PrepareRenameResponse,
    ReferenceParams, RenameOptions, RenameParams, ServerCapabilities, ServerInfo, SymbolKind,
    TextDocumentPositionParams, TextDocumentSyncCapability, TextDocumentSyncKind,
    TextDocumentSyncOptions, TextEdit, Url, WorkspaceEdit,
};
use tower_lsp::{Client, LanguageServer, LspService, Server};

use super::{analysis, convert};

// ── File state ──────────────────────────────────────────────────

struct FileState {
    source: String,
    line_index: LineIndex,
    thir: Option<ThirModule>,
}

// ── Backend ─────────────────────────────────────────────────────

pub(super) struct Backend {
    client: Client,
    state: Arc<RwLock<HashMap<Url, FileState>>>,
}

impl Backend {
    fn new(client: Client) -> Self {
        Self { client, state: Arc::new(RwLock::new(HashMap::new())) }
    }

    /// Run the full compilation pipeline on a source file and update state.
    async fn analyze_file(&self, uri: &Url, source: String) {
        let line_index = LineIndex::new(&source);
        let (thir, diagnostics) = compile_source(&source);

        let lsp_diags = convert::to_lsp_diagnostics(&diagnostics, &line_index, uri);

        // Update stored state.
        {
            let mut state = self.state.write().await;
            state.insert(uri.clone(), FileState { source, line_index, thir });
        }

        // Publish diagnostics to the editor.
        self.client.publish_diagnostics(uri.clone(), lsp_diags, None).await;
    }
}

/// Run the Asatsuyu compilation pipeline on source text.
///
/// Returns the THIR module (if type checking succeeded) and all diagnostics.
fn compile_source(source: &str) -> (Option<ThirModule>, Vec<asatsuyu_syntax::Diagnostic>) {
    let mut all_diags = Vec::new();

    // 1. Parse
    let parse_result = asatsuyu_parser::parse(FileId(0), source);
    all_diags.extend(parse_result.diagnostics().iter().cloned());
    if parse_result.has_errors() {
        return (None, all_diags);
    }

    // 2. AST lowering
    let ast_result = asatsuyu_ast::lower(&parse_result, FileId(0));
    all_diags.extend(ast_result.diagnostics.iter().cloned());
    if ast_result.has_errors() {
        return (None, all_diags);
    }

    // 3. HIR lowering (name resolution)
    let hir_result = asatsuyu_hir::lower_to_hir(&ast_result.module);
    all_diags.extend(hir_result.diagnostics.iter().cloned());
    if hir_result.has_errors() {
        return (None, all_diags);
    }

    // 4. Type checking
    let ffi_config = FfiResolverConfig::default();
    let ty_result = asatsuyu_ty::check_types_with_ffi_config(&hir_result.module, &ffi_config);
    all_diags.extend(ty_result.diagnostics.iter().cloned());

    // Return THIR even if there are type errors — partial info is useful.
    (Some(ty_result.module), all_diags)
}

// ── LanguageServer trait ────────────────────────────────────────

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _params: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(TextDocumentSyncKind::FULL),
                        save: Some(true.into()),
                        ..Default::default()
                    },
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                document_formatting_provider: Some(OneOf::Left(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![".".to_owned()]),
                    ..Default::default()
                }),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options:
                        tower_lsp::lsp_types::WorkDoneProgressOptions::default(),
                })),
                references_provider: Some(OneOf::Left(true)),
                document_symbol_provider: Some(OneOf::Left(true)),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "asatsuyu-lsp".to_owned(),
                version: Some(env!("CARGO_PKG_VERSION").to_owned()),
            }),
        })
    }

    async fn initialized(&self, _params: InitializedParams) {
        self.client.log_message(MessageType::INFO, "Asatsuyu LSP initialized").await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    // ── Text synchronization ────────────────────────────────────

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let source = params.text_document.text;
        self.analyze_file(&uri, source).await;
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let uri = params.text_document.uri;

        // Read source from disk (did_save may not include text).
        let path = uri_to_path(&uri);
        let source = if let Some(s) = path.and_then(|p| std::fs::read_to_string(p).ok()) {
            s
        } else {
            // Fallback: try to use stored source.
            let state = self.state.read().await;
            let Some(fs) = state.get(&uri) else { return };
            fs.source.clone()
        };
        self.analyze_file(&uri, source).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        // With Full sync, the last change contains the full text.
        let uri = params.text_document.uri;
        if let Some(change) = params.content_changes.into_iter().last() {
            // Update stored source but don't re-analyze on every keystroke.
            // Analysis happens on save.
            let line_index = LineIndex::new(&change.text);
            let mut state = self.state.write().await;
            if let Some(fs) = state.get_mut(&uri) {
                fs.source = change.text;
                fs.line_index = line_index;
                fs.thir = None;
            } else {
                state.insert(uri, FileState { source: change.text, line_index, thir: None });
            }
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        // Clear diagnostics and state.
        self.client.publish_diagnostics(uri.clone(), vec![], None).await;
        let mut state = self.state.write().await;
        state.remove(&uri);
    }

    // ── Hover ───────────────────────────────────────────────────

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;

        let state = self.state.read().await;
        let Some(file_state) = state.get(uri) else {
            return Ok(None);
        };
        let Some(ref thir) = file_state.thir else {
            return Ok(None);
        };

        let offset = position_to_offset(pos, &file_state.source);
        let Some(text) = analysis::hover_at_offset(thir, offset) else {
            return Ok(None);
        };

        Ok(Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: format!("```asatsuyu\n{text}\n```"),
            }),
            range: None,
        }))
    }

    // ── Go to definition ────────────────────────────────────────

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;

        let state = self.state.read().await;
        let Some(file_state) = state.get(uri) else {
            return Ok(None);
        };
        let Some(ref thir) = file_state.thir else {
            return Ok(None);
        };

        let offset = position_to_offset(pos, &file_state.source);
        let Some(def_span) = analysis::find_definition_at_offset(thir, offset) else {
            return Ok(None);
        };

        let range = convert::span_to_range(def_span, &file_state.line_index);
        Ok(Some(GotoDefinitionResponse::Scalar(Location { uri: uri.clone(), range })))
    }

    // ── Document formatting ─────────────────────────────────────

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let uri = &params.text_document.uri;

        let state = self.state.read().await;
        let Some(file_state) = state.get(uri) else {
            return Ok(None);
        };

        let result = asatsuyu_parser::format_source(&file_state.source);
        if result.has_parse_errors || result.formatted == file_state.source {
            return Ok(None);
        }

        // Replace the entire document using a precise EOF range.
        #[allow(clippy::cast_possible_truncation)]
        let eof_span = asatsuyu_syntax::Span::new(FileId(0), 0, file_state.source.len() as u32);
        let full_range = convert::span_to_range(eof_span, &file_state.line_index);

        Ok(Some(vec![TextEdit { range: full_range, new_text: result.formatted }]))
    }

    // ── Completion ──────────────────────────────────────────────

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = &params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;

        let state = self.state.read().await;
        let Some(file_state) = state.get(uri) else {
            return Ok(None);
        };
        let Some(ref thir) = file_state.thir else {
            return Ok(None);
        };

        let offset = position_to_offset(pos, &file_state.source);
        let entries = analysis::collect_completions(thir, offset);

        let items: Vec<CompletionItem> = entries
            .into_iter()
            .map(|entry| CompletionItem {
                label: entry.name.to_string(),
                kind: Some(def_kind_to_completion_kind(entry.kind)),
                detail: entry.ty.map(|t| format!("{t}")),
                ..Default::default()
            })
            .collect();

        Ok(Some(CompletionResponse::Array(items)))
    }

    // ── Rename ──────────────────────────────────────────────────

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        let uri = &params.text_document.uri;
        let pos = params.position;

        let state = self.state.read().await;
        let Some(file_state) = state.get(uri) else {
            return Ok(None);
        };
        let Some(ref thir) = file_state.thir else {
            return Ok(None);
        };

        let offset = position_to_offset(pos, &file_state.source);
        let Some(info) = analysis::find_node_at_offset(thir, offset) else {
            return Ok(None);
        };

        // Only allow rename on named nodes (Var, FnDef).
        let def_id = match info {
            analysis::NodeInfo::Var { def_id, .. } | analysis::NodeInfo::FnDef { def_id, .. } => {
                def_id
            }
            analysis::NodeInfo::Expr { .. } => return Ok(None),
        };

        let def = thir.symbol_table.get(def_id);
        let range = convert::span_to_range(def.span, &file_state.line_index);
        Ok(Some(PrepareRenameResponse::Range(range)))
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let uri = &params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;
        let new_name = &params.new_name;

        let state = self.state.read().await;
        let Some(file_state) = state.get(uri) else {
            return Ok(None);
        };
        let Some(ref thir) = file_state.thir else {
            return Ok(None);
        };

        let offset = position_to_offset(pos, &file_state.source);
        let Some(info) = analysis::find_node_at_offset(thir, offset) else {
            return Ok(None);
        };

        let def_id = match info {
            analysis::NodeInfo::Var { def_id, .. } | analysis::NodeInfo::FnDef { def_id, .. } => {
                def_id
            }
            analysis::NodeInfo::Expr { .. } => return Ok(None),
        };

        let ref_spans = analysis::find_all_references(thir, def_id);
        let edits: Vec<TextEdit> = ref_spans
            .into_iter()
            .map(|span| TextEdit {
                range: convert::span_to_range(span, &file_state.line_index),
                new_text: new_name.clone(),
            })
            .collect();

        let mut changes = HashMap::new();
        changes.insert(uri.clone(), edits);
        Ok(Some(WorkspaceEdit { changes: Some(changes), ..Default::default() }))
    }

    // ── References ──────────────────────────────────────────────

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = &params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;

        let state = self.state.read().await;
        let Some(file_state) = state.get(uri) else {
            return Ok(None);
        };
        let Some(ref thir) = file_state.thir else {
            return Ok(None);
        };

        let offset = position_to_offset(pos, &file_state.source);
        let Some(info) = analysis::find_node_at_offset(thir, offset) else {
            return Ok(None);
        };

        let def_id = match info {
            analysis::NodeInfo::Var { def_id, .. } | analysis::NodeInfo::FnDef { def_id, .. } => {
                def_id
            }
            analysis::NodeInfo::Expr { .. } => return Ok(None),
        };

        let ref_spans = analysis::find_all_references(thir, def_id);
        let locations: Vec<Location> = ref_spans
            .into_iter()
            .map(|span| Location {
                uri: uri.clone(),
                range: convert::span_to_range(span, &file_state.line_index),
            })
            .collect();

        Ok(Some(locations))
    }

    // ── Document symbols ────────────────────────────────────────

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = &params.text_document.uri;

        let state = self.state.read().await;
        let Some(file_state) = state.get(uri) else {
            return Ok(None);
        };
        let Some(ref thir) = file_state.thir else {
            return Ok(None);
        };

        let entries = analysis::collect_document_symbols(thir);
        #[allow(deprecated)] // `deprecated` field is required but deprecated in LSP spec
        let symbols: Vec<DocumentSymbol> = entries
            .into_iter()
            .map(|entry| {
                let range = convert::span_to_range(entry.span, &file_state.line_index);
                DocumentSymbol {
                    name: entry.name.to_string(),
                    detail: None,
                    kind: def_kind_to_symbol_kind(entry.kind),
                    tags: None,
                    deprecated: None,
                    range,
                    selection_range: range,
                    children: None,
                }
            })
            .collect();

        Ok(Some(DocumentSymbolResponse::Nested(symbols)))
    }
}

// ── Helpers ─────────────────────────────────────────────────────

/// Convert an LSP `Position` (0-based line, 0-based character) to a byte offset.
///
/// Scans the source text to find the byte offset at the given line/column.
fn position_to_offset(pos: Position, source: &str) -> u32 {
    let target_line = pos.line as usize;
    let target_col = pos.character as usize;
    let mut current_line = 0usize;
    let mut line_start = 0usize;

    for (i, byte) in source.bytes().enumerate() {
        if current_line == target_line {
            let col = i - line_start;
            if col == target_col {
                #[allow(clippy::cast_possible_truncation)]
                return i as u32;
            }
        }
        if byte == b'\n' {
            if current_line == target_line {
                // Column is past end of line — clamp to end.
                #[allow(clippy::cast_possible_truncation)]
                return i as u32;
            }
            current_line += 1;
            line_start = i + 1;
        }
    }

    // Past end of file or at the last line.
    if current_line == target_line {
        let offset = line_start + target_col;
        #[allow(clippy::cast_possible_truncation)]
        return offset.min(source.len()) as u32;
    }
    #[allow(clippy::cast_possible_truncation)]
    {
        source.len() as u32
    }
}

/// Convert a file URI to a filesystem path.
fn uri_to_path(uri: &Url) -> Option<PathBuf> {
    uri.to_file_path().ok()
}

/// Map `DefKind` to LSP `CompletionItemKind`.
fn def_kind_to_completion_kind(kind: DefKind) -> CompletionItemKind {
    match kind {
        DefKind::Function | DefKind::Builtin => CompletionItemKind::FUNCTION,
        DefKind::Parameter | DefKind::LocalBinding => CompletionItemKind::VARIABLE,
        DefKind::Constructor => CompletionItemKind::CONSTRUCTOR,
        DefKind::Type => CompletionItemKind::STRUCT,
        DefKind::Import => CompletionItemKind::MODULE,
    }
}

/// Map `DefKind` to LSP `SymbolKind`.
fn def_kind_to_symbol_kind(kind: DefKind) -> SymbolKind {
    match kind {
        DefKind::Function | DefKind::Builtin => SymbolKind::FUNCTION,
        DefKind::Parameter | DefKind::LocalBinding => SymbolKind::VARIABLE,
        DefKind::Constructor => SymbolKind::CONSTRUCTOR,
        DefKind::Type => SymbolKind::STRUCT,
        DefKind::Import => SymbolKind::MODULE,
    }
}

// ── Server startup ──────────────────────────────────────────────

pub(super) async fn run_server() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_to_offset_clamps_at_line_end() {
        let source = "ab\ncd\n";
        assert_eq!(position_to_offset(Position { line: 0, character: 1 }, source), 1);
        assert_eq!(position_to_offset(Position { line: 0, character: 99 }, source), 2);
        assert_eq!(position_to_offset(Position { line: 1, character: 1 }, source), 4);
    }

    #[test]
    fn formatting_range_covers_entire_document() {
        let source = "a\n";
        let index = LineIndex::new(source);
        #[allow(clippy::cast_possible_truncation)]
        let eof_span = asatsuyu_syntax::Span::new(FileId(0), 0, source.len() as u32);
        let range = convert::span_to_range(eof_span, &index);
        assert_eq!(range.start, Position { line: 0, character: 0 });
        assert_eq!(range.end, Position { line: 1, character: 0 });
    }
}
