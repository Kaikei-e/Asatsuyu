---
name: asatsuyu-emitter-review
description: >-
  Asatsuyu の Python emitter が生成した .py コードの品質をレビューするスキル。
  emitter の実装・変更後、生成コードのスナップショット更新時、
  「生成された Python を確認して」「emitter の出力をレビューして」
  「この出力は読めるか」などのフレーズで必ず使うこと。
  生成 Python の可読性は憲章の評価軸の 2 番目であり、「透明なコンパイル出力」は
  中核原則の 1 つなので、通常のコード生成よりはるかに高い可読性基準を適用する。
allowed-tools: Read, Grep, Glob, Bash(cargo *), Bash(uvx *), Bash(python3 *), Bash(pyright *), Bash(diff *)
---

# Asatsuyu Emitter Output Review

生成 Python の品質は Asatsuyu の差別化そのものである。「動く」だけでは不合格。

根拠は言語憲章（`docs/concepts/principles.md`）:

- §2 評価軸 — 型エラーの質、**生成コードの可読性**、Python 資産を使う際の摩擦の低さ
- §3 原則 5 — 生成 Python は人間が読め、追跡でき、レビューできなければならない
- §5 — Python 3.12+ を前提に `match`/`case` と新しい型パラメータ構文を使い、自然な出力にする

## レビュー基準（すべて満たすこと）

1. **単独可読性（最重要）**: Asatsuyu を知らない Python 開発者が、この .py ファイル**だけ**を読んで
   何をしているか理解し、レビューできるか。コンパイラ都合の名前（`_tmp1`、`__asty_*` の乱発）、
   不自然な制御フロー、説明のつかないヘルパーが 1 つでもあれば不合格。
2. **慣用的な Python 3.12+**: PEP 695 の型パラメータ構文、PEP 634 の `match`/`case`、
   `dataclass(frozen=True)` 等、現代の Python 開発者が自分で書くのと同じ構文を使っているか。
   古い typing 構文（`TypeVar` の手動宣言等）に落ちていないか。
3. **型チェッカクリーン**: 生成コードが pyright / ty でエラーゼロか。Asatsuyu が健全と主張する以上、
   生成物が Python 側の型チェッカに怒られるのは矛盾である。
4. **決定性と diff 安定性**: 同じ入力から常にバイト同一の出力が出るか。ソースの小さな変更が
   生成物の小さな diff に対応するか（snapshot でレビューする前提なので、diff の読みやすさは品質要件）。
5. **ランタイム最小**: 生成コードが依存する Asatsuyu ランタイムヘルパーは最小か。
   ヘルパーを足すより慣用的な Python に展開するほうを常に優先する。
6. **追跡可能性**: `--source-map` 指定時の `# asty:L<n>` コメントが元ソースの行と対応しているか。
   失敗は例外の握りつぶしではなく `Result` として値で運ばれているか（憲章 §3 原則 2）。

## 手順

1. 対象の `.asty` を生成する。パッケージ一式が要らなければ `--no-emit-package` で .py だけ出す。
   出力先はスクラッチ領域にし、リポジトリ内に生成物を残さない。

   ```bash
   cd asatsuyu && cargo run -q -p asatsuyu-cli -- build <path/to/file.asty> --no-emit-package --source-map -o <scratch-dir>
   ```

   golden case を見るだけなら、既存の snapshot（`crates/asatsuyu-cli/tests/snapshots/` の `py`）を Read すればよい。
2. `.asty` ソースと生成 .py を並べて読み、基準 1・2・6 を目視でレビューする。
3. 静的検査を実行する（`ruff` はローカルに入っていない前提で `uvx` 経由）:

   ```bash
   uvx ruff check <生成ファイル>
   uvx ruff format --check <生成ファイル>
   pyright <生成ファイル>          # 無ければ: uvx ty check <生成ファイル>
   ```

4. 生成コードを実際に実行（`cargo run -q -p asatsuyu-cli -- run <file.asty>`、またはテスト経由）し、動作を確認する。
5. 基準 4 の確認: 別々の出力先に 2 回生成し、`diff -r` で差分ゼロであることを確認する。

## 出力形式

基準 1〜6 それぞれについて 合格 / 不合格 + 根拠（該当する生成コードの抜粋）を報告する。
不合格項目には、コンパイラ側の修正方針（どの段で直すべきか: HIR の脱糖 / THIR / emitter）を必ず添える。
生成物へ手パッチを当てる提案はしない — 直すのは常にコンパイラ側である。
