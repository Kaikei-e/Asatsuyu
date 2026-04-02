//! Language Server Protocol implementation for the Asatsuyu language.
//!
//! Runs as `asatsuyu lsp` (stdio transport) and provides:
//! - Diagnostics on save/open
//! - Hover (type information)
//! - Go to definition
//! - Document formatting

mod analysis;
mod convert;
mod semantic_tokens;
mod server;

/// Start the LSP server on stdio.
///
/// This blocks until the client disconnects.
pub(crate) fn start_lsp() {
    let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
    rt.block_on(server::run_server());
}
