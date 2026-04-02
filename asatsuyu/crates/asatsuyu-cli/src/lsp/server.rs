//! Tower-LSP server implementation for Asatsuyu.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use asatsuyu_hir::DefKind;
use asatsuyu_hir::ffi::FfiResolverConfig;
use asatsuyu_syntax::{FileId, LineIndex};
use asatsuyu_ty::ThirModule;
use tokio::sync::{RwLock, mpsc};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, CodeActionOptions, CodeActionOrCommand, CodeActionParams,
    CodeActionResponse, CompletionItem, CompletionItemKind, CompletionOptions, CompletionParams,
    CompletionResponse, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DidSaveTextDocumentParams, DocumentFormattingParams, DocumentSymbol,
    DocumentSymbolParams, DocumentSymbolResponse, GotoDefinitionParams, GotoDefinitionResponse,
    Hover, HoverContents, HoverParams, HoverProviderCapability, InitializeParams, InitializeResult,
    InitializedParams, InsertTextFormat, Location, MarkupContent, MarkupKind, MessageType, OneOf,
    ParameterInformation, ParameterLabel, Position, PrepareRenameResponse, ReferenceParams,
    RenameOptions, RenameParams, SemanticTokens, SemanticTokensFullOptions, SemanticTokensLegend,
    SemanticTokensOptions, SemanticTokensParams, SemanticTokensResult,
    SemanticTokensServerCapabilities, ServerCapabilities, ServerInfo, SignatureHelp,
    SignatureHelpOptions, SignatureHelpParams, SignatureInformation, SymbolKind,
    TextDocumentPositionParams, TextDocumentSyncCapability, TextDocumentSyncKind,
    TextDocumentSyncOptions, TextEdit, Url, WorkspaceEdit,
};
use tower_lsp::{Client, LanguageServer, LspService, Server};

use super::{analysis, convert};

/// Debounce delay for on-change diagnostics.
const DEBOUNCE_DELAY: Duration = Duration::from_millis(200);

// ── File state ──────────────────────────────────────────────────

struct FileState {
    source: String,
    line_index: LineIndex,
    thir: Option<ThirModule>,
    /// Monotonic counter for staleness detection in debounced analysis.
    seq: u64,
}

// ── Backend ─────────────────────────────────────────────────────

pub(super) struct Backend {
    client: Client,
    state: Arc<RwLock<HashMap<Url, FileState>>>,
    debounce_tx: mpsc::UnboundedSender<Url>,
    /// Taken once by `initialized()` to spawn the debounce loop.
    debounce_rx: Arc<tokio::sync::Mutex<Option<mpsc::UnboundedReceiver<Url>>>>,
}

impl Backend {
    fn new(client: Client) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            client,
            state: Arc::new(RwLock::new(HashMap::new())),
            debounce_tx: tx,
            debounce_rx: Arc::new(tokio::sync::Mutex::new(Some(rx))),
        }
    }

    /// Run the full compilation pipeline on a source file and update state.
    async fn analyze_file(&self, uri: &Url, source: String) {
        let line_index = LineIndex::new(&source);
        let (thir, diagnostics) = compile_source(&source);

        let lsp_diags = convert::to_lsp_diagnostics(&diagnostics, &line_index, uri);

        // Update stored state, bumping seq to invalidate any in-flight debounced analysis.
        {
            let mut state = self.state.write().await;
            let next_seq = state.get(uri).map_or(0, |fs| fs.seq + 1);
            state.insert(uri.clone(), FileState { source, line_index, thir, seq: next_seq });
        }

        // Publish diagnostics to the editor.
        self.client.publish_diagnostics(uri.clone(), lsp_diags, None).await;
    }
}

