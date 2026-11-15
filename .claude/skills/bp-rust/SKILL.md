---
name: bp-rust
description: >-
  Asatsuyu の Rust 実装規約（Edition 2024）。可視性、エラー型と診断の分離、所有権と借用、
  enum と網羅的 match、出力方針（stdout / stderr / tracing）、依存追加の基準、lint 例外の置き場所を扱う。
  .rs ファイルを書く・直す・レビューするときに使う。ユーザが「規約」「ベストプラクティス」と
  言わなくても、asatsuyu/crates/ の実装・修正に入るなら使う。
  テストの実行だけ、Cargo.toml を眺めるだけ、他言語の作業では使わない。
  パイプラインの段の責務やパーサの回復方式など、コンパイラ設計の決まりは asatsuyu-compiler-dev が担当する。
paths:
  - "asatsuyu/crates/**/*.rs"
---

# Rust Best Practices (Asatsuyu)

正は `docs/best_practices/rust.md`。以下はその要点で、タスク全体を通じて有効な standing rules である。
根拠やコード例が要るときは、該当する節だけを Read する（全文は読まない）。

## 要点

1. **Edition 2024、lint はワークスペースで一元管理** — `clippy::all` + `clippy::pedantic` が warn、
   CI 相当は `-D warnings`。lint 例外はコードの `#[allow]` ではなくワークスペース設定に集約する（§2, §13）
2. **`pub(crate)` がデフォルト** — `pub` は意図した公開 API だけ。`lib.rs` が公開境界、`main.rs` は薄い入口（§2）
3. **内部エラーとユーザ向け診断を分ける** — クレート内は `thiserror` のドメインエラー、ユーザ向けは
   code / span / labels / hints / notes を持つ `Diagnostic`。表示は CLI で `miette` に変換（§5）
4. **`panic!` はバグ専用** — ユーザ入力・ファイル内容では絶対に panic しない。`unwrap()` はテスト以外で安易に使わず、
   `expect` は「落ちたらバグ」の文脈だけ（§5, §14）
5. **ライブラリクレートは出力しない** — 解析クレートで `println!` / `eprintln!` / `std::process::exit` を呼ばない。
   CLI では正規出力を stdout、診断・進捗を stderr、内部計測を `tracing` に分ける（§3, §9, §10）
6. **enum + 網羅的 match** — 文字列で種別を判定しない。`_` で情報を捨てすぎない。
   将来の variant 追加をコンパイラに検出させる（§7）
7. **借用優先、`clone()` を安易に足さない** — `String` を乱用せず、識別子は `SmolStr`、位置は `Span`。
   ホットパスの割り当てに注意（§4, §12）
8. **コアは同期** — `async` は外部 I/O が必要な境界（LSP サーバー）だけ。CPU バウンドな解析を `tokio` に載せない（§8）
9. **段を潰す便利関数を作らない** — 公開 API の関数名は段を表す（`parse` / `lower_to_hir` / `emit_module`）。
   デバッグ可能性を優先する（§6）
10. **依存追加は理由を説明できるものだけ** — 既存の基盤（`logos` / `rowan` / `clap` / `miette` / `smol_str` /
    `la-arena` / `insta`）を基準にし、大きな依存は先にユーザに確認する（§15）
11. **`unsafe` 禁止** — ワークスペース全体で `unsafe_code = "deny"`（§14）
12. **最適化は計測してから**（§12）

## 節の対応

| 知りたいこと | `docs/best_practices/rust.md` |
|---|---|
| クレート境界と依存方向 | §3 Crate Boundaries |
| データの持ち方、span の保持 | §4 Data and Ownership |
| エラーと診断 | §5 Errors and Diagnostics |
| 公開 API の形 | §6 API Design |
| 出力・ログ・tracing | §9, §10 |
| テストの置き方 | §11 Testing |
| レビュー観点 | §16 Review Checklist |

## 変更後の確認

```bash
cd asatsuyu && cargo fmt --all
cd asatsuyu && cargo clippy --workspace --all-targets -- -D warnings
cd asatsuyu && cargo test --workspace
```
