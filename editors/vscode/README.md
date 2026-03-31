# Asatsuyu VS Code Extension

Minimal VS Code extension for the Asatsuyu programming language.

## Features

- **Diagnostics**: Type errors and warnings displayed on save
- **Hover**: Show type information for variables and functions
- **Go to Definition**: Navigate to where a name is defined
- **Document Formatting**: Format `.asty` files on save
- **Completion**: Name completion with type information
- **Rename**: Project-wide rename (F2)
- **Find References**: Find all references to a symbol
- **Document Symbols**: Outline view of functions and types (Ctrl+Shift+O)

## Setup

### Prerequisites

1. Build the Asatsuyu compiler:
   ```bash
   cd asatsuyu && cargo build --release
   ```
2. Ensure `asatsuyu` is in your PATH, or configure `asatsuyu.serverPath` in VS Code settings.
3. Enable `editor.formatOnSave` for `.asty` files if you want save-time formatting.

To make `fn` blue and function definition names green in your own setup, add this to VS Code settings:

```json
"editor.tokenColorCustomizations": {
  "textMateRules": [
    {
      "scope": "keyword.control.function.asatsuyu",
      "settings": {
        "foreground": "#4A90E2"
      }
    },
    {
      "scope": "entity.name.function.asatsuyu",
      "settings": {
        "foreground": "#2E8B57"
      }
    }
  ]
}
```

### Install the extension

```bash
cd editors/vscode
npm install
npm run compile
```

Then open VS Code and run "Developer: Install Extension from Location..." pointing to the `editors/vscode` directory.

### Neovim

For Neovim, add to your LSP configuration:

```lua
vim.lsp.start({
  name = "asatsuyu",
  cmd = { "asatsuyu", "lsp" },
  filetypes = { "asatsuyu" },
  root_dir = vim.fs.dirname(vim.fs.find("asatsuyu.toml", { upward = true })[1]),
})
```

Or with `nvim-lspconfig`, register a custom server config.
