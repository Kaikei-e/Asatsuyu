# Asatsuyu 言語憲章

## 0. 文書の位置づけ

本憲章は、Asatsuyu の設計判断における最上位原則を定める。
構文、型システム、FFI、コード生成、ツールチェイン、ロードマップに関する個別判断は、すべて本憲章に従わなければならない。

本憲章は、実装詳細を列挙する仕様書ではない。
本憲章の役割は、Asatsuyu が何者であり、何を守り、何を捨てるかを固定することにある。Asatsuyu はまず「型安全な Python フロントエンドを 1 本通す」ことに集中する。

---

## 1. 定義

Asatsuyu は、**Python エコシステムのための、静的型付き・関数型・コンパイル型アプリケーション言語**である。

Asatsuyu の目的は、Python のライブラリ資産と運用資産を維持したまま、Python 単体では得にくい以下を前面に出すことにある。

* 設計の明快さ
* 失敗の可視性
* 分岐の安全性
* データ形状の明示性
* 境界の秩序

Asatsuyu は、Python を捨てさせるための言語ではない。
また、Python に型ヒントを薄く足すためだけの言語でもない。
Asatsuyu は、**Python の現実に接続されたまま、アプリケーション層の品質を一段引き上げるための言語**である。

---

## 2. 使命

Asatsuyu の使命は、Python の資産を維持したまま、より安全で見通しのよいアプリケーションロジックを書けるようにすることである。

Asatsuyu は、速度そのものを第一目的としない。
初期段階の評価軸は、以下の三つである。

* 型エラーの品質
* 生成コードの読解性
* Python 資産活用時の摩擦の低さ

この優先順位は変更しない。

---

## 3. 中核原則

### 第1原則: 明快さは簡潔さに優先する

Asatsuyu は、短く書けることより、誤読されにくく壊れにくいことを優先する。
型、分岐、失敗、データ形状は、可能な限りコード上に現れていなければならない。

Asatsuyu は、賢く見える言語ではなく、見通しを回復する言語である。

### 第2原則: 失敗は隠さない

Asatsuyu の内部世界では、失敗は原則として `Result` に載る。
失敗は暗黙の例外伝播に委ねず、型として明示されなければならない。

例外は Python 境界にのみ存在しうる。
Asatsuyu の純粋領域に、無秩序な例外伝播を持ち込んではならない。

### 第3原則: データを主役にする

Asatsuyu の中心は、class / inheritance ではない。
中心に置くのは、以下である。

* immutable data
* algebraic data types
* functions
* pattern matching

振る舞いをオブジェクトへ閉じ込めるよりも、データ形状を明示し、関数で扱うことを優先する。
Asatsuyu は、OOP の再発明を目指さない。

### 第4原則: Python を敵にしない

Asatsuyu は Python を否定する言語ではない。
Python の不足しがちな厳密さを補う、補完的な言語である。

したがって、Asatsuyu の価値は Python との断絶ではなく、Python との接続の質によって測られる。

### 第5原則: 透明なコンパイル結果を守る

Asatsuyu は、Python に落ちることを隠さない。
むしろ、**Python に落ちること自体を透明性として利用する**。

生成される Python コードは、人間が読め、追跡でき、必要時にレビュー可能でなければならない。
デバッグ性、可観測性、CPython 互換性は本線で守る。

---

## 4. 言語の境界

Asatsuyu の成否は、内部の型システムだけでは決まらない。
**Python との境界設計が、美しく、狭く、明示的であること**が必須である。

Asatsuyu の FFI は、次の原則に従う。

1. Python 関数は、型付きで import できなければならない。
2. 型情報は、`.pyi`、stub package、`py.typed`、typeshed、PEP 561 系の既存基盤を優先して利用する。 ([PEP 561][2])
3. 独自宣言フォーマットは、初手で発明しない。
4. Python 例外は、境界で `Result` に持ち上げる。
5. `Any` は危険型として扱い、暗黙に安全世界へ流入させない。`Any` は全型と相互に代入可能であり、境界を無制限に通すと soundness を壊す。 ([Typing spec: Any][3])
6. `__getattr__`、`eval`、monkey patch のような動的性は危険領域に隔離する。 ([Typing spec: distributing][4])
7. 任意の Python ライブラリに対する完全 soundness は初年度の目標にしない。
8. FFI は `Verified` / `Checked` / `Unsafe` の3層で扱う。
9. partial stub package や unknown を含む surface は `Verified` に入れない。
10. 巨大な concrete class 全体を直接信じるより、`Protocol`・`TypedDict`・明示 validator を優先する。 ([Protocols][5]) ([PEP 647][6])

### 4.1 FFI の健全性モデル

