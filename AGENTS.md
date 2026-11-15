# AGENTS Playbook

Guardrails for AI coding agents working in this repository — in particular delegated workers
(Antigravity CLI via `agy-orch-mcp`, Codex) that do not load `CLAUDE.md` automatically.
Root `CLAUDE.md` stays the canonical project context; this file repeats only what a worker needs
to act safely on its own.

## Mission & Scope

- Asatsuyu is a statically typed functional language that compiles to Python 3.12+ source. The compiler is written in Rust.
- The language charter, `docs/concepts/principles.md`, is the single top-level authority. A change that contradicts it is rejected, however attractive. Decisions that interpret or revise it live in `docs/adr/`.
- `docs/research/` is git-ignored exploration, not a contract. Do not cite its vocabulary as current specification.

## Repository Map

- `asatsuyu/` — the Cargo workspace. All Rust code lives under `asatsuyu/crates/`.
  - `asatsuyu-syntax` — shared definitions (`SyntaxKind`, `Span`, `Diagnostic`). No external dependencies.
  - `asatsuyu-lexer` → `asatsuyu-parser` (lossless CST on rowan, formatter in `format/`) → `asatsuyu-ast` → `asatsuyu-hir` (name resolution, desugaring, FFI, purity) → `asatsuyu-ty` (Hindley-Milner, exhaustiveness) → `asatsuyu-backend-python` → `asatsuyu-cli` (subcommands and the LSP server in `lsp/`).
  - `asatsuyu-runtime-python` — PyO3 boundary layer for Checked FFI, independent of the compiler crates.
  - `examples/`, `fixtures/projects/` — sample sources and executable fixture projects.
- `docs/` — charter, ADRs, architecture, roadmap, grammar, FFI guides.
- `editors/vscode/` — VS Code extension (TypeScript, pnpm, esbuild).
- `tests/` — pytest suite for the runtime and FFI. `scripts/` — docs-sync and FFI verification.

## Commands

The Cargo workspace is **not** at the repository root. Run one simple command per call; do not chain with `cd … &&`.
From the repository root, pass the manifest path:

```bash
cargo fmt --all --manifest-path asatsuyu/Cargo.toml
cargo clippy --manifest-path asatsuyu/Cargo.toml --workspace --all-targets -- -D warnings
cargo test --manifest-path asatsuyu/Cargo.toml --workspace
cargo test --manifest-path asatsuyu/Cargo.toml -p asatsuyu-parser
cargo run -q --manifest-path asatsuyu/Cargo.toml -p asatsuyu-cli -- check asatsuyu/examples/hello.asty
python3 scripts/check_docs_sync.py
```

- Snapshot tests use `insta`. CI runs with `INSTA_UPDATE=no`, so a pending snapshot fails the build. Read every `*.snap.new` before running `cargo insta accept`; `cargo insta review` is interactive and will hang a headless session.
- Python tests (`pytest tests/`) need the PyO3 runtime installed into the active virtualenv first (`maturin develop` in `asatsuyu/crates/asatsuyu-runtime-python`).
- VS Code extension: `pnpm run check-types` and `pnpm test` inside `editors/vscode/`.

## Hard Rules

1. **Verify before reporting done** — fmt, then clippy with `-D warnings`, then the full test suite, in that order. If you touched diagnostic codes, FFI classification, fixture projects, CLI flags or `docs/`, also run `python3 scripts/check_docs_sync.py`.
2. **Never panic on user input** — failures caused by `.asty`, `.pyi` or `asatsuyu.toml` content become a `Diagnostic` with code, span, labels and hints. The parser recovers and returns a partial CST plus diagnostics.
3. **Keep the CST lossless** — whitespace, comments and newlines are trivia tokens and must be preserved.
4. **One-way dependencies, nine crates** — never make an upstream crate depend on a downstream one. Do not add a crate or a new dependency without the user's approval; stop and report instead.
5. **No `unsafe`** — the workspace denies it. The only exceptions already exist (`asatsuyu-runtime-python`, and the `SyntaxKind` conversion in `asatsuyu-syntax`).
6. **Do not silence lints or tests** — no new `#[allow(...)]`, no `#[ignore]`, no blind snapshot acceptance to get to green.
7. **Never hand-edit generated Python** — fix the compiler.
8. **Library crates do not print** — no `println!`, `eprintln!` or `std::process::exit` outside `asatsuyu-cli`.
9. **The grammar is frozen** — `docs/grammar.md` is the syntax authority. Do not add or change syntax unless the task explicitly says so.
10. **No issue numbers or phase names in code, comments or docstrings.** Commit messages, code comments and docstrings are in English.

## Conventions Worth Reading

- `docs/best_practices/rust.md` — Rust implementation rules (visibility, errors vs diagnostics, ownership, output policy).
- `.claude/skills/asatsuyu-compiler-dev/SKILL.md` — compiler design rules (stage responsibilities, parser recovery, diagnostics, tests).
- `.claude/skills/tdd-workflow/SKILL.md` — which test harness to write first for each kind of change.
- `docs/architecture.md` — crate layout and data flow. Large; read only the section you need.

## Git

- Never run `git push`, `gh pr create`, `gh pr merge`, `gh workflow run`, `gh run rerun` or `gh run cancel`. Report that the work is ready instead.
- Commit only when the task says so. Do not commit per step; the one exception is test-first work, where the failing tests and the implementation go in two separate commits.
- Never use `--no-verify` or skip signing.

## Delegation Policy

- **Host orchestrator** (Claude Code, Codex): decomposes work into task packets sized for a resumable turn of at most 15 minutes, then reviews `git diff` and the machine-extracted `verified_outputs` ledger rather than the worker's prose.
- **Worker** (Antigravity CLI): explores, edits and tests directly inside the workspace it was given.
- **No recursive delegation** — a worker completes its task with native tools. It must not invoke `agy-orch-mcp` or delegate back to the host.
- A task packet states the **goal**, the **file scope** (what to edit, what to leave alone), the **constraints and acceptance commands**, and the **expected handoff**.
- When a command is denied, do not look for a workaround. Report the denied command and what it was needed for.

## Handoff Checklist

- Files changed, with the reason for each.
- Commands run and their results — fmt, clippy, tests, and docs-sync when applicable.
- Pending snapshots (`cargo insta pending-snapshots`): none, or each one listed with what changed.
- Anything skipped, blocked or left undecided, stated plainly.
