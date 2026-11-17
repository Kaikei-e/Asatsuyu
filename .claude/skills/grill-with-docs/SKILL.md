---
name: grill-with-docs
description: >-
  計画を Asatsuyu の文書化済みの決定（言語憲章・アーキテクチャ・FFI 信頼モデル）と
  実装コードに突き合わせて詰問するスキル。曖昧な用語を潰し、コードと照合し、
  憲章や既存の契約と矛盾する箇所を洗い出す。
  「この設計を詰めて」「計画を叩いて」「憲章と矛盾してないか見て」
  「この案の穴を突いて」などのフレーズで使うこと。
  実装順序やスライスの切り方を決めたいだけなら asatsuyu-slice-planner を使う。
  文書に照らす必要がなく抽象的に設計を議論したいだけなら、このスキルは使わない。
allowed-tools: Read, Grep, Glob, Bash, Agent
---

<what-to-do>

この計画のあらゆる側面について、共通理解に到達するまで容赦なく問い詰めること。設計ツリーの各枝を降りていき、決定間の依存関係を一つずつ解消する。各質問には必ず自分の推奨回答を添える。

質問は一度に一つだけ。各質問への回答を待ってから次に進むこと。

コードベースを調べれば答えが出る質問は、ユーザーに聞かずに自分で調べること。

</what-to-do>

<supporting-info>

## ドメインの土台

このリポジトリに Obsidian vault は無い。文書化された決定は `docs/` 以下の素の
Markdown と `CLAUDE.md` にある。何かを叩く前に、まず該当する権威を読むこと。

| 権威 | 場所 |
|---|---|
| 言語憲章（最上位。ここと矛盾する計画は却下される） | `docs/concepts/principles.md` |
| コンパイラアーキテクチャ・クレート境界・データフロー | `docs/architecture.md` |
| 実装済み範囲とマイルストーン | `docs/roadmap.md` |
| 構文の正 | `docs/grammar.md` |
| FFI 信頼段階と CI ゲート | `docs/concepts/ffi_conformance.md`, `docs/guides/ffi.md` |
| Rust 実装規約 | `docs/best_practices/rust.md` |
| リリース手順 | `docs/release-checklist.md` |
| 運用ルール（コマンド、禁止事項、ディレクトリの罠） | `CLAUDE.md` |
| 設計判断の記録（憲章の解釈・改訂） | `docs/adr/NNNN-<kebab-case>.md`（新規は asatsuyu-adr-writer 経由） |

判断の優先順位は **コード > `docs/concepts/principles.md` > その他の `docs/`**。
憲章は「何を守り、何を捨てるか」を固定する文書なので、計画が §3 の 5 原則や §8 の
Non-Goals に触れる場合は、まずそこで勝負がつく。ADR は憲章の下位にあり、憲章の特定の
文言をどう読むかを固定している。憲章 §0 により、ADR が憲章を改訂するときは憲章本文も
同じ変更で直されているはずなので、両者が食い違っていたらそれ自体が指摘事項になる。

どの ADR があるかは決め打ちせず `docs/adr/` を Glob して確認すること。ステータス行が
「廃止」の ADR は現行契約ではない。

### `docs/research/` は契約ではない

`docs/research/` は `.gitignore` されている。`IMPL_PHASES*.md` / `core_concept*.md` /
`refine.md` / `asatsuyu-roadmap.md` は探索メモであって、追跡されている契約ではない。
リポジトリが保証しているのは `docs/concepts/principles.md` の方。research 側の語彙を
現行契約として扱わないこと。

### 文書と実装のズレを機械的に見る

`make docs-sync`（= `python scripts/check_docs_sync.py`）が、診断コードの範囲・FFI の
Verified/Checked 分類・fixture README・スナップショット数・CLI の FFI フラグについて、
文書と実装の一致を検査する。文書の記述を根拠に何かを主張する前にこれを走らせれば、
その記述が守られているかどうかが分かる。

このリポジトリに ADR のグラフを解決する仕組みは無い。置換関係は各 ADR 冒頭の
ステータス行（`廃止（→ NNNN で置換）`）と本文中のリンクを読んで判断すること。

## セッション中の振る舞い

### 文書化された語彙で殴る

ユーザーの使う用語が文書の定義と食い違っていたら、その場で指摘する。
「憲章 §4.1 では Verified は "no `Any` leaks" が条件ですが、今の話だと部分 stub を
含んでいます。それは Checked ですよね？」

**既知の罠 — FFI の段階名**: コード上の正式な段階は
`FfiTrustLevel::{Unsafe, Checked, Verified}`（`asatsuyu-hir/src/ffi/model.rs`）で、
`Ord` は `Unsafe < Checked < Verified`。`Opaque` は段階名ではなく `Unsafe` の値の
表現（`PyOpaque[module.Symbol]`）。憲章 §4.1 や roadmap の見出しにある
「Unsafe / Opaque FFI」は段階と値表現を並べた書き方なので、`Opaque` を 4 つ目の段階や
`Unsafe` の別名として引かないこと。

### 先送りと書いてある機能が実装済みなことがある