Asatsuyu の内部世界は sound であることを目指す。
ただし、その soundness を Python 境界の外側まで無条件に拡張してはならない。
PEP 561 は型情報の配布順序を与えるが、stub の正しさそのものを保証しない。したがって Asatsuyu は、
**sound core + verified boundary** を守る言語として設計する。 ([PEP 561][2])

#### Verified FFI

以下を満たすものだけを `Verified` とする。

* `py.typed`、stub package、typeshed のいずれかで型情報を解決できる
* exported surface に `Any`、bare generic、partial stub 由来の unknown が残らない
* 型 completeness の検査を CI に組み込める
* stub/runtime の乖離検査を継続できる

`Verified` に入った symbol だけが、Asatsuyu の通常の型として扱われる。Pyright の `--verifytypes` は型 completeness の検査に利用でき、mypy の `stubtest` は stub と runtime 実装の乖離検査に使える。 ([Pyright --verifytypes][7]) ([stubtest][8])

#### Checked FFI

`Checked` は、static な型情報はあるが、そのまま sound 扱いできない領域である。
この層では compiler-generated wrapper により、少なくとも次を行わなければならない。

* 引数検査
* 戻り値検査
* 例外の `Result` 化
* 動的値の narrowing

JSON や dict ベースの戻り値、`requests.Response.json()` のような API は、初年度はこの層に置く。runtime validator はまず既存の Python typing 資産を活用して立ち上げ、必要に応じて生成 validator へ移行してよい。 ([Typeguard][9])

#### Unsafe / Opaque FFI

`Unsafe` は sound world に入れてはならない領域である。
この層の値は `PyOpaque[module.Symbol]` のような opaque 型として隔離し、
field access、pattern matching、暗黙変換を禁じる。
許されるのは、同じ opaque 型を受け取る別の foreign call へ受け渡すことと、
明示的な境界関数による checked conversion のみである。

### 4.2 境界で許可する型の優先順位

MVP では、次の型を優先して受理する。

* `int`, `float`, `str`, `bool`, `None`
* `list[T]`, `tuple[...]`, `dict[K, V]`
* `Literal`, `Union`, `Optional`
* `TypedDict`
* fully-known な generic class
* 面を絞った `Protocol`

逆に、以下は `Verified` に入れてはならない。

* `Any`
* 型引数なし generic
* partial stub package
* `__getattr__` 依存の surface
* plugin がないと型が閉じない API

### 4.3 MVP における FFI の線引き

初期フェーズの対象は次のとおりとする。

* `pathlib`, `os`, `sys` は `Verified` を目指す
* `json` は `Any` を含むため `Checked` として扱う
* `requests` は `Checked` で受け入れる
* `numpy` は `Checked / Opaque-first` で扱う
* `pandas`, `torch` は `Opaque-first` を原則とする

---

## 5. コンパイルターゲット

Asatsuyu の本線バックエンドは、**Python ソースコード生成**に固定する。
少なくとも初年度において、これを揺らがせてはならない。

理由は以下のとおりである。

* デバッグ性を高く保てる
* 可観測性を保てる
* CPython 互換性を最大化できる
* 生成結果を人間が監査できる
* 移行型言語としての信頼を作りやすい

ランタイムターゲットは Python 3.12+ とする。
Asatsuyu は、Python 側の `match/case` と新しい型パラメータ構文・`type` 文を前提に、生成コードの簡潔さと自然さを確保する。    ([Python Enhancement Proposals (PEPs)][1])

---

## 6. 文法と意味論で守るべき核

Asatsuyu は全部盛りの言語であってはならない。
最初に守るべき核は、次のとおりである。

### 6.1 値中心

文よりも式を優先する。
ただし、極端な式言語にはしない。
可読性が落ちるなら、簡潔さを捨てる。

### 6.2 ADT 中心

record / tuple / enum 的な断片機能ではなく、**バリアントを持つ代数的データ型**を中核に置く。

### 6.3 パターンマッチ中心

`match` は補助構文ではない。
`if` より強いデータ分解の主要構文である。Python 側でも structural pattern matching は既に仕様化されているため、Asatsuyu の `match` を Python へ自然に投影する方針は妥当である。    ([Python Enhancement Proposals (PEPs)][1])

### 6.4 `Option` による nullability の封じ込め

`None` 相当の存在は散らさない。
nullable な値は `Option` に閉じ込め、型の世界で扱う。

### 6.5 `Result` による失敗表現

失敗は `Result` を用いて表現する。
日常的な try/catch 文化を Asatsuyu 内部へ持ち込んではならない。
`try` は Python 境界の例外吸収に限定される。

### 6.6 脱糖は HIR まで

