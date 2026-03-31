# Asatsuyu

## Commands

Rust ワークスペースは `asatsuyu/` サブディレクトリにある。全 cargo コマンドはそこで実行すること。

```bash
# Format
cd asatsuyu && cargo fmt --all

# Lint (warnings = error)
cd asatsuyu && cargo clippy --workspace --all-targets -- -D warnings

# Test
cd asatsuyu && cargo test --workspace

# Format check (CI と同じ)
cd asatsuyu && cargo fmt --all --check

# 単一クレートのテスト
cd asatsuyu && cargo test -p asatsuyu-parser
```

実装タスク完了前に必ず fmt → clippy → test の順で全パスを確認すること。

## What This Is

Asatsuyu は **Python 3.12+ ソースコードにコンパイルする静的型付き言語**。TypeScript:JavaScript の関係と同様に、Python エコシステムの資産を活かしつつ型安全性を提供する。Rust で実装されたコンパイラ。

## Architecture

5段パイプライン、9クレートの一方向依存。逆方向の依存は禁止。

```
Source (.asty)
  → asatsuyu-lexer           logos DFA による字句解析
  → asatsuyu-parser          手書き再帰下降、ロスレス CST (rowan)
  → asatsuyu-ast             CST → 型なし AST
  → asatsuyu-hir             名前解決・脱糖
  → asatsuyu-ty              Hindley-Milner 型推論
  → asatsuyu-backend-python  THIR → Python 3.12+ ソース生成
```

- `asatsuyu-syntax` — 全クレート共有の最下層（SyntaxKind, Span, Diagnostic）。外部依存ゼロ
- `asatsuyu-cli` — ユーザー入口。薄いラッパー（`check`, `build`, `run`, `fmt`, `new` 等）
- `asatsuyu-parser::format` — CST ベースの決定的フォーマッタ（設定なし、Gleam 方式）
- `asatsuyu-runtime-python` — Checked FFI 用の PyO3 ランタイム境界層。コンパイラクレートとは独立

## Things That Will Bite You

### Working directory
プロジェクトルートは `Asatsuyu/` だが、Cargo workspace は `Asatsuyu/asatsuyu/` にある。
`cargo` コマンドをプロジェクトルートで実行すると失敗する。

### unsafe は禁止
`unsafe_code = "deny"` がワークスペース全体に設定済み。unsafe は絶対に書かないこと。
唯一の例外は `asatsuyu-runtime-python`（PyO3 マクロが内部で unsafe を生成するため crate レベルで allow）。

### clippy pedantic が有効
`clippy::all` + `clippy::pedantic` が warn。以下のみ allow:
- `module_name_repetitions`
- `missing_panics_doc`
- `missing_errors_doc`

### ロスレス CST
パーサーは rowan ベースのロスレス CST を構築する。空白・コメント・改行はすべて trivia トークンとして保持される。スキップしてはならない。

### 単一 SyntaxKind enum
トークンとノードの両方を単一の `#[repr(u16)] SyntaxKind` enum で表現する（rust-analyzer パターン）。`is_token()` / `is_node()` ヘルパーで分類。

### スナップショットテスト
`insta` クレートによるスナップショットテストを多用。新しいテストケース追加後は `cargo insta review` でスナップショットを確認・承認する。

### IR は immutable
AST / HIR / THIR の全ノードは不変。`Rc`/`Arc` は必要最小限に留める。Arena 確保 (`la-arena`) を HIR コレクションに使用。

## Code Conventions

### Visibility
- `pub(crate)` がデフォルト。公開 API のみ `pub`
- `lib.rs` は公開境界、`main.rs` は薄い入口

### Rust edition
- `edition = "2024"`

### Diagnostics
- 全クレートは `Vec<Diagnostic>` を生成する
- エラーコード・スパン・ラベル・ヒントを含む構造化診断
- 表示は `miette` が担当（CLI 層で集約）

### String interning
- 識別子には `smol_str::SmolStr` を使用

### 新しいクレートは作らない
- 既存の 9 クレート構成を維持する。新クレート追加は設計議論が必要

## Language Design Decisions

Asatsuyu 言語の設計方針。コンパイラ実装時にこれらを前提とすること。

- **ADT**: Gleam 風の代数的データ型（名前付きコンストラクタ）
- **Record**: Go 風のフラットな構造体
- **null なし**: `Option` で表現
- **例外なし**: `Result` で表現。Asatsuyu コード内では例外は発生しない
- **mutable 変数なし**: 意図的な設計判断。再束縛もなし
- **パイプライン**: `|>` 演算子あり
- **Python 3.12+ 固定**: バックエンドは Python ソース生成のみ。バイトコードや他言語は対象外

## References

詳細は以下を参照:

- @docs/concepts/principles.md — 言語憲章（5つの中核原則、11の設計判断）
- @docs/best_practices/rust.md — Rust 実装規約（クレート境界、エラー処理、テスト方針）
- @core_concept.md — アーキテクチャ詳細、FFI モデル（Verified / Checked / Opaque）
- `IMPL_PHASES.md` — 実装フェーズとマイルストーン（完了済み・進行中・未着手）。大きいファイルなので必要時のみ参照すること