`docs/roadmap.md` の "What Comes Next" は過去に更新漏れを起こしている（`async`/`await`・
`Task(T)`・`let mut` が、実装済みなのに先送り欄に載っていた。現在は訂正済み）。
同じことは再発しうる。**roadmap を根拠に「まだ無い」と断じないこと**。無いと主張する前に
必ずコードかテスト（golden case は `asatsuyu/crates/asatsuyu-cli/tests/cases/`）で確認する。

逆に、憲章 §8 の Non-Goals は生きている。クラス・継承・トレイト・マクロ・依存型・
複数バックエンドは、計画がそこに踏み込んだ時点で憲章違反を指摘する。ただし §8 には
読み違えやすい項目が 2 つあり、ADR と憲章本文が線を引いている。

- **効果**: 禁じられているのは *type-and-effect system*（関数型に効果変数を載せて単一化する
  こと）であり、これは先送りではなく永久の除外（ADR 0001）。純粋な関数と副作用のある関数を
  *区別すること* は除外されていない。その機構は HIR 呼び出しグラフ上の不動点 + opt-in の
  `pure` 宣言で決定済み（ADR 0002）。「effect system は Non-Goal だから純粋性も扱えない」は誤読。
- **並行**: `async` / `await` と `Task(T)` は実装済みで、`async def` と `asyncio.run` を生成する。
  未決なのは表層構文を structured concurrency に置き換えるかどうか。除外されているのは
  並列スループットの追求（速度は評価軸ではない）。

### 曖昧な言葉を尖らせる

ユーザーが曖昧語・多義語を使ったら、正確な用語を提案する。Asatsuyu で特に潰す価値が
あるのは以下:

- **「AST」** — CST / AST / HIR / THIR のどれか。ロスレス CST と型なし AST は別物で、
  責務を持つクレートも違う
- **「型」** — 推論中の `Ty` か、表層の型注釈か、`ThirModule` に載る確定型か
- **「エラー」** — ユーザー向けの `Diagnostic`（code / span / labels / hints 付き）か、
  クレート内部の `thiserror` 型か。憲章的にはこれは別の生き物
- **「Result」** — Asatsuyu 言語の `Result` か、コンパイラ実装側 Rust の `Result` か
- **「トークンを飛ばす」** — trivia は保持必須。ロスレス性を壊す提案なら即座に指摘
- **「Phase」「Milestone」** — roadmap の Chapter / Milestone / Phase は別軸。かつ
  `CLAUDE.md` によりコード・コメント・docstring には書けない
- **「純粋」** — HIR の不動点が推論した純粋性か、`pure` 宣言による検査対象か（ADR 0002）
- **「文法を変える」** — `docs/grammar.md` は凍結されている。構文の追加は合意が要る変更で、
  フォーマッタ・LSP・VS Code 拡張の tmLanguage に波及する

### 具体的なシナリオで詰める

概念間の関係が話題になったら、具体例で殴る。境界の曖昧さを露呈させるシナリオを
自分で作ること。Asatsuyu なら、生成される Python のスニペットを実際に書かせるのが
最も効く。「その設計だと `match` の網羅性が漏れたケースで何が emit されますか？」

### コードと照合する

ユーザーが「これはこう動く」と言ったら、コードがそれに同意しているか確認する。
矛盾を見つけたら表に出す。

証拠として使えるもの:

- `cargo test -p <crate>` — テスト名がそのまま仕様の主張になっている
- `insta` スナップショット — emitter / parser の出力は snapshot が正
- `asatsuyu/crates/asatsuyu-hir/tests/ffi_conformance.rs` — FFI 分類の実際の主張
- `make docs-sync` — 文書と実装の機械的一致

cargo は `asatsuyu/` で走らせること（リポジトリルートでは失敗する）。

### 確定した用語はその場で記録する

用語が固まったら、最後にまとめてではなく、走っている要約にその場で書き込む。
精度こそがこのセッションの目的であり、後回しにすると蒸発する。

`docs/` への書き込みは別行為で、ユーザーの承認が要る。先に聞くこと。
`CLAUDE.md` の方針どおり、頼まれていない新規ドキュメントを作らない。用語集ファイルを
勝手に作ると、誰も保守しない第二の正が生まれる。

### ADR は出し惜しむ

ADR 化を提案するのは、次の 3 つが全て成り立つときだけ:

1. **覆しにくい** — 後で気が変わったときのコストが実質的に大きい
2. **文脈なしでは驚く** — 将来の読者が「なぜこうした？」と思う
3. **本物のトレードオフの結果** — 実在する代替案があり、理由があって片方を選んだ

一つでも欠けるなら ADR は要らない。

該当する場合は、ここで形式を発明せず **asatsuyu-adr-writer** に渡す。このリポジトリの
ADR 規約は一つ（`docs/adr/NNNN-<kebab-case-title>.md`、4 桁連番、日本語、
「言語憲章との関係」セクション必須）で、競合する第二の形式を作らないこと。
憲章の文言に触れる決定なら、ADR と同じ変更で憲章本文も改訂する（憲章 §0）。

### 実装順序の話になったら渡す

「で、どういう順で作るのか」に話が移ったら、このスキルの仕事は終わり。
**asatsuyu-slice-planner** に渡すこと。このリポジトリには縦に薄い end-to-end 貫通を
優先する明示的な方針があり、それはこのスキルの守備範囲ではない。

</supporting-info>
