# Release Checklist

Release gate checklist for Asatsuyu. The CI `release-gate` job aggregates all automated
gates; all must pass before a release can be tagged.

---

## Automated Gates (CI `release-gate` job)

All gates must be green before tagging on the `main` branch.

### Rust Quality

- [ ] `cargo fmt --all --check` — code formatting (`fmt` job)
- [ ] `cargo clippy --workspace --all-targets` — pedantic lint (`clippy` job)
- [ ] `cargo test --workspace` with `INSTA_UPDATE=no` — all tests (`test` job)

### Snapshot Stability

Verified within the `test` job:

- [ ] 32+ diagnostic snapshot tests (`diagnostic_snapshots.rs`)
- [ ] 54+ golden pipeline tests / 293+ snapshots (`golden.rs`)
- [ ] FFI surface snapshot (`ffi_conformance.rs`)
- [ ] Crash corpus inventory = 30 files (`crash_safety.rs`)

### FFI Contract

Verified within the `test` job + `verify-ffi` job:

- [ ] Trust summary: `3 Verified, 2 Checked, 0 Unsafe` (`e2e.rs`)
- [ ] Verified modules have no `Any` in type surfaces (`ffi_conformance.rs`)
- [ ] Symbol count regression guards pass (`ffi_conformance.rs`)
- [ ] `verify_ffi.py` pyright/stubtest pass for Verified modules (`verify-ffi` job x 3 Python versions)

### Fixture Projects

Verified within the `test` job + `package-install` job:

- [ ] All 5 fixtures pass check/build/run (`fixture_projects.rs`)
- [ ] 4 installable fixtures pass pip install smoke test (`package-install` job x 3 Python versions)

### Python Version Matrix

The following jobs must pass on Python 3.12, 3.13, and 3.14:

- [ ] `verify-ffi` — FFI trust report + pyright/stubtest
- [ ] `maturin-build` — wheel build
- [ ] `pytest` — Python test suite
- [ ] `package-install` — build, pip install, import

### Documentation

- [ ] `scripts/check_docs_sync.py` all checks pass (`docs-sync` job)

---

## Manual Review (before tagging)

Items not covered by automation. Verify before cutting a tag.

- [ ] CHANGELOG has an entry for this release
- [ ] `Cargo.toml` workspace version has been bumped
- [ ] No blocking TODO/FIXME items remain
- [ ] `cargo insta review` confirms no unintended snapshot diffs

---

## Branch Protection (recommended)

Configure the following required status check on the `main` branch in GitHub repository settings:

- `release gate` — the aggregate job for all automated gates

---

## Local Verification

```bash
# Full local release gate (CI equivalent + docs sync)
make release-gate
```
