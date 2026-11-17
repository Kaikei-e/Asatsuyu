---
name: asatsuyu-compiler-dev
description: >-
  Asatsuyu コンパイラ（Rust 実装）の開発規約。asatsuyu/crates/ 以下の Rust コードを
  書く・変更する・レビューするときは必ずこのスキルを使うこと。
  lexer、parser、CST、AST lowering、HIR、型検査、Python emitter、診断（diagnostics）、
  フォーマッタ、LSP、テスト追加のいずれに触れる場合も適用する。「パーサを直して」
  「emitter を実装して」「型検査を追加して」などのフレーズでも必ず使用すること。
  Rust 一般の書き方（可視性・エラー型・所有権）は bp-rust、テストを書く順序は tdd-workflow が担当する。
paths:
  - "asatsuyu/crates/**/*.rs"
  - "asatsuyu/**/Cargo.toml"
---

# Asatsuyu Compiler Development Conventions

クレート構成とデータフローの詳細は `docs/architecture.md`（大きいので必要な節だけ Read する）。

## アーキテクチャ原則

- パイプラインは `lexer → parser (CST) → ast → hir → ty (THIR) → backend-python → cli` の一方向。
  下流のクレートを上流から参照しない。各段の責務を混ぜない。
- CST は lossless（rowan）。空白・コメント・改行は trivia トークンとして完全に保持する。
  フォーマッタ（`asatsuyu-parser::format`）と LSP（`asatsuyu-cli::lsp`）はこの層に依存している。
- **パーサは決して panic しない。** 不正入力はすべてエラーリカバリで吸収し、部分的な CST + 診断を返す。
  リカバリは rust-analyzer 流の `TokenSet`（follow set ベース）方式を維持する。
- **ユーザ入力・ファイル内容では panic しない。** 入力由来の失敗は `Diagnostic` で表現する。
  `expect` は「落ちたらコンパイラのバグ」と言える箇所に限る。新しく panic 経路を見つけたら
  `crates/asatsuyu-cli/tests/corpus/crash-<stage>-<desc>.asty` に入力を足して固定する。
- 脱糖（パイプライン、文字列連結など）は HIR までに終える。型推論は構文糖衣を知らない（憲章 §6.6）。
- 純粋性は HIR の呼び出しグラフ上の不動点 + opt-in の `pure` 宣言で扱う（ADR 0002、`asatsuyu-hir/src/purity.rs`）。
  関数型に効果を載せない — type-and-effect system は永久 Non-Goal（ADR 0001）。
- IR（AST / HIR / THIR）は immutable。HIR のコレクションは `la-arena`、識別子は `SmolStr`。
- `unsafe` は書かない。既存の例外は `asatsuyu-syntax` の `SyntaxKind` 変換と `asatsuyu-runtime-python` だけ。
- 新しいクレートを作らない。フォーマッタも LSP も既存クレートのモジュールとして入っている。

## 診断（diagnostics）

- 全クレートは `Vec<Diagnostic>` を返す。表示は CLI 層の `miette` が担当し、解析クレートは端末に出力しない。
- 診断は code・span・labels・hints・notes を持つ。メッセージだけの診断を足さない。
- エラーメッセージは「何が悪いか」だけでなく「どう直すか」を示す（Elm / Rust コンパイラのエラー文化に倣う）。
  型不一致は expected / actual を必ず出す（憲章 §7）。
- `DiagnosticCode` は段ごとに範囲が割り当てられている（`asatsuyu-syntax/src/diagnostic.rs` 冒頭のコメント）。
  新しいコードは該当範囲から採り、`make docs-sync` で整合を確認する。
- 新しい診断を足すときは、それを実際に発生させる fixture
  （`crates/asatsuyu-cli/tests/fixtures/diag/<stage>-<desc>.asty`）を同じ変更に含める。

## テスト規約

- 置き場所と書く順序は tdd-workflow に従う。golden case（`tests/cases/<name>/input.asty`）は
  CST / AST / HIR / THIR / 生成 Python / 診断を段ごとに snapshot する。
- lexer / parser の変更には、正常系だけでなくエラーリカバリの結果（壊れた入力 → 部分 CST + 診断）も含める。
- snapshot は Read で中身を確認してから `cargo insta accept`。CI は `INSTA_UPDATE=no` で走る。
- 既存のテスト記法・ディレクトリ構成に従う。新しいテストフレームワークやマクロを勝手に導入しない
  （導入したい場合は先に提案し、採用するなら asatsuyu-adr-writer で記録）。
- 生成 Python の snapshot は人間がレビューする前提。diff が読みやすい粒度でケースを分ける。

## 実装スタイル

- 縦に薄く貫通させることを優先する。「この段を完璧にしてから次へ」という進め方を提案しない
  （asatsuyu-slice-planner の方針に従う）。
- 新しい構文は波及先が多い: grammar（`docs/grammar.md` は凍結されており変更には合意が要る）、
  フォーマッタ、LSP の semantic tokens、VS Code 拡張の tmLanguage とスニペット（`editors/vscode/`）。
  どこまでを今回の変更に含めるかを最初に決める。
- 新しい依存クレートの追加は保守的に。追加する場合は必ず理由を述べ、ユーザーの了承を得る。
- パフォーマンス最適化は計測してから。推測に基づく最適化のための複雑化はしない。
- コード・コメント・docstring に issue 番号や Phase / Milestone 名を書かない。

## 変更後の確認

コード変更後は必ず以下を **`asatsuyu/` で**、この順に実行し、結果を報告する:

```bash
cd asatsuyu && cargo fmt --all
cd asatsuyu && cargo clippy --workspace --all-targets -- -D warnings
cd asatsuyu && cargo test --workspace
```

診断コード・FFI 分類・fixture・CLI フラグ・`docs/` に触れた場合は、リポジトリルートで `make docs-sync` も通す。
