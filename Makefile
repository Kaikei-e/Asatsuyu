.PHONY: dev dev-release build-wheel test check fmt verify-ffi pytest ci docs-sync release-gate

# Rust workspace directory
CARGO_DIR := asatsuyu
RUNTIME_DIR := $(CARGO_DIR)/crates/asatsuyu-runtime-python

# ── Local development ──────────────────────────────────────────────

## Install native runtime into active virtualenv (debug)
dev:
	cd $(RUNTIME_DIR) && maturin develop

## Install native runtime into active virtualenv (release)
dev-release:
	cd $(RUNTIME_DIR) && maturin develop --release

# ── Build ──────────────────────────────────────────────────────────

## Build manylinux wheel
build-wheel:
	cd $(RUNTIME_DIR) && maturin build --release --out dist

# ── Quality ────────────────────────────────────────────────────────

## Format all Rust code
fmt:
	cd $(CARGO_DIR) && cargo fmt --all

## Run clippy with warnings as errors
check:
	cd $(CARGO_DIR) && cargo clippy --workspace --all-targets -- -D warnings

## Run all Rust tests
test:
	cd $(CARGO_DIR) && cargo test --workspace

## Emit FFI trust report and run external verification
verify-ffi:
	cd $(CARGO_DIR) && cargo run -p asatsuyu-cli -- verify-ffi
	@if [ -f scripts/verify_ffi.py ]; then python scripts/verify_ffi.py; fi

## Run Python tests (requires maturin develop first)
pytest:
	pytest tests/ -v

# ── CI equivalent ──────────────────────────────────────────────────

## Run the full CI pipeline locally (fmt → check → test → dev → pytest → verify-ffi)
ci: fmt check test dev pytest verify-ffi

# ── Release gate ──────────────────────────────────────────────────────

## Run the docs sync check
docs-sync:
	python scripts/check_docs_sync.py

## Run the full release gate locally (ci + docs-sync)
release-gate: ci docs-sync
	@echo "RELEASE GATE: PASS"
