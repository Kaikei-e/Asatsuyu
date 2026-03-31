# Rust Best Practices — Asatsuyu

`docs/best_practices/rust.md` は Asatsuyu の Rust 実装向け規約である。
対象は **Python フロントエンド言語のコンパイラ** と CLI である。設計上の前提は [`core_concept.md`](../core_concept.md) と
[`IMPL_PHASES.md`](../IMPL_PHASES.md) に従う。

## 1. Scope

- Asatsuyu は **型安全な Python フロントエンド** を Rust で実装する
- ワークスペースは `lexer -> parser -> ast -> hir -> ty -> backend-python` の
  一方向パイプラインを保つ
- `asatsuyu-cli` はユーザー入口、`asatsuyu-syntax` は最下層共有クレートとして扱う
- コンパイラ本体と CLI に不要な責務を持ち込まない
- 実装順は縦切り優先とし、最初の目標を `hello.asty -> hello.py -> 実行` に置く
- MVP では LSP / formatter / 多バックエンド化を前提に設計を複雑化しない

## 2. Edition And Workspace

- `edition = "2024"` を使う
- 共通依存と lint は workspace で一元管理する
- crate ごとの責務を狭く保ち、`lib.rs` は公開境界、`main.rs` は薄い入口にする
- `pub(crate)` を基本とし、公開 API だけを `pub` にする

```toml
[workspace.package]
edition = "2024"

[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
pedantic = { level = "warn", priority = -1 }
module_name_repetitions = "allow"
missing_panics_doc = "allow"
missing_errors_doc = "allow"
```

```rust
// crates/asatsuyu-cli/src/main.rs
fn main() -> std::process::ExitCode {
    asatsuyu_cli::run()
}
```

## 3. Crate Boundaries

- `asatsuyu-syntax`: token kind, span, file id, diagnostic などの共有定義
- `asatsuyu-lexer`: 字句解析だけに集中し、構文木や意味解析を持ち込まない
- `asatsuyu-parser`: ロスレス CST の構築に集中する
- `asatsuyu-ast`: CST から意味のある AST へ整形する
- `asatsuyu-hir`: 名前解決と脱糖
- `asatsuyu-ty`: 型推論と型検査
- `asatsuyu-backend-python`: THIR から Python 3.12+ 生成
- `asatsuyu-cli`: 入出力、診断整形、サブコマンド制御

```rust
// ✅ 依存方向は一方向に保つ
asatsuyu_cli -> asatsuyu_ty -> asatsuyu_hir -> asatsuyu_ast -> asatsuyu_parser
```

- 下層クレートから上層クレートへ逆依存しない
- `cli` だけが端末表示や exit code を知る
- 解析系クレートは `std::process::exit`, `println!`, `eprintln!` を呼ばない

## 4. Data And Ownership

- 構文・型情報は値オブジェクトとして渡し、副作用を局所化する
- `Span`, `FileId`, `TextRange` のような source mapping を最初から保持する
- AST/HIR/THIR 間の変換では「何を捨て、何を保持するか」を明示する
- `String` を乱用せず、識別子やパスには専用型を検討する

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub file_id: FileId,
    pub start: u32,
    pub end: u32,
}
```

## 5. Errors And Diagnostics

- 内部エラー型とユーザー向け診断を分ける
- crate 内部では `thiserror` の domain error を使い、最終表示は CLI で `miette` 等へ変換する
- `panic!` はバグのみ。ユーザー入力エラーは必ず `Result` / `Diagnostic` に乗せる
- 診断には message だけでなく `code`, `span`, `labels`, `hints`, `notes` を持たせる

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("unexpected token")]
    UnexpectedToken { span: Span },
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub code: &'static str,
    pub message: String,
    pub span: Span,
    pub hints: Vec<String>,
}
```

```rust
// ❌ ライブラリ層で直接終了しない
if has_errors {
    std::process::exit(1);
}
```

## 6. API Design

- public API は crate の責務をそのまま表す名前にする
- `parse`, `lower_to_hir`, `infer_types`, `emit_python` のように段階が分かる関数名を使う
- 便利関数で段階を潰しすぎない。デバッグ可能性を優先する
- builder が不要なら素直な関数を選ぶ

```rust
pub fn parse(file_id: FileId, source: &str) -> ParseResult<Cst>;
pub fn lower(cst: &Cst) -> Ast;
pub fn lower_to_hir(ast: &Ast, db: &mut HirDb) -> Result<Hir, HirError>;
pub fn infer_program(hir: &Hir, db: &mut TyDb) -> Result<Thir, TyError>;
pub fn emit_module(thir: &Thir) -> String;
```

## 7. Pattern Matching And Enums

- 固定集合の分岐は enum を優先する
- 文字列ベースの kind 判定を避ける
- `match` は網羅的に書き、`_` で情報を捨てすぎない
- small object polymorphism より enum + exhaustive match を優先する

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Let,
    Fn,
    Type,
    Ident,
}

