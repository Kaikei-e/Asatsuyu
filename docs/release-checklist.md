# Release Checklist

Asatsuyu のリリースゲート一覧。CI の `release-gate` ジョブがすべての自動ゲートを集約し、
全 PASS でなければリリース不可とする。

---

## Automated Gates (CI `release-gate` job)

`main` ブランチでタグを切る前に、すべてのゲートが green であること。

### Rust Quality

- [ ] `cargo fmt --all --check` — コード整形 (`fmt` job)
- [ ] `cargo clippy --workspace --all-targets` — pedantic lint (`clippy` job)
- [ ] `cargo test --workspace` with `INSTA_UPDATE=no` — 全テスト (`test` job)

### Snapshot Stability

`test` job 内で検証:

- [ ] 32+ diagnostic snapshot tests (`diagnostic_snapshots.rs`)
- [ ] 54+ golden pipeline tests / 293+ snapshots (`golden.rs`)
- [ ] FFI surface snapshot (`ffi_conformance.rs`)
- [ ] Crash corpus inventory = 30 files (`crash_safety.rs`)

### FFI Contract

`test` job + `verify-ffi` job 内で検証:

- [ ] Trust summary: `3 Verified, 2 Checked, 0 Unsafe` (`e2e.rs`)
- [ ] Verified modules have no `Any` in type surfaces (`ffi_conformance.rs`)
- [ ] Symbol count regression guards pass (`ffi_conformance.rs`)
- [ ] `verify_ffi.py` pyright/stubtest pass for Verified modules (`verify-ffi` job × 3 Python)

### Fixture Projects

`test` job + `package-install` job 内で検証:

- [ ] All 5 fixtures pass check/build/run (`fixture_projects.rs`)
- [ ] 4 installable fixtures pass pip install smoke (`package-install` job × 3 Python)

### Python Version Matrix

以下のジョブが Python 3.12, 3.13, 3.14 の全バージョンで pass:

- [ ] `verify-ffi` — FFI trust report + pyright/stubtest
- [ ] `maturin-build` — wheel build
- [ ] `pytest` — Python テストスイート
- [ ] `package-install` — build → pip install → import

### Documentation

- [ ] `scripts/check_docs_sync.py` 全チェック pass (`docs-sync` job)

---

## Manual Review (before tagging)

自動化されていない項目。タグを切る前に人間が確認する。

- [ ] CHANGELOG に今回のリリースのエントリがある
- [ ] `Cargo.toml` workspace version がバンプ済み
- [ ] ブロッキングな TODO/FIXME がない
- [ ] `cargo insta review` で意図しない差分がないことを確認済み

---

## Branch Protection (recommended)

GitHub リポジトリ設定で `main` ブランチに以下の required status check を設定することを推奨:

- `release gate` — 全自動ゲートの集約ジョブ

---

## Local Verification

```bash
# Full local release gate (CI equivalent + docs sync)
make release-gate
```
