---
name: tdd-workflow
description: >-
  Asatsuyu コンパイラの変更をテストファーストで進める。外側から順に
  受け入れ（golden case / diagnostic fixture / e2e）→ 境界契約（FFI conformance・docs-sync）→
  クレート単体（RED-GREEN-REFACTOR）と書き、最後に CI と同じゲートをローカルで通す。
  機能追加・バグ修正・リファクタの着手時、「最初にどのテストを書くか」を決めるとき、
  ユーザが「TDDで」「TDD で実装」「失敗テストから」と言ったときに使う。
  「テストが通った」は「CI が通る」ではないので、完了を宣言する前にも使う。
  生成 Python の読みやすさそのものを評価したいときは asatsuyu-emitter-review を使う。
allowed-tools: Bash, Read, Glob, Grep, Edit, Write
argument-hint: <feature-description> [--crate=<asatsuyu-xxx>]
---

# TDD Workflow (Asatsuyu)

テストは外側から書く: **受け入れ → 境界契約 → 単体**。これは*書く順序*であり、数はピラミッドに従う
（受け入れは少なく、単体は多く）。`$ARGUMENTS` は機能説明。`--crate=` で対象クレートを固定する。

cargo は必ず `asatsuyu/` で実行する（リポジトリルートでは失敗する）。

**コミット粒度:** 実装中はステップ単位で commit しない。TDD の例外として
**RED と GREEN だけは別 commit** にする。RED commit は失敗するテスト一式（Phase 0〜2）、
GREEN commit は実装とリファクタで、Phase 5 のゲートが全部通ってから打つ。

## Phase 0: 受け入れテストを先に書く

変更が外から観測できる形を、最も外側のハーネスで固定する。隣のケースを真似て書く。

| 変更の種類 | 書く場所 | 何が固定されるか |
|---|---|---|
| 構文・型・脱糖・生成コードの追加や変更 | `crates/asatsuyu-cli/tests/cases/<kebab-name>/input.asty` | `golden.rs` が CST / AST / HIR / THIR / 生成 Python / 診断を段ごとに snapshot |
| 新しい診断、診断文言・span・hint の変更 | `crates/asatsuyu-cli/tests/fixtures/diag/<stage>-<desc>.asty` | `diagnostic_snapshots.rs` が code / severity / span / labels / hints / notes を snapshot |
| 実行結果や CLI の stdout / stderr / exit code | `crates/asatsuyu-cli/tests/e2e.rs`（単一ファイル）、`fixtures/projects/<name>/` + `fixture_projects.rs`（プロジェクト） | 実バイナリを subprocess で起動して検証 |
| panic の再発防止 | `crates/asatsuyu-cli/tests/corpus/crash-<stage>-<desc>.asty` | `crash_safety.rs` が全段で panic しないことを検証 |
| フォーマッタ | `crates/asatsuyu-parser/tests/format_golden.rs` | 整形結果と冪等性 |
| LSP | `crates/asatsuyu-cli/src/lsp/` の単体 + snapshot | hover / definition / semantic tokens など |

1 つのクレート内部で完結し、外から見える振る舞いが変わらないリファクタは Phase 2 から始める。

実行して **正しい理由で RED** になっていることを確認する。期待した機能が無いから落ちているのであって、
構文エラーやパス間違いで落ちているのではないこと。

### snapshot テストの RED

insta の新規 snapshot は `*.snap.new` として保留され、CI と同じ `INSTA_UPDATE=no` では失敗する。
これが snapshot の RED。ただし「snapshot が無いから落ちる」は弱い RED なので、振る舞いの核には
手書きの assert を 1 つ添える（例: 診断コードが `E0203` であること、生成 Python に
`match` 文が含まれること）。snapshot は形の回帰検知、assert は意図の表明。

```bash
cd asatsuyu && INSTA_UPDATE=no cargo test -p asatsuyu-cli --test golden      # CI と同じ挙動
cd asatsuyu && cargo insta pending-snapshots                                  # 保留中の一覧
```

`cargo insta review` は対話 UI なのでエージェントからは使えない。保留された `*.snap.new` を Read で
**中身まで読み**、意図した出力だと確認できたものだけ `cargo insta accept` する。
読まずに accept すると、バグった出力がそのまま「正」になる。

## Phase 1: 境界契約の確認

Asatsuyu で「契約」と呼べるのは、パイプラインの外に約束しているものである。変更が次のどれかに
触れるなら、実装より先にその契約側のテスト・文書を更新して RED にする。どれにも触れないなら Phase 2 へ。