パイプライン、文字列結合などの構文糖は許容する。
ただし、それらは HIR までに脱糖され、型推論系は構文上の都合を知らずに動けなければならない。

---

## 7. 型システムの責務

Asatsuyu の型システムは、単なる注釈の表示機構ではない。
型システムは、設計そのものを支える中核機構である。

最低限、型システムは以下を満たさなければならない。

* Hindley-Milner 系の型推論
* occurs check
* let-polymorphism
* ADT の型付け
* `match` の網羅性検査
* 到達不能アームの検出
* 「期待型 / 実際型」を示すエラー診断

Asatsuyu の型システムは、日常の設計を一段引き上げるために存在する。
研究目的の過剰な理論装置を、初年度の中核へ持ち込んではならない。

---

## 8. 非目標

Asatsuyu は、以下を初年度の中心目標にしない。

* `.pyc` 直生成の本流採用
* JIT / ネイティブ最適化
* クラス / 継承 / trait 的抽象
* async/await の完全ネイティブ設計
* effect system / macro system
* package registry
* 多バックエンド化
* 依存型 / refinement type
* 可変変数の導入

また、研究トラックとして検討するものは、本線を侵食してはならない。
Python バイトコード直生成、スタブ自動生成、Rust ネイティブ拡張は、MVP 後に扱う。

---

## 9. 想定ユーザーと適用範囲

Asatsuyu の第一ターゲットは、次の条件を満たす開発者である。

* Python を実務または個人開発で使っている
* 型安全を欲している
* Rust ほど重い移行は望んでいない
* 関数型の利点に魅力を感じる
* しかし Python 資産は捨てたくない

Asatsuyu が初期に強くあるべき領域は、以下である。

* CLI ツール
* API クライアント
* JSON / HTTP 処理
* データ変換
* バッチ処理
* 軽量なドメインロジック
* Python ライブラリ呼び出しを含むアプリケーション層

逆に、以下は初期の主戦場にしない。

* 超低レベル処理
* 高性能数値計算の中心部
* 複雑な async 基盤
* Python メタプログラミング依存コード
* 巨大フレームワークの深部統合

---

## 10. MVP の定義

Asatsuyu の MVP は、次を満たした時点で成立する。

* Asatsuyu で 300〜500 行程度の CLI を書ける
* `Result` / `Option` / `match` / ADT を実用的に使える
* `pathlib` を Verified FFI として、`requests` を Checked FFI として呼べる
* 読める Python 3.12+ パッケージを生成できる
* 型エラー品質、生成コードの読解性、Python 資産活用時の摩擦で十分な水準に達する

MVP の判定基準に、速度を置いてはならない。

---

## 11. 設計判断の基準

今後の機能追加・仕様変更・最適化判断は、次の問いで評価する。

1. その機能は、失敗・分岐・データ・境界の秩序を強くするか。
2. その機能は、Python との橋を太くするか。
3. その機能は、生成 Python の透明性を保つか。
4. その機能は、MVP の対象領域を強くするか。
5. それとも、単に言語を派手にするだけか。

5番目に当てはまる機能は、原則として採用してはならない。

---

## 12. 結語

Asatsuyu は、動的で豊かな現実世界である Python に対して、静的で明快な思考の足場を与えるための言語である。

Asatsuyu の価値は、厳密さそのものではない。
Asatsuyu の価値は、**失敗、分岐、データ、境界に秩序を与え、見通しを回復すること**にある。

この原則に反する変更は、どれほど魅力的に見えても、本憲章に照らして退ける。

必要なら次に、これを **ADR 風の規範文書** にして、
`Status / Context / Decision / Consequences / Non-Goals` の形へ変換します。

[1]: https://peps.python.org/pep-0634/ "PEP 634 – Structural Pattern Matching: Specification"
[2]: https://peps.python.org/pep-0561/ "PEP 561 – Distributing and Packaging Type Information"
[3]: https://typing.python.org/en/latest/spec/special-types.html "Special types in annotations — Any"
[4]: https://typing.python.org/en/latest/spec/distributing.html "Distributing type information — typing documentation"
[5]: https://typing.python.org/en/latest/reference/protocols.html "Protocols and structural subtyping — typing documentation"
[6]: https://peps.python.org/pep-0647/ "PEP 647 – User-Defined Type Guards"
[7]: https://github.com/microsoft/pyright/blob/main/docs/typed-libraries.md "Pyright typed libraries guidance"
[8]: https://mypy.readthedocs.io/en/stable/stubtest.html "Automatic stub testing (stubtest)"
[9]: https://typeguard.readthedocs.io/en/latest/userguide.html "Typeguard user guide"
