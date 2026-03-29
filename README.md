<h1 align="center">Asatsuyu</h1>

<p align="center">
  <strong>A statically typed language that compiles to Python.</strong>
</p>

<p align="center">
  <a href="LICENSE-MIT"><img src="https://img.shields.io/badge/license-MIT%2FApache--2.0-blue?style=flat-square" alt="License"></a>
</p>

---

> **Status: Early development.**
> Asatsuyu is a personal project exploring programming language design and
> implementation. The compiler is not yet functional — this repository
> contains the language design, architecture decisions, and work-in-progress
> implementation. Contributions, feedback, and design discussions are welcome.

---

Asatsuyu is an experiment in bringing sound static typing to the Python
ecosystem. Its syntax and semantics are heavily inspired by
[Gleam](https://gleam.run) — algebraic data types, exhaustive pattern matching,
the pipeline operator, `Result`/`Option`, and the principle that a language
should be small and have one obvious way to do things. Asatsuyu applies these
ideas to a Python compilation target, aiming to let you write safer code while
still calling Python libraries directly.

The name comes from the Japanese word **朝露** (morning dew) — something that
takes shape quietly at dawn, just as Asatsuyu aims to bring structure to the
dynamic world of Python.

```
pub fn main() {
  [1, -3, 5, -2, 8]
  |> list.filter(fn(x) { x > 0 })
  |> list.map(fn(x) { x * 2 })
  |> io.println
}
```

intended output:

```python
def main() -> None:
    _pipe0 = [x for x in [1, -3, 5, -2, 8] if x > 0]
    _pipe1 = [x * 2 for x in _pipe0]
    print(_pipe1)
```

## Motivation

Python's ecosystem is extraordinary — NumPy, pandas, PyTorch, Django, FastAPI,
and thousands more. But Python's type system is optional and gradual, which
means type errors surface at runtime rather than at compile time, and
refactoring large codebases requires significant care.

Asatsuyu explores an approach similar to what TypeScript did for JavaScript:

- **Write** in a language with sound static types and algebraic data types.
- **Compile** to clean, human-readable Python 3.12+ source code.
- **Run** on standard CPython with access to existing Python libraries.

The generated Python should look like code you could have written by hand —
dataclasses, `match`/`case`, PEP 695 generics, and nothing more.

## Planned Language Features

> These features reflect the language design. Implementation is in progress.

**Type inference.** Hindley-Milner type inference with let-polymorphism. Type
annotations are rarely needed — the compiler infers them — but every
expression has a known type at compile time.

**Algebraic data types.** Define variants, carry data, and match on them. The
compiler rejects incomplete matches.

```
pub type Shape {
  Circle(radius: Float)
  Rectangle(width: Float, height: Float)
}

pub fn area(shape: Shape) -> Float {
  match shape {
    Circle(r) -> 3.14159 * r * r
    Rectangle(w, h) -> w * h
  }
}
```

**Pipeline operator.** Chain transformations left-to-right instead of nesting
function calls.

```
pub fn process(data: List(String)) -> List(String) {
  data
  |> list.filter(fn(s) { string.length(s) > 3 })
  |> list.map(string.uppercase)
  |> list.sort
}
```

**Result and Option.** No null. No exceptions inside Asatsuyu. Errors are
values you handle explicitly.

```
pub fn parse_age(input: String) -> Result(Int, String) {
  match int.parse(input) {
    Ok(n) if n >= 0 && n <= 150 -> Ok(n)
    Ok(_) -> Error("age out of range")
    Error(_) -> Error("not a number")
  }
}
```

**Python interop.** Import Python libraries using `from python import`.
Asatsuyu reads type stubs (`.pyi` / typeshed) for type information. Python
exceptions are caught at the boundary and wrapped into `Result`.

```
from python import numpy as np
from python import requests

pub fn fetch_mean(url: String) -> Result(Float, String) {
  let response = try requests.get(url)
  let data = response.json()
  let values = np.array(data["values"])
  Ok(np.mean(values))
}
```

**Readable output.** Generated Python uses `@dataclass(frozen=True)`,
PEP 695 generics (`class Ok[T]`), `type` statements, and `match`/`case` —
idiomatic Python 3.12+ code.

## Design Goals

1. **Sound types over gradual typing.** Every Asatsuyu program that compiles
   should be free of type errors. No `Any` escape hatch in the language itself.

2. **Python is the runtime, not a second-class target.** Generated code should
   be idiomatic, readable, and debuggable.

3. **Python interop without a new declaration format.** Type information comes
   from the existing typeshed / PEP 561 ecosystem.

4. **Fast feedback.** The compiler is written in Rust. `asatsuyu check` is
   designed to be the fastest path — type-check without code generation.

5. **One way to do it.** Following Gleam's formatter philosophy, there is a
   single canonical style with zero configuration. Following Go's language
   design philosophy, the language is deliberately small.

6. **Helpful errors.** Inspired by Elm and Gleam, error messages should explain
   *what went wrong* and *how to fix it*.

## Non-Goals

- **No classes or inheritance.** Use algebraic data types and functions.
- **No exceptions.** Errors are `Result` values. Python exceptions are caught
  at the FFI boundary only.
- **No mutable variables.** All bindings are immutable.
- **No macros.** The language should be understandable without a
  meta-programming layer.
- **No bytecode generation.** We generate `.py` source files, not `.pyc`.
- **No multi-backend.** Python 3.12+ is the sole compilation target.
- **No effect system, dependent types, or refinement types.** The type system
  should be powerful enough to be useful and simple enough to learn in an
  afternoon.

## Architecture

The compiler is implemented as a Rust workspace with single-responsibility
crates. The pipeline has five stages: CST → AST → HIR → THIR → Python emitter.

```
Source (.asty)
  │
  ▼
asatsuyu-lexer ──────── Token stream (logos)
  │
  ▼
asatsuyu-parser ─────── Lossless CST (rowan, hand-written recursive descent)
  │
  ▼
asatsuyu-ast ────────── Untyped AST
  │
  ▼
asatsuyu-hir ────────── Name resolution, desugaring ◄── asatsuyu-ffi-python
  │                                                       (reads typeshed/.pyi)
  ▼
asatsuyu-ty ─────────── HM type inference → Typed HIR (THIR)
  │
  ▼
asatsuyu-backend-python  THIR → Python 3.12+ source
```

Cross-cutting crates:

| Crate | Role |
|---|---|
| `asatsuyu-syntax` | Shared types: token kinds, CST node kinds, `Span`, `Diagnostic` |
| `asatsuyu-cli` | Entry point: `check`, `build`, `run`, `fmt`, `test` |
| `asatsuyu-format` | CST-based formatter (preserves comments, zero config) |
| `asatsuyu-lsp` | Language Server Protocol via `tower-lsp` |

### Technical Decisions

- **Hand-written recursive descent parser** rather than a parser generator,
  following the approach proven by Ruff for better error recovery and
  performance.
- **Lossless CST with `rowan`** so the formatter and LSP can operate on the
  full syntactic structure including comments and whitespace.
- **`logos` for lexing** — compile-time DFA generation.
- **`miette` for diagnostics** — rich terminal error output with source
  snippets and suggestions.
- **`typeshed` as the FFI type source.** We parse `.pyi` stubs to understand
  Python library types rather than inventing a new declaration format.

## Project Configuration

```toml
# asatsuyu.toml
[project]
name = "my-app"
version = "0.1.0"

[python]
version = ">=3.12"

[python-dependencies]
numpy = ">=1.26"
requests = ">=2.31"
```

## CLI (Planned)

```sh
asatsuyu check   # Type-check only (fastest feedback loop)
asatsuyu build   # Generate Python package
asatsuyu run     # Build and execute
asatsuyu fmt     # Format source code (opinionated, zero config)
asatsuyu test    # Run tests
asatsuyu new     # Create a new project
```

## How It Compares

| | Asatsuyu | mypy / pyright | Cython | Mojo |
|---|---|---|---|---|
| **Approach** | New language → Python source | Type checker for Python | Python superset → C | Python superset → MLIR |
| **Type system** | Sound, inferred (HM) | Gradual, opt-in | Gradual + C types | Gradual + ownership |
| **Runtime** | CPython (no runtime) | CPython | Custom C extension | Custom runtime |
| **Output** | Readable `.py` files | N/A (checker only) | `.so` / `.pyd` | Binary |
| **Goal** | Correctness + ecosystem access | Catch bugs in Python | Performance | Performance |

Asatsuyu is not a type checker, a performance tool, or a Python superset. It is
a separate language that targets Python as its compilation backend — similar to
how Gleam targets Erlang or Elm targets JavaScript.

## Roadmap

The MVP goal is to write a 300–500 line CLI application in Asatsuyu using
`Result`, `Option`, `match`, ADTs, and typed calls to `requests` and `pathlib`,
producing readable Python 3.12+ output.

- [x] Language design and compiler architecture
- [ ] Workspace setup and CI
- [ ] Lexer and parser
- [ ] AST and HIR (name resolution, desugaring)
- [ ] Hindley-Milner type inference
- [ ] ADT typing and match exhaustiveness
- [ ] Python backend (dataclass, match/case, prelude)
- [ ] CLI (`check`, `build`, `run`)
- [ ] FFI via typeshed (`pathlib`, `json`, `os`, `sys`, then `requests`)
- [ ] MVP sample application
- [ ] Formatter and LSP (post-MVP)

## Contributing

This is my first programming language project, so I'm learning as I go.
Feedback on the language design, compiler architecture, or implementation
approach is genuinely appreciated.

For design discussions, open a
[Discussion](https://github.com/Kaikei-e/asatsuyu/discussions).
For bugs or concrete improvements, open an
[Issue](https://github.com/Kaikei-e/asatsuyu/issues).

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup and guidelines.

## License

Licensed under either of

- [MIT License](LICENSE-MIT)
- [Apache License, Version 2.0](LICENSE-APACHE)

at your option.

## Influences

Asatsuyu does not exist in a vacuum. Its design borrows explicitly from several
languages and tools, and this section documents those debts honestly.

**[Gleam](https://gleam.run)** is the single strongest influence on Asatsuyu's
design. The surface syntax (`pub fn`, `type ... { }`, `match`, `|>`, `<>`),
the ADT-centric data model, `Result`/`Option` as first-class error handling,
the opinionated zero-config formatter, and the philosophy of "one way to do it"
are all drawn directly from Gleam. Where Gleam compiles to Erlang and
JavaScript, Asatsuyu targets Python — but the language-level ideas are Gleam's.

**[Elm](https://elm-lang.org)** shaped the approach to error messages (explaining
*what went wrong* and *how to fix it*) and reinforced the commitment to
immutability and sound types without escape hatches.

**[Go](https://go.dev)** influenced the tooling philosophy: a single binary
with `build`, `run`, `fmt`, and `test` built in, a deliberately small language
surface, and the conviction that a language's value comes partly from what it
leaves out.

**[F#](https://fsharp.org)** informed the pipeline-oriented programming style
and the idea that a functional language can coexist productively with a large
existing runtime ecosystem (as F# does with .NET). The `|>` operator itself
originates in the ML family; F# popularized it in a mainstream context.

**[Ruff](https://github.com/astral-sh/ruff)** and
**[rust-analyzer](https://rust-analyzer.github.io)** are the primary
references for compiler engineering decisions — hand-written recursive descent
parsing, lossless CST with `rowan`, `logos` for lexing, `miette` for
diagnostics, and the general approach to incremental analysis in Rust.

**[TypeScript](https://www.typescriptlang.org)** established the precedent
that a new statically typed language can succeed by compiling to an existing
dynamic language's source code and embracing its ecosystem wholesale. Asatsuyu
follows this strategy for Python.

The Python ecosystem — and the communities that maintain
[typeshed](https://github.com/python/typeshed), CPython, and thousands of
libraries — makes this project possible.