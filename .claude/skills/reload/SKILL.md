---
name: reload
description: >-
  Asatsuyu の正とされるプロジェクト文脈（CLAUDE.md・言語憲章・現行 ADR）を読み直し、
  いま進めている作業に効くルールを言い直す。長時間セッションのあと、コンテキスト圧縮のあと、
  作業の前提がリポジトリの記述からずれてきたと感じたとき（「コンテキスト再読み込み」）に手動で使う。
  既にある計画を文書に照らして叩きたいときは grill-with-docs を使う。
allowed-tools: Read, Glob
disable-model-invocation: true
---

# Reload Project Context

次を読み、セッションの残りについて全行を正として扱う。

1. `./CLAUDE.md` — コマンド、Critical Rules、ハマりどころ
2. `docs/concepts/principles.md` — 言語憲章。唯一の上位権威
3. `docs/adr/*.md` — 憲章の解釈・改訂の記録。どの ADR があるかは決め打ちせず Glob で確認する。
   ステータスが「廃止」のものは現行契約として扱わない
4. 作業が Rust 実装に及ぶ場合のみ `docs/best_practices/rust.md`

スキルは各自の frontmatter から発見されるので、ここで読む必要はない。
`docs/research/` は `.gitignore` された探索メモであり、契約ではない。読み直しの対象にしない。

## 出力チェックリスト

読み直した結果を、次の形で短くまとめる。

- [ ] **Critical Rules (`CLAUDE.md`)**: 番号付きルールのうち、進行中の作業に効くものを言い直す
- [ ] **憲章**: 作業が触れる原則（§3）・境界（§4）・Non-Goals（§8）を節番号つきで挙げる。
      触れないなら「該当なし」と明記する
- [ ] **現行 ADR**: 作業に関係する ADR を番号とタイトルで挙げ、その決定が課す制約を 1 行で書く
- [ ] **ずれ**: これまでの作業前提のうち、読み直した内容と食い違っていたものがあれば列挙する
