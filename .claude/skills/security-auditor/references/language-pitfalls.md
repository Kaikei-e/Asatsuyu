# Language-Specific Security Pitfalls

Asatsuyu のスタックに合わせた危険パターン集。各項目は **grep → なぜ危険 → 安全な書き方** の 3 点セット。
grep はリポジトリルートから実行する。

**出典ポリシー:** 各項目は OWASP Top 10:2025 / ASVS 5.0 / 公式言語ドキュメントに準拠。

## Contents
- [Rust（コンパイラ・CLI・LSP）](#rustコンパイラclilsp)
- [生成 Python（emitter の出力）](#生成-pythonemitter-の出力)
- [PyO3 ランタイム](#pyo3-ランタイム)
- [TypeScript（VS Code 拡張）](#typescriptvs-code-拡張)
- [使い方](#使い方)

---

## Rust（コンパイラ・CLI・LSP）

### Panic on untrusted input
```
grep -rnE '\.unwrap\(\)|\.expect\(|panic!|unreachable!|todo!|\[[a-z_]+\]' --include='*.rs' asatsuyu/crates/asatsuyu-{lexer,parser,ast,hir,ty,backend-python}/src
```
- **危険:** `.asty` / `.pyi` / `asatsuyu.toml` はユーザ入力。入力由来の値で panic すると、CLI は異常終了し
  LSP はサーバーごと落ちる（A10:2025、CWE-248 / CWE-754）。スライスの添字アクセスと整数変換も同じ。
- **安全:** 入力由来の失敗は `Diagnostic` に載せる。`expect` は「落ちたらコンパイラのバグ」の文脈だけ。
  再発防止は `crates/asatsuyu-cli/tests/corpus/crash-<stage>-<desc>.asty` に入力を足す。

### Unbounded recursion / resource exhaustion
```
grep -rnE 'fn (parse|lower|infer|check|emit|unify)_[a-z_]+' --include='*.rs' asatsuyu/crates | head -40
```
- **危険:** 再帰下降パーサと木を歩く各段は、深くネストした入力でスタックを使い切る（CWE-674）。
  型推論は病的な入力で時間・メモリが膨らむ（CWE-400）。LSP では 1 文書が全体を止める。
- **安全:** ネスト深さの上限と診断、occurs check の維持、LSP 側のキャンセル・タイムアウト。

### Command injection / argument injection
```
grep -rn 'Command::new' --include='*.rs' asatsuyu/crates
```
- **危険:** shell を経由しなくても、ユーザ由来の文字列が**引数の先頭**に来ると `-` 始まりの値が
  オプションとして解釈される（CWE-88）。`asatsuyu add <name> <spec>` の値が `uv` / `pip` に渡る経路が該当。
  `Command::new("sh").arg("-c")` は入力がそのままシェルに届く（CWE-78）。
- **安全:** shell を使わない。引数は 1 要素ずつ `.arg()` で渡す。パッケージ名・バージョン指定は
  PEP 503 / PEP 440 として**パースしてから**渡し、位置引数の前に `--` を置けるコマンドでは置く。

### Executable resolution via PATH
```
grep -rnE 'Command::new\("(python3?|uv|pip|which)"\)' --include='*.rs' asatsuyu/crates
```
- **危険:** 素の名前での起動は `PATH` を信頼する（CWE-426）。カレントディレクトリや書き込み可能な
  ディレクトリが `PATH` にあると別物が走る。`--python` で渡されたパスはそのまま実行される。
- **安全:** 解決した絶対パスを保持して使い回す。解決結果を診断・ログに出して利用者が確認できるようにする。
  「どのインタプリタを実行したか」は隠さない。

### Path traversal / unintended overwrite
```
grep -rnE 'fs::(write|create_dir_all|remove_dir_all|remove_file|rename)|\.join\(' --include='*.rs' asatsuyu/crates/asatsuyu-cli/src
```
- **危険:** `asatsuyu.toml` の `name`、モジュール名、出力ディレクトリがパスの部品になる。`..` や絶対パス、
  シンボリックリンクを含むと、プロジェクト外への書き込み・削除になる（CWE-22 / CWE-59）。
  `remove_dir_all` は取り返しがつかない。
- **安全:** パス部品にする前に名前を検証する（Python の識別子規則 + 予約語）。出力先は正規化して
  プロジェクトルート配下であることを確認してから書く。削除は自分が生成したと分かるものだけ。

### LSP: URI → path
```
grep -rnE 'to_file_path|Url::' --include='*.rs' asatsuyu/crates/asatsuyu-cli/src/lsp
```
- **危険:** クライアントが送る URI は `file:` とは限らず、ワークスペース外も指せる（CWE-22）。
- **安全:** `file` スキーム以外は拒否。読み書きの対象はワークスペース配下に限定する。

### Unsafe
```
grep -rnE 'unsafe|allow\(unsafe_code\)' --include='*.rs' asatsuyu/crates
```
- **危険:** `unsafe` は UB / memory corruption の入口（CWE-119 系）。ワークスペースは `unsafe_code = "deny"`。
- **安全:** 許可された例外は 2 箇所だけ — `asatsuyu-syntax` の `SyntaxKind` 変換（`raw < __LAST` を assert した上での
  transmute）と `asatsuyu-runtime-python`（PyO3 マクロ）。それ以外に `allow(unsafe_code)` が増えていたら finding。
  既存の 2 箇所は `// SAFETY:` の不変条件が今も成り立つかを確認する（enum に variant を足したとき `__LAST` が末尾のままか）。

### Deserialization of project files
```
grep -rnE 'toml::from_str|serde_json::from_|toml_edit' --include='*.rs' asatsuyu/crates
```
- **危険:** `#[serde(untagged)]` は入力次第で型の取り違えを起こす。未知フィールドの黙認は設定の打ち間違いを隠す。
- **安全:** `#[serde(deny_unknown_fields)]` を検討。値は読み込み直後に検証し、検証済みの型に詰め替える。

---

## 生成 Python（emitter の出力）

コンパイラ固有の最重要領域。`.asty` に書かれた文字列・名前は、生成 Python の**コードの一部**になる。

### String literal escaping
```
grep -rnE 'format!\(.*\\"\{|push_str\(.*"' --include='*.rs' asatsuyu/crates/asatsuyu-backend-python/src | head -40
```
- **危険:** 文字列リテラルを素朴に `"{}"` で包むと、`"` `\` 改行、f-string 化される場合の `{` `}` を含む値で
  リテラルが閉じ、後続が任意の Python として実行される（A05:2025、CWE-94）。文字列補間の脱糖が該当しやすい。
- **安全:** エスケープは 1 箇所の関数に集約し、`"` `\` 制御文字・`{` `}` を網羅する。
  golden case に「引用符・バックスラッシュ・波括弧・改行を含む文字列」を入れて snapshot で固定する。

### Identifier sanitization
```
grep -rn 'sanitize_python_name' --include='*.rs' asatsuyu/crates/asatsuyu-backend-python/src
```
- **危険:** Asatsuyu の識別子が Python の予約語・組み込み名（`class`、`list`、`__import__`）と衝突すると、
  構文エラーか、組み込みの上書きによる意味の変化になる。import されるモジュール名は
  `import <name>` の形で埋め込まれるので、識別子として妥当でない値はそのままコードになる。
- **安全:** 名前を生成コードに出す経路がすべてサニタイズ関数を通っていること。素通しの経路を探す。

### Comments and source maps
- **危険:** `# asty:L<n>` や元ソース由来のテキストをコメントに埋めるとき、改行を含む値はコメントを抜けて
  次の行がコードになる。
- **安全:** コメントに入れる値は数値か、改行を除去した文字列だけ。

### Dangerous constructs in output
```
grep -rnE '\b(eval|exec|compile|__import__|pickle|subprocess|os\.system)\b' asatsuyu/crates/asatsuyu-backend-python/src
```
- **危険:** 生成コードやランタイム shim がこれらを含むと、生成物を読む人の信頼前提（憲章 §3 原則 5）が崩れる。
- **安全:** 動的実行を使わない。`importlib` での遅延 import も、名前がコンパイル時定数であることを確認。

### Exception boundary
- **危険:** Checked FFI のラッパーが `except Exception` で握りつぶして成功値を返すと、失敗が隠れる（A10:2025）。
  逆に `BaseException` まで捕まえると `KeyboardInterrupt` / `SystemExit` を飲む。
- **安全:** 例外は `Result` の `Err` に変換して値として返す（憲章 §3 原則 2）。捕捉範囲は `Exception` まで。

---

## PyO3 ランタイム

```
grep -rnE '#\[py(function|class|methods|module)\]|extract\(|downcast' --include='*.rs' asatsuyu/crates/asatsuyu-runtime-python/src
```
- **危険:** Python から来る値は任意の型。`extract` の失敗を `unwrap` すると Python プロセスごと abort する。
  検証をすり抜けた値が `Verified` 扱いで Asatsuyu 側に入ると、健全性の前提が崩れる（憲章 §4.1）。
- **安全:** 失敗は `PyResult` の `Err` で返す。検証関数は「通したら型が保証される」ことをテストで固定する
  （`tests/test_ffi_conformance.py`、`tests/test_runtime.py`）。再帰的な検証には深さ上限を置く。

---

## TypeScript（VS Code 拡張）

### Server binary resolution
```
grep -rnE 'ServerOptions|command:|workspace\.getConfiguration' editors/vscode/src
```
- **危険:** サーバーのパスをワークスペース設定（`.vscode/settings.json`）から読むと、細工したリポジトリを
  開くだけで任意のバイナリが起動する（CWE-829）。
- **安全:** 実行ファイルのパスは user 設定スコープに限定するか、Workspace Trust が無効な間は既定のバイナリだけを使う。
  `package.json` の `capabilities.untrustedWorkspaces` の宣言を確認する。

### eval / dynamic require
```
grep -rnE '\beval\(|new Function\(|require\([^"'"'"']' editors/vscode/src
```
- **危険:** 入力次第で任意コード実行。
- **安全:** 使わない。設定値は型を検証してから使う。

### Dependencies and packaging
- **注意:** `.vsix` に同梱されるのは esbuild のバンドル結果。`pnpm-lock.yaml` の drift、post-install script を持つ
  依存の追加、`--no-dependencies` で意図せず欠ける実行時依存が無いかを見る。

---

## 使い方

1. 対象を `Glob` で特定
2. 上記 grep を回して hit を列挙
3. hit ごとに context を Read で確認
4. 誤検知を除いた残りを report の Findings に起票（OWASP カテゴリ + ASVS ID を必ず添える）

grep で hit しない = 安全ではない（書き方を変えれば同等に危険なので、パターンはあくまで入口）。
重要なのは **データフロー** と **信頼境界** の確認。
