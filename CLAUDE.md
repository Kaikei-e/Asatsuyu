# Asatsuyu

## WHAT

Asatsuyu は **Python 3.12+ ソースコードにコンパイルする静的型付き関数型言語**。TypeScript:JavaScript の関係と同様に、
Python エコシステムの資産を活かしつつ型安全性を提供する。コンパイラは Rust 実装。

5 段パイプライン、9 クレートの一方向依存。逆方向の依存は禁止。

```
Source (.asty)
  → asatsuyu-lexer           logos DFA による字句解析
  → asatsuyu-parser          手書き再帰下降、ロスレス CST (rowan)
  → asatsuyu-ast             CST → 型なし AST
  → asatsuyu-hir             名前解決・脱糖・FFI 解決・純粋性解析
  → asatsuyu-ty              Hindley-Milner 型推論、網羅性検査 (THIR)
  → asatsuyu-backend-python  THIR → Python 3.12+ ソース生成
  → asatsuyu-cli             check / build / run / fmt / new / lock / add / remove / sync / lsp / verify-ffi
```

- `asatsuyu-syntax` — 全クレート共有の最下層（SyntaxKind, Span, Diagnostic）。外部依存ゼロ
- `asatsuyu-parser::format` — CST ベースの決定的フォーマッタ（設定なし、Gleam 方式）
- `asatsuyu-cli::lsp` — LSP サーバー（tower-lsp）。`asatsuyu lsp` で起動
- `asatsuyu-runtime-python` — Checked FFI 用の PyO3 ランタイム境界層。コンパイラクレートとは独立

リポジトリの配置:

| パス | 中身 |
|---|---|
| `asatsuyu/` | **Cargo workspace**（`crates/`, `examples/`, `fixtures/projects/`, `vendor/typeshed`） |
| `docs/` | 憲章・ADR・アーキテクチャ・roadmap・grammar・FFI ガイド |
| `editors/vscode/` | VS Code 拡張（TypeScript, pnpm, esbuild） |
| `tests/` | pytest（PyO3 ランタイムと FFI の検証） |
| `scripts/`, `Makefile` | docs-sync、FFI 検証、パッケージ導入テスト、CI 相当の集約ターゲット |

## WHY

- **憲章が唯一の上位権威** — `docs/concepts/principles.md`。構文・型システム・FFI・コード生成・ツーリングの
  すべての判断はここに従う。解釈・改訂は `docs/adr/` に記録し、憲章本文を同じ変更で直す。
- **評価軸の順序は固定** — 型エラーメッセージの質 → 生成コードの可読性 → Python 資産を使う際の摩擦の低さ。
  速度は評価軸ではない。
- **縦に薄く貫通させる** — 段を 1 つずつ完成させるのではなく、細い機能を `.asty` → 生成 .py → 実行まで通す。
- **透明なコンパイル出力** — 生成 Python は人間が読め、追跡でき、レビューできること。

## HOW

### Commands

Rust ワークスペースは `asatsuyu/` サブディレクトリにある。全 cargo コマンドはそこで実行すること。

```bash
cd asatsuyu && cargo fmt --all                                            # Format
cd asatsuyu && cargo clippy --workspace --all-targets -- -D warnings      # Lint (warnings = error)
cd asatsuyu && cargo test --workspace                                     # Test
cd asatsuyu && cargo fmt --all --check                                    # Format check (CI と同じ)
cd asatsuyu && cargo test -p asatsuyu-parser                              # 単一クレート
cd asatsuyu && cargo run -q -p asatsuyu-cli -- run examples/hello.asty    # コンパイラを動かす
cd asatsuyu && cargo run -q -p asatsuyu-cli -- check --purity examples/purity.asty
```

`make` はリポジトリルートで実行する（内部で `asatsuyu/` に入る）。

```bash
make docs-sync         # 文書と実装の一致（診断コード範囲 / FFI 分類 / fixture README / snapshot 数 / CLI フラグ）
make dev               # PyO3 ランタイムを現在の venv に入れる（maturin develop）。pytest の前提
make pytest            # tests/ の Python テスト
make verify-ffi        # FFI 信頼レポート + pyright / mypy による外部検証
make package-install   # 生成パッケージを一時 venv に入れて import できるか
make ci                # 上記を CI と同じ順で全部
make release-gate      # ci + docs-sync
```

VS Code 拡張: `cd editors/vscode && pnpm run check-types && pnpm test`

### Snapshots (insta)

snapshot テストを多用する。CI は `INSTA_UPDATE=no` で走るので、保留 snapshot があると落ちる。

```bash
cd asatsuyu && cargo insta pending-snapshots    # 保留中の一覧
cd asatsuyu && cargo insta accept               # 中身を確認したあとで
```

`cargo insta review` は対話 UI。エージェントは `*.snap.new` を Read で確認してから `accept` する。

## Critical Rules

1. **IMPORTANT: 完了前に fmt → clippy → test** — 実装タスクを完了と報告する前に、上の 3 コマンドを
   この順で全パスさせる。診断コード・FFI 分類・fixture・CLI フラグ・`docs/` に触れたら `make docs-sync` も通す。