match token.kind {
    TokenKind::Let => parse_let(p),
    TokenKind::Fn => parse_fn(p),
    TokenKind::Type => parse_type(p),
    TokenKind::Ident => parse_expr_stmt(p),
}
```

## 8. Async And Concurrency

- MVP のコンパイラ本体は同期処理を基本にする
- `async` は外部 I/O が必要な境界でのみ使う
- CPU-bound な解析処理を理由なく `tokio` に載せない
- 並列化は profiling で必要性が出てから導入する

```rust
// ✅ まずは同期 API を基本にする
pub fn check_file(path: &Path) -> Result<Vec<Diagnostic>, CliError> {
    let source = std::fs::read_to_string(path)?;
    let cst = asatsuyu_parser::parse(FileId(0), &source)?;
    // ...
    Ok(vec![])
}
```

## 9. User Output, Logging And Tracing

- `println!` を一律禁止しない
- **CLI の正規出力** には `stdout` を使う。成功結果、生成コード、機械可読 JSON などが対象
- **診断・警告・進捗** は `stderr` を使う。`miette` の表示や `eprintln!` をここに出す
- **内部観測・デバッグイベント** は `tracing` を使う
- ライブラリ層はユーザー向け出力をしない。出力ポリシーは `asatsuyu-cli` に集約する
- `build`, `run`, `check`, `fmt`, `test` の各コマンドで stdout/stderr の契約を固定する

```rust
// ✅ CLI のユーザー向け結果
println!("{python_source}");

// ✅ CLI の診断やエラー
eprintln!("{report:?}");

// ✅ 内部観測
tracing::debug!(tokens = tokens.len(), "lex finished");
```

```rust
// ❌ ライブラリ層で端末出力しない
pub fn infer_program(hir: &Hir) -> Result<Thir, TyError> {
    println!("type inference started");
    todo!()
}
```

```rust
// ✅ CLI 側で subscriber を配線する
pub fn init_tracing() {
    use tracing_subscriber::{EnvFilter, fmt};

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("warn,asatsuyu=info"));

    fmt().with_env_filter(filter).without_time().init();
}
```

## 10. CLI Conventions

- `asatsuyu-cli` は subcommand ごとに exit code 契約を持つ
- 成功時の人間向けメッセージは簡潔にする
- `--json` を導入する場合、人間向け出力と混在させない
- エラー件数がある場合は集約してから表示する
- 色や装飾は CLI 境界に閉じ込める
- 最初の重要コマンドは `check`, `build`, `run`。規約もこの3つを優先して守る

出力の基本契約:

- `check`: 成功時は静かに終了するか簡潔な成功表示、失敗時の診断は `stderr`
- `build`: 生成物パスなどの結果は `stdout`、進捗や警告は `stderr`
- `run`: コンパイル系の診断は `stderr`、生成された Python プログラムの標準出力はそのまま `stdout`

```rust
pub fn run() -> std::process::ExitCode {
    match try_run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            std::process::ExitCode::FAILURE
        }
    }
}
```

## 11. Testing

- unit test は各 crate に近接配置する
- parser / formatter / backend では snapshot test を積極的に使う
- lexer/parser は golden test で token 列や tree 形状を固定する
- type checker は診断コードと span を含めて検証する
- CLI は integration test で stdout, stderr, exit code を分けて確認する
- `IMPL_PHASES.md` の DoD に沿って e2e test を追加する

```rust
#[test]
fn parses_let_binding() {
    let cst = parse(FileId(0), "let x = 1");
    assert!(cst.is_ok());
}
```

```rust
#[test]
fn check_command_writes_diagnostics_to_stderr() {
    let output = run_cli(["check", "examples/type_error.asty"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
}
```

## 12. Performance

- 最適化前に `cargo bench`, `criterion`, `hyperfine` などで測る
- lexer / parser では不要な allocation を避ける
- hot path で `clone()` を雑に増やさない
- 文字列連結より span + source slice を優先する
- 巨大入力を見据えて incremental / arena / interning を検討するが、早すぎる抽象化は避ける

## 13. Lints And Formatting

- `cargo fmt`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` を CI 基本線にする
- pedantic lint は有効化しつつ、ノイズは workspace で明示的に `allow` する
- `unwrap()` は test / prototype 以外で漫然と使わない
- 新しい lint 例外はコードではなく設定に集約する

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## 14. Unsafe, Panics And Stability

- `unsafe` は原則禁止。必要なら `// SAFETY:` を付ける
- `expect` は「ここが壊れたらバグ」という文脈でのみ使う
- ユーザー入力やファイル内容では panic しない
- public API の error surface は安定させる

## 15. Dependency Policy

- 新規依存は「この crate が必要な理由」を説明できるものだけ入れる
- 解析基盤では軽量性とデバッグ可能性を優先する
- `logos`, `rowan`, `clap`, `miette`, `smol_str`, `la-arena`, `insta` など、
  `IMPL_PHASES.md` で採用済みの依存を基準に増減を判断する
- `syn`, `serde`, `tokio` のような大型依存は必要性を確認してから導入する
- feature flag は利用者の理解コストを増やすので最小限にする

## 16. Review Checklist

- crate の責務は増えすぎていないか
- 下層 crate が CLI / 表示責務を持っていないか
- 診断に span, code, hint が含まれているか
- `stdout` と `stderr` が混ざっていないか
- `tracing` は内部観測だけに使い、ユーザー向け文面と混同していないか
- `core_concept.md` のパイプラインと依存方向に反していないか
- `IMPL_PHASES.md` の現在の milestone / DoD を不必要に遠回りしていないか

## References

- Asatsuyu 設計: [`core_concept.md`](../core_concept.md)
- 実装計画: [`IMPL_PHASES.md`](../IMPL_PHASES.md)
- Rust Book: stdout/stderr の使い分け
- Rust std: `println!`, `eprintln!`
- `tracing` / `tracing-subscriber`
- `miette`