| 触れるもの | 契約の置き場所 | 確認コマンド |
|---|---|---|
| `DiagnosticCode` の追加・範囲変更 | `asatsuyu-syntax/src/diagnostic.rs` の範囲コメント | `make docs-sync` |
| FFI モジュールの Verified / Checked 分類 | `crates/asatsuyu-hir/tests/ffi_conformance.rs`, `docs/concepts/ffi_conformance.md` | `cargo test -p asatsuyu-hir --test ffi_conformance`, `make docs-sync` |
| Checked FFI のランタイム検証・例外→`Result` 変換 | `tests/test_ffi_*.py`, `tests/test_runtime.py` | `make dev && make pytest` |
| CLI の FFI フラグ | `docs/guides/ffi.md` | `make docs-sync` |
| fixture project の追加・削除 | `asatsuyu/fixtures/projects/README.md` の表 | `make docs-sync` |
| 生成パッケージの形（`pyproject.toml` / wheel） | `scripts/test_package_install.py` | `make package-install` |
| 憲章 §3 の原則・§8 の Non-Goals | `docs/concepts/principles.md`, `docs/adr/` | 実装を止めて grill-with-docs で照合。覆すなら asatsuyu-adr-writer |

`make` はリポジトリルートで実行する。

## Phase 2: RED（単体）

1. 触る段を特定する: lexer / parser / ast / hir / ty / backend-python / cli。段をまたぐ変更は
   上流の段から順に書く。
2. そのクレートの既存の置き方に合わせる（`src/` 内の `#[cfg(test)] mod tests`、または `tests/*.rs`）。
   新しいテストフレームワークやマクロは導入しない。
3. テストするのは振る舞い: 正常系・エラー系・境界値。型検査なら診断の **code と span** まで assert する。
   パーサなら壊れた入力に対する **回復後の CST + 診断** も含める。
4. 実装側は先にシグネチャだけ置き、本体は `todo!()` にする（clippy は通らないので GREEN までに消す）。
5. テストが実装なしで通ってしまったら、そのテストは何も検証していない。書き直す。
6. Phase 0〜2 のテストをまとめて RED commit:

   ```bash
   git commit -m "test(<crate>): add failing tests for <feature>"
   ```

## Phase 3: GREEN

テストを通すのに必要なコードだけを書く。

- テストを書き換えて通さない。テストが無い振る舞いを足さない。
- 依存方向を守る: `lexer → parser → ast → hir → ty → backend-python → cli`。下流クレートを上流から参照しない。
- ユーザ入力由来の失敗は `Diagnostic` に載せる。`panic!` / `unwrap()` で落とさない。
- 脱糖は HIR までに終える。型推論に構文糖衣の知識を漏らさない（憲章 §6.6）。
- 保留 snapshot は Read で確認してから `cargo insta accept`。

## Phase 4: REFACTOR

命名を直し、重複を消し、単純にする。変更のたびにテストを回す。生成 Python が変わるリファクタは
snapshot の diff を必ず読む（意図しない出力変化はここで見つかる）。実装 commit は Phase 5 まで保留。

## Phase 5: ローカル CI パリティと GREEN commit

CI（`.github/workflows/ci.yml`）と同じゲートを、完了報告の前にローカルで通す。

```bash
cd asatsuyu && cargo fmt --all
cd asatsuyu && cargo clippy --workspace --all-targets -- -D warnings
cd asatsuyu && INSTA_UPDATE=no cargo test --workspace
```

`git diff --name-only` で触った場所を列挙し、該当するものを追加で回す:

| 触った場所 | 追加ゲート |
|---|---|
| 診断コード / FFI 分類 / fixture / CLI フラグ / `docs/` | `make docs-sync` |
| `asatsuyu-runtime-python/`、Checked FFI、`tests/*.py` | `make dev && make pytest` |
| FFI resolver / stub / typeshed | `make verify-ffi` |
| パッケージ生成 | `make package-install` |
| `editors/vscode/` | `cd editors/vscode && pnpm run check-types && pnpm test` |

- 落ちたゲートを黙らせない（`#[allow(...)]` の追加、テストの `#[ignore]`、snapshot の無確認 accept）。
  lint 例外が本当に要るならワークスペース設定側に置き、理由を報告する。
- `cargo insta pending-snapshots` が空であること。`*.snap.new` を残したまま commit しない。
- 全部通ったら GREEN commit:

  ```bash
  git commit -m "feat(<crate>): implement <feature>"
  ```

commit はここまで。`git push` と PR 作成はユーザの明示指示があるまで行わない。