2. **IMPORTANT: 憲章に反する変更をしない** — 設計に踏み込む前に `docs/concepts/principles.md` の該当節を確認する。
   憲章の解釈を変える必要があるなら、実装より先に ADR（asatsuyu-adr-writer）で決定を記録する。
3. **IMPORTANT: ユーザ入力で panic しない** — `.asty` / `.pyi` / `asatsuyu.toml` 由来の失敗は必ず `Diagnostic` に載せる。
   パーサはエラーリカバリで吸収し、部分 CST + 診断を返す。
4. **ロスレス CST を壊さない** — 空白・コメント・改行はすべて trivia トークンとして保持する。スキップしない。
5. **依存方向は一方向、クレートは 9 個のまま** — 下流クレートを上流から参照しない。新クレートの追加と
   新しい依存クレートの採用は、先にユーザの了承を得る。
6. **`unsafe` を書かない** — `unsafe_code = "deny"` がワークスペース全体に設定済み。
7. **snapshot は読んでから accept** — 無確認の accept はバグった出力を「正」にする。`*.snap.new` を残さない。
8. **生成 Python を手で直さない** — 出力がおかしいときに直すのは常にコンパイラ側。
9. **lint を黙らせない** — `#[allow(...)]` を足して通さない。例外が本当に要るならワークスペース設定に置き、理由を報告する。
10. **`docs/research/` を根拠にしない** — `.gitignore` された探索メモであり契約ではない。そこの語彙を現行仕様として引かない。

`.claude/settings.json` のフックが `git push` / `gh pr create` 等の push 系コマンドを遮断する。
遮断されたら回避せず、ユーザに実行を依頼する。

## Skills

フレーズとスキルの対応。ユーザスコープの設定に同じフレーズの対応表がある場合、このリポジトリではこの表を優先する。

| フレーズ / 状況 | 呼ぶスキル |
|---|---|
| 「ADR書いて」「ADRにまとめて」「ADRに記録して」 | `asatsuyu-adr-writer` |
| 「計画立てて」「設計して」「プラン作って」「次に何をやるべきか」 | `asatsuyu-slice-planner` + Plan agent |
| 「この設計を詰めて」「憲章と矛盾してないか見て」 | `grill-with-docs` |
| 「TDDで」「TDD で実装」「失敗テストから」 | `tdd-workflow` |
| 「セキュリティレビュー」「脆弱性チェック」「OWASP」 | `security-auditor` |
| 「公式ドキュメント確認して」「調べて」「リサーチして」 | `web-researcher` |
| 「ポストモーテム書いて」 | `postmortem-writer`（ユーザスコープ） |
| emitter を変更した、生成 Python の snapshot が動いた | `asatsuyu-emitter-review` |
| `asatsuyu/crates/**/*.rs` を書く・直す | `asatsuyu-compiler-dev`, `bp-rust`（パスで自動的に有効になる） |
| 長いセッションで前提がずれた | `/reload`（手動） |

## Antigravity MCP delegation

`.mcp.json`（gitignore 済み、マシン固有）が `antigravity` サーバー（agy-orch-mcp）を登録している。

- `antigravity_run` は、独立した範囲の決まった調査・計画・実装を別コンテキストに切り出すために使う。
  既定は `mode: "plan"` / `autonomy: "safe"`。`accept-edits` はタスクが明示的に変更を求めるときだけ。
- `workspace` はリポジトリルートの絶対パス。タスクパケットは GOAL / SCOPE（編集してよいファイル）/
  CONSTRAINTS（通すべきコマンド）/ HANDOFF を書き、1 ターン 15 分以内に収まる大きさに切る。
- ワーカーには 1 コールにつき単純なコマンド 1 つを使わせる。`cd asatsuyu && cargo ...` ではなく
  `cargo test --manifest-path asatsuyu/Cargo.toml --workspace` の形で指示する。
- 独立した読み取り専用の調査は、1 メッセージに複数の `antigravity_run` を並べて並列に出す。
  同じリポジトリへの並列なコード変更は `antigravity_batch`（作業ツリーが clean であること。結果は
  `final.patch` として返り、作業ツリーには適用されない）と `antigravity_fetch` を使う。
- フォローアップは、返ってきた `conversation_id` を `antigravity_continue` に渡し、同じ workspace で続ける。
  `TIMEOUT`（`resumable: true`）も同じく continue で再開する。`BUSY` は `queue_timeout_seconds` で待つ。
- ワーカーの散文を信用しない。実行の証拠は `verified_outputs`、変更は `git diff` で確認し、
  fmt → clippy → test は自分で回し直す。
- `model` / `effort` は渡さない（モデルは `.mcp.json` の env が唯一の正）。委譲先からの再帰的な委譲はさせない。

ワーカー側が従う規約は `AGENTS.md`。

## Things That Will Bite You

### Working directory
プロジェクトルートは `Asatsuyu/` だが、Cargo workspace は `Asatsuyu/asatsuyu/` にある。
`cargo` コマンドをプロジェクトルートで実行すると失敗する（`--manifest-path asatsuyu/Cargo.toml` を付ければ通る）。
`make` と `scripts/*.py` は逆にルートから実行する。