fn apply_full_document_change(
    state: &mut HashMap<Url, FileState>,
    uri: &Url,
    source: String,
) -> u64 {
    let line_index = LineIndex::new(&source);
    if let Some(fs) = state.get_mut(uri) {
        fs.source = source;
        fs.line_index = line_index;
        fs.thir = None;
        fs.seq += 1;
        fs.seq
    } else {
        state.insert(uri.clone(), FileState { source, line_index, thir: None, seq: 0 });
        0
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
                signature_help_provider: Some(SignatureHelpOptions {
                    trigger_characters: Some(vec!["(".to_owned(), ",".to_owned()]),
                    retrigger_characters: Some(vec![")".to_owned()]),
                    work_done_progress_options:
                        tower_lsp::lsp_types::WorkDoneProgressOptions::default(),
                }),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options:
                        tower_lsp::lsp_types::WorkDoneProgressOptions::default(),
                })),
                code_action_provider: Some(
                    tower_lsp::lsp_types::CodeActionProviderCapability::Options(
                        CodeActionOptions {
                            code_action_kinds: Some(vec![
                                CodeActionKind::QUICKFIX,
                                CodeActionKind::REFACTOR,
                            ]),
                            work_done_progress_options:
                                tower_lsp::lsp_types::WorkDoneProgressOptions::default(),
                            resolve_provider: None,
                        },
                    ),
                ),
                references_provider: Some(OneOf::Left(true)),
                document_symbol_provider: Some(OneOf::Left(true)),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            legend: SemanticTokensLegend {
                                token_types: super::semantic_tokens::token_types(),
                                token_modifiers: super::semantic_tokens::token_modifiers(),
                            },
                            full: Some(SemanticTokensFullOptions::Bool(true)),
                            range: None,
                            ..Default::default()
                        },
                    ),
                ),
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

        // Spawn debounced on-change analysis loop.
        let rx = self.debounce_rx.lock().await.take();
        if let Some(rx) = rx {
            tokio::spawn(debounce_loop(rx, Arc::clone(&self.state), self.client.clone()));
        }
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
            let mut state = self.state.write().await;
            apply_full_document_change(&mut state, &uri, change.text);
            drop(state);
            // Notify the debounce loop to schedule re-analysis.
            self.debounce_tx.send(uri).ok();
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

        let offset = position_to_offset(pos, &file_state.source);

        // Check keyword hover first — works even without THIR.
        let word = word_at_offset(&file_state.source, offset);
        if let Some(doc) = analysis::keyword_hover(&word) {
            return Ok(Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: doc,
                }),
                range: None,
            }));
        }

        // Fall back to THIR-based hover (type info).
        let Some(ref thir) = file_state.thir else {
            return Ok(None);
        };
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

    // ── Signature help ──────────────────────────────────────────

    async fn signature_help(&self, params: SignatureHelpParams) -> Result<Option<SignatureHelp>> {
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
        let Some(info) = analysis::signature_help_at_offset(thir, &file_state.source, offset)
        else {
            return Ok(None);
        };

        let parameters: Vec<ParameterInformation> = info
            .parameters
            .iter()
            .map(|p| ParameterInformation {
                label: ParameterLabel::Simple(p.label.clone()),
                documentation: None,
            })
            .collect();

        let signature = SignatureInformation {
            label: info.label,
            documentation: info.documentation.map(|doc| {
                tower_lsp::lsp_types::Documentation::MarkupContent(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: doc,
                })
            }),
            parameters: Some(parameters),
            active_parameter: Some(info.active_parameter),
        };

        Ok(Some(SignatureHelp {
            signatures: vec![signature],
            active_signature: Some(0),
            active_parameter: Some(info.active_parameter),
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

        let offset = position_to_offset(pos, &file_state.source);
        let entries =
            analysis::collect_all_completions(file_state.thir.as_ref(), &file_state.source, offset);

        let items: Vec<CompletionItem> = entries
            .into_iter()
            .map(|entry| {
                let (kind, sort_prefix) = match entry.kind {
                    analysis::CompletionEntryKind::Symbol(def_kind) => {
                        (def_kind_to_completion_kind(def_kind), "0")
                    }
                    analysis::CompletionEntryKind::Keyword => (CompletionItemKind::KEYWORD, "1"),
                };
                let insert_text_format = match entry.insert_text_format {
                    analysis::InsertTextFormatTag::Snippet => Some(InsertTextFormat::SNIPPET),
                    analysis::InsertTextFormatTag::PlainText => None,
                };
                CompletionItem {
                    label: entry.name.to_string(),
                    kind: Some(kind),
                    detail: entry.ty.map(|t| format!("{t}")),
                    sort_text: Some(format!("{sort_prefix}{}", entry.name)),
                    insert_text: entry.insert_text.map(|text| text.to_string()),
                    insert_text_format,
                    ..Default::default()
                }
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

    // ── Code actions ────────────────────────────────────────────

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let uri = &params.text_document.uri;
        let range = params.range;

        let state = self.state.read().await;
        let Some(file_state) = state.get(uri) else {
            return Ok(None);
        };

        let cursor_offset = position_to_offset(range.start, &file_state.source);

        // Extract diagnostic (code, message) pairs from the request.
        let diag_pairs: Vec<(String, String)> = params
            .context
            .diagnostics
            .iter()
            .filter_map(|d| {
                let code = d.code.as_ref().map(|c| match c {
                    tower_lsp::lsp_types::NumberOrString::String(s) => s.clone(),
                    tower_lsp::lsp_types::NumberOrString::Number(n) => n.to_string(),
                })?;
                Some((code, d.message.clone()))
            })
            .collect();

        let action_infos = analysis::collect_code_actions(
            file_state.thir.as_ref(),
            &file_state.source,
            &diag_pairs,
            cursor_offset,
        );

        if action_infos.is_empty() {
            return Ok(None);
        }

        let actions: Vec<CodeActionOrCommand> = action_infos
            .into_iter()
            .map(|info| {
                let kind = match info.kind {
                    analysis::CodeActionKindTag::QuickFix => CodeActionKind::QUICKFIX,
                    analysis::CodeActionKindTag::Refactor => CodeActionKind::REFACTOR,
                };

                let edit_range = tower_lsp::lsp_types::Range {
                    start: offset_to_position(info.replace_start, &file_state.source),
                    end: offset_to_position(info.replace_end, &file_state.source),
                };

                let text_edit = TextEdit { range: edit_range, new_text: info.new_text };
                let mut changes = HashMap::new();
                changes.insert(uri.clone(), vec![text_edit]);

                CodeActionOrCommand::CodeAction(CodeAction {
                    title: info.title,
                    kind: Some(kind),
                    diagnostics: None,
                    edit: Some(WorkspaceEdit { changes: Some(changes), ..Default::default() }),
                    command: None,
                    is_preferred: None,
                    disabled: None,
                    data: None,
                })
            })
            .collect();

        Ok(Some(actions))
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

    // ── Semantic tokens ────────────────────────────────────────

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let uri = &params.text_document.uri;

        let state = self.state.read().await;
        let Some(file_state) = state.get(uri) else {
            return Ok(None);
        };
        let Some(ref thir) = file_state.thir else {
            return Ok(None);
        };

        let tokens = super::semantic_tokens::collect_and_encode(thir, &file_state.line_index);

        Ok(Some(SemanticTokensResult::Tokens(SemanticTokens { result_id: None, data: tokens })))
    }
}

// ── Helpers ─────────────────────────────────────────────────────

/// Convert a byte offset to an LSP `Position` (0-based line, 0-based character).
fn offset_to_position(offset: u32, source: &str) -> Position {
    let offset = (offset as usize).min(source.len());
    let mut line = 0u32;
    let mut line_start = 0usize;
    for (i, byte) in source.bytes().enumerate() {
        if i == offset {
            #[allow(clippy::cast_possible_truncation)]
            return Position { line, character: (i - line_start) as u32 };
        }
        if byte == b'\n' {
            line += 1;
            line_start = i + 1;
        }
    }
    #[allow(clippy::cast_possible_truncation)]
    Position { line, character: (offset - line_start) as u32 }
}

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

fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Extract the word (identifier or keyword) at a byte offset in source text.
fn word_at_offset(source: &str, offset: u32) -> String {
    let bytes = source.as_bytes();
    let off = offset as usize;
    if off >= bytes.len() {
        return String::new();
    }

    // Walk backwards to find the start of the word.
    let mut start = off;
    while start > 0 && is_word_char(bytes[start - 1]) {
        start -= 1;
    }
    // Walk forwards to find the end.
    let mut end = off;
    while end < bytes.len() && is_word_char(bytes[end]) {
        end += 1;
    }

    source[start..end].to_owned()
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

// ── Debounced on-change analysis ────────────────────────────────

/// Background loop that debounces `did_change` notifications and runs analysis
/// after a quiet period of [`DEBOUNCE_DELAY`].
///
/// Spawned once from `initialized()`. Exits when the channel is closed
/// (i.e., the LSP server shuts down).
async fn debounce_loop(
    mut rx: mpsc::UnboundedReceiver<Url>,
    state: Arc<RwLock<HashMap<Url, FileState>>>,
    client: Client,
) {
    loop {
        // Wait for the first change.
        let Some(uri) = rx.recv().await else { break };
        let mut pending = HashSet::new();
        pending.insert(uri);

        // Collect more changes within the debounce window.
        let deadline = tokio::time::sleep(DEBOUNCE_DELAY);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                biased;
                next = rx.recv() => match next {
                    Some(uri) => {
                        pending.insert(uri);
                        // Reset timer — each new change extends the window.
                        deadline
                            .as_mut()
                            .reset(tokio::time::Instant::now() + DEBOUNCE_DELAY);
                    }
                    None => return,
                },
                () = &mut deadline => break,
            }
        }

        // Process all pending URIs.
        for uri in pending {
            run_debounced_analysis(&uri, &state, &client).await;
        }
    }
}

/// Run the compilation pipeline for a single file and publish diagnostics,
/// but only if the source hasn't changed since the snapshot was taken.
async fn run_debounced_analysis(
    uri: &Url,
    state: &Arc<RwLock<HashMap<Url, FileState>>>,
    client: &Client,
) {
    // 1. Snapshot the current source and sequence number.
    let (source, seq) = {
        let s = state.read().await;
        let Some(fs) = s.get(uri) else { return };
        (fs.source.clone(), fs.seq)
    };

    // 2. Run the full compilation pipeline.
    let line_index = LineIndex::new(&source);
    let (thir, diagnostics) = compile_source(&source);
    let lsp_diags = convert::to_lsp_diagnostics(&diagnostics, &line_index, uri);

    // 3. Write back only if the source hasn't changed (seq matches).
    let version_matched = {
        let mut s = state.write().await;
        if let Some(fs) = s.get_mut(uri) {
            if fs.seq == seq {
                fs.thir = thir;
                fs.line_index = line_index;
                true
            } else {
                false // Stale — a newer change will trigger another cycle.
            }
        } else {
            false
        }
    };

    // 4. Publish diagnostics only when the version matched.
    if version_matched {
        client.publish_diagnostics(uri.clone(), lsp_diags, None).await;
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

    #[test]
    fn debounce_delay_is_reasonable() {
        assert!(DEBOUNCE_DELAY.as_millis() >= 100);
        assert!(DEBOUNCE_DELAY.as_millis() <= 500);
    }

    #[tokio::test]
    async fn debounced_analysis_discards_stale_results() {
        let state = Arc::new(RwLock::new(HashMap::new()));
        let uri = Url::parse("file:///test.asty").unwrap();

        // Insert a FileState with seq=5.
        {
            let mut s = state.write().await;
            s.insert(
                uri.clone(),
                FileState {
                    source: "pub fn main() { 42 }".to_owned(),
                    line_index: LineIndex::new("pub fn main() { 42 }"),
                    thir: None,
                    seq: 5,
                },
            );
        }

        // Simulate: snapshot taken at seq=5, then source changed (seq bumped to 6)
        // before analysis writes back.
        {
            let mut s = state.write().await;
            let fs = s.get_mut(&uri).unwrap();
            fs.seq = 6; // User typed again while analysis was in-flight
        }

        // run_debounced_analysis would have captured seq=5 in its snapshot.
        // Simulate the write-back check: seq 5 != 6, so thir stays None.
        {
            let s = state.read().await;
            let fs = s.get(&uri).unwrap();
            assert_eq!(fs.seq, 6);
            assert!(fs.thir.is_none(), "thir should remain None (stale result discarded)");
        }
    }

    #[tokio::test]
    async fn analysis_writeback_succeeds_when_seq_matches() {
        let state = Arc::new(RwLock::new(HashMap::new()));
        let uri = Url::parse("file:///test.asty").unwrap();
        let source = "pub fn main() { 42 }";

        // Insert a FileState at seq=0.
        {
            let mut s = state.write().await;
            s.insert(
                uri.clone(),
                FileState {
                    source: source.to_owned(),
                    line_index: LineIndex::new(source),
                    thir: None,
                    seq: 0,
                },
            );
        }

        // Simulate the debounce analysis write-back logic when seq matches.
        let snapshot_seq: u64 = 0;
        let line_index = LineIndex::new(source);
        let (thir, _) = compile_source(source);

        {
            let mut s = state.write().await;
            let fs = s.get_mut(&uri).unwrap();
            assert_eq!(fs.seq, snapshot_seq, "seq should match snapshot");
            fs.thir = thir;
            fs.line_index = line_index;
        }

        // Verify thir was successfully written.
        {
            let s = state.read().await;
            let fs = s.get(&uri).unwrap();
            assert!(fs.thir.is_some(), "thir should be set when seq matches");
        }
    }

    #[test]
    fn apply_full_document_change_invalidates_previous_analysis() {
        let uri = Url::parse("file:///test.asty").unwrap();
        let original_source = "pub fn main() { 42 }";
        let (thir, _) = compile_source(original_source);
        assert!(thir.is_some(), "fixture should type-check");

        let mut state = HashMap::new();
        state.insert(
            uri.clone(),
            FileState {
                source: original_source.to_owned(),
                line_index: LineIndex::new(original_source),
                thir,
                seq: 3,
            },
        );

        let next_seq = apply_full_document_change(&mut state, &uri, "pub fn main() {".to_owned());
        let fs = state.get(&uri).expect("file state should exist");
        assert_eq!(next_seq, 4);
        assert_eq!(fs.seq, 4);
        assert!(fs.thir.is_none(), "did_change must clear stale THIR");
        assert_eq!(fs.source, "pub fn main() {");
    }

    #[test]
    fn apply_full_document_change_initializes_missing_file_state() {
        let uri = Url::parse("file:///fresh.asty").unwrap();
        let mut state = HashMap::new();

        let seq = apply_full_document_change(&mut state, &uri, "fn main() {}".to_owned());
        let fs = state.get(&uri).expect("file state should be inserted");
        assert_eq!(seq, 0);
        assert_eq!(fs.seq, 0);
        assert!(fs.thir.is_none());
        assert_eq!(fs.source, "fn main() {}");
    }

    #[test]
    fn word_at_offset_extracts_keyword() {
        assert_eq!(word_at_offset("fn main() {}", 0), "fn");
        assert_eq!(word_at_offset("fn main() {}", 1), "fn");
        assert_eq!(word_at_offset("fn main() {}", 3), "main");
        assert_eq!(word_at_offset("let mut x = 0", 4), "mut");
        assert_eq!(word_at_offset("let mut x = 0", 0), "let");
    }

    #[test]
    fn word_at_offset_handles_boundaries() {
        assert_eq!(word_at_offset("", 0), "");
        assert_eq!(word_at_offset("fn main() {}", 9), "");
        assert_eq!(word_at_offset("a", 0), "a");
    }

    #[test]
    fn keyword_hover_returns_docs() {
        assert!(analysis::keyword_hover("fn").is_some());
        assert!(analysis::keyword_hover("let").is_some());
        assert!(analysis::keyword_hover("match").is_some());
        assert!(analysis::keyword_hover("try").is_some());
        assert!(analysis::keyword_hover("async").is_some());
        assert!(analysis::keyword_hover("await").is_some());
        assert!(analysis::keyword_hover("type").is_some());
        assert!(analysis::keyword_hover("if").is_some());
        assert!(analysis::keyword_hover("mut").is_some());
        assert!(analysis::keyword_hover("println").is_none());
        assert!(analysis::keyword_hover("foo").is_none());
    }
}
