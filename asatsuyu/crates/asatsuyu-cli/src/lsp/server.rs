//! Tower-LSP server implementation for Asatsuyu.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use asatsuyu_hir::ffi::FfiResolverConfig;
use asatsuyu_syntax::{FileId, LineIndex};
use asatsuyu_ty::ThirModule;
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DidSaveTextDocumentParams, DocumentFormattingParams, GotoDefinitionParams,
    GotoDefinitionResponse, Hover, HoverContents, HoverParams, HoverProviderCapability,
    InitializeParams, InitializeResult, InitializedParams, Location, MarkupContent, MarkupKind,
    MessageType, OneOf, Position, Range, ServerCapabilities, ServerInfo,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit, Url,
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

        let lsp_diags = convert::to_lsp_diagnostics(&diagnostics, &line_index);

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
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                document_formatting_provider: Some(OneOf::Left(true)),
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

        // Replace the entire document.
        #[allow(clippy::cast_possible_truncation)]
        let line_count = file_state.source.lines().count() as u32;
        #[allow(clippy::cast_possible_truncation)]
        let last_line_len = file_state.source.lines().last().map_or(0, str::len) as u32;

        Ok(Some(vec![TextEdit {
            range: Range {
                start: Position { line: 0, character: 0 },
                end: Position { line: line_count, character: last_line_len },
            },
            new_text: result.formatted,
        }]))
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

// ── Server startup ──────────────────────────────────────────────

pub(super) async fn run_server() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