### unsafe の例外は 2 箇所だけ
- `asatsuyu-runtime-python` — PyO3 マクロが内部で unsafe を生成するため crate レベルで allow
- `asatsuyu-syntax/src/syntax_kind.rs` の `From<u16> for SyntaxKind` — 範囲を assert した上での transmute
  （rust-analyzer と同じパターン）。variant を足すときは `__LAST` が末尾のままであること

### clippy pedantic が有効
`clippy::all` + `clippy::pedantic` が warn。allow は `module_name_repetitions` / `missing_panics_doc` /
`missing_errors_doc` / `assert_is_empty` のみ。CI の clippy ジョブはコマンドに `-D warnings` を付けていないが、
`setup-rust-toolchain` が `RUSTFLAGS=-D warnings` を設定するので警告で落ちる。toolchain は固定していないため、
stable の更新で新しい lint が増えると CI が先に赤くなる。

### 単一 SyntaxKind enum
トークンとノードの両方を単一の `#[repr(u16)] SyntaxKind` enum で表現する（rust-analyzer パターン）。
`is_token()` / `is_node()` ヘルパーで分類。

### grammar は凍結されている
`docs/grammar.md` が構文の正で、凍結済み。構文の追加・変更は合意が要り、フォーマッタ・LSP・
VS Code 拡張（`editors/vscode/` の tmLanguage とスニペット、`pnpm test` の grammar snapshot）に波及する。

### IR は immutable
AST / HIR / THIR の全ノードは不変。`Rc`/`Arc` は必要最小限に留める。Arena 確保 (`la-arena`) を HIR コレクションに使用。

### Python 側のテストは準備が要る
`make pytest` は PyO3 ランタイムが venv に入っていることが前提（`make dev`）。`make verify-ffi` は pyright と mypy を使う。

### docs-sync は snapshot 数の下限を見ている
snapshot をまとめて消すと `make docs-sync` が落ちる。意図した削除なら `scripts/check_docs_sync.py` の下限も直す。

## Code Conventions

詳細は `docs/best_practices/rust.md`（`.rs` を触ると `bp-rust` スキルが要点を出す）。

- **Visibility** — `pub(crate)` がデフォルト。公開 API のみ `pub`。`lib.rs` は公開境界、`main.rs` は薄い入口
- **Edition** — `edition = "2024"`
- **Diagnostics** — 全クレートは `Vec<Diagnostic>` を生成する。エラーコード・スパン・ラベル・ヒントを含む構造化診断。
  表示は `miette` が担当（CLI 層で集約）。解析クレートは端末に出力しない
- **String interning** — 識別子には `smol_str::SmolStr`

## Language Design Decisions

Asatsuyu 言語の設計方針。コンパイラ実装時にこれらを前提とすること。

- **ADT**: Gleam 風の代数的データ型（名前付きコンストラクタ）
- **Record**: Go 風のフラットな構造体
- **null なし**: `Option` で表現
- **例外なし**: `Result` で表現。Asatsuyu コード内では例外は発生しない。例外は Python 境界でのみ存在し、そこで `Result` に持ち上げる
- **`let mut` によるローカル可変変数あり**: `let mut x = 0` で明示的に mutable 宣言、`x = 1` で再代入。top-level mutable・parameter 再代入は禁止
- **パイプライン**: `|>` 演算子あり。糖衣は HIR までに脱糖する
- **純粋性**: opt-in の `pure fn`。HIR 呼び出しグラフ上の不動点で検査し、型には載せない（ADR 0002）。
  type-and-effect system は永久 Non-Goal（ADR 0001）
- **並行**: `async` / `await` と組み込み型 `Task(T)` は実装済み。並列スループットは目標ではない
- **FFI**: 信頼段階は `FfiTrustLevel::{Unsafe, Checked, Verified}`。`Opaque` は段階名ではなく `Unsafe` の値表現 `PyOpaque[module.Symbol]`
- **Python 3.12+ 固定**: バックエンドは Python ソース生成のみ。バイトコードや他言語は対象外

## References

- @docs/concepts/principles.md — 言語憲章（5 つの中核原則、設計判断の基準）。**唯一の上位権威**
- `docs/adr/` — 設計判断の記録（`NNNN-<kebab-case>.md`）。憲章の解釈・改訂はここに残す
- `docs/best_practices/rust.md` — Rust 実装規約（クレート境界、エラー処理、テスト方針）
- `docs/architecture.md` — クレート構成とデータフロー、生成 Python の形。大きいファイルなので必要時のみ参照すること
- `docs/roadmap.md` — 実装済み範囲とマイルストーン
- `docs/grammar.md` — 構文の正（凍結）
- `docs/concepts/ffi_conformance.md` / `docs/guides/ffi.md` — FFI 信頼段階と CI ゲート
- `docs/release-checklist.md` — リリース手順
- `AGENTS.md` — 委譲先エージェント（Antigravity / Codex）向けのプレイブック
