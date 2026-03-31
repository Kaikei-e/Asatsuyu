# Fixture Projects

Executable fixture projects for integration testing and documentation.
Each directory is a complete Asatsuyu project with `asatsuyu.toml` and `src/main.asty`.

## Projects

| Fixture | FFI Level | Description |
|---|---|---|
| `hello_cli` | None | Pure Asatsuyu: ADT, match, pipeline, string concat |
| `pathlib_walk` | Verified | stdlib FFI using `pathlib` |
| `stdlib_ffi` | Verified | stdlib FFI using `os` and `sys` |
| `requests_client` | Checked | Third-party FFI using `requests` with `try` |
| `build_install` | None | Full pipeline: build, package, install |

## Usage

```bash
# Type-check a fixture
asatsuyu check fixtures/projects/hello_cli/src/main.asty

# Build to Python package
asatsuyu build fixtures/projects/hello_cli/src/main.asty -o /tmp/hello_cli_dist

# Compile and run
asatsuyu run fixtures/projects/hello_cli/src/main.asty
```

## CI

These fixtures are tested automatically via `cargo test -p asatsuyu-cli --test fixture_projects`.
Tests cover `check`, `build`, and `run` for each fixture.
Network-dependent tests (`requests_client` run) are `#[ignore]`.
