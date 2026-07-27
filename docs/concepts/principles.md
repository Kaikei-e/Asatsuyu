# Asatsuyu Language Charter

## 0. Purpose of This Document

This charter defines the top-level principles governing all design decisions in Asatsuyu.
Every decision regarding syntax, the type system, FFI, code generation, tooling, and the roadmap must conform to this charter.

This is not a specification enumerating implementation details.
Its purpose is to fix what Asatsuyu is, what it protects, and what it discards. Asatsuyu's immediate focus is delivering a single, end-to-end statically typed Python frontend.

This charter is the single top-level authority. There is no second concept document; individual decisions that interpret or revise it are recorded as ADRs under [`docs/adr/`](../adr/), and the charter text is amended in the same change.

> **Note:** This document is a design charter — it describes the principles and goals that guide Asatsuyu's development. The MVP criteria in section 10 are targets, not claims about the current implementation state.

---

## 1. Definition

Asatsuyu is a **statically typed, functional, compiled application language for the Python ecosystem**.

Asatsuyu's purpose is to foreground the following qualities — which Python alone makes difficult to achieve — while preserving Python's library and operational assets:

* Clarity of design
* Visibility of failure
* Safety of branching
* Explicitness of data shapes
* Order at boundaries

Asatsuyu is not a language designed to replace Python.
Nor is it a language that merely adds thin type hints to Python.
Asatsuyu is **a language that raises the quality of the application layer while remaining connected to the reality of Python**.

---

## 2. Mission

Asatsuyu's mission is to enable safer, more transparent application logic while preserving Python's assets.

Asatsuyu does not pursue raw performance as its primary goal.
The evaluation axes for the initial phase are:

* Quality of type error messages
* Readability of generated code
* Low friction when leveraging Python assets

This priority order is fixed.

---

## 3. Core Principles

### Principle 1: Clarity over brevity

Asatsuyu prioritizes being hard to misread and hard to break over being short to write.
Types, branches, failures, and data shapes must be visible in the code wherever possible.

Asatsuyu is not a language that looks clever — it is a language that restores visibility.

### Principle 2: Failures are never hidden

In Asatsuyu's internal world, failures are carried by `Result` as a rule.
Failures must be made explicit as types, not delegated to implicit exception propagation.

Exceptions may exist only at the Python boundary.
Uncontrolled exception propagation must not be brought into Asatsuyu's pure domain.

### Principle 3: Data takes center stage

The center of Asatsuyu is not class/inheritance.
What occupies the center is:

* Immutable data
* Algebraic data types
* Functions
* Pattern matching

Rather than encapsulating behavior in objects, Asatsuyu prioritizes making data shapes explicit and handling them with functions.
Asatsuyu does not aim to reinvent OOP.

### Principle 4: Do not antagonize Python

Asatsuyu is not a language that rejects Python.
It is a complementary language that supplements the rigor Python tends to lack.

Therefore, Asatsuyu's value is measured not by its disconnection from Python, but by the quality of its connection to Python.

### Principle 5: Protect transparent compilation output

Asatsuyu does not hide that it compiles to Python.
Rather, **it leverages the compilation to Python as a form of transparency**.

Generated Python code must be human-readable, traceable, and reviewable when needed.
Debuggability, observability, and CPython compatibility are protected on the main track.

---

## 4. Language Boundaries

Asatsuyu's success is not determined solely by its internal type system.
**The boundary design with Python must be elegant, narrow, and explicit.**

Asatsuyu's FFI follows these principles:

1. Python functions must be importable with type information.
2. Type information preferentially uses existing infrastructure: `.pyi`, stub packages, `py.typed`, typeshed, and PEP 561. ([PEP 561][2])
3. A proprietary declaration format must not be invented as a first step.
4. Python exceptions are lifted to `Result` at the boundary.
5. `Any` is treated as a dangerous type and must not silently flow into the safe world. `Any` is mutually assignable with all types; passing it through boundaries without restriction breaks soundness. ([Typing spec: Any][3])
6. Dynamic features such as `__getattr__`, `eval`, and monkey patching are isolated in the dangerous zone. ([Typing spec: distributing][4])
7. Complete soundness for arbitrary Python libraries is not a first-year goal.
8. FFI is handled in three tiers: `Verified` / `Checked` / `Unsafe`.
9. Surfaces containing partial stub packages or unknowns must not enter `Verified`.
10. Rather than trusting entire large concrete classes directly, prefer `Protocol`, `TypedDict`, and explicit validators. ([Protocols][5]) ([PEP 647][6])

### 4.1 FFI Soundness Model

Asatsuyu's internal world aims to be sound.
However, that soundness must not be unconditionally extended beyond the Python boundary.
PEP 561 provides an ordering for type information distribution but does not guarantee the correctness of stubs themselves. Therefore Asatsuyu is designed as a language that protects **sound core + verified boundary**. ([PEP 561][2])

#### Verified FFI

Only the following qualify as `Verified`:

* Type information can be resolved via `py.typed`, stub packages, or typeshed
* No `Any`, bare generics, or partial-stub-derived unknowns remain in the exported surface
* Type completeness checks can be integrated into CI
* Stub/runtime divergence checks can be sustained

Only symbols that have entered `Verified` are treated as normal types in Asatsuyu. Pyright's `--verifytypes` can be used for type completeness verification, and mypy's `stubtest` can check for divergence between stubs and runtime implementations. ([Pyright --verifytypes][7]) ([stubtest][8])

Currently supported Verified modules: `pathlib`, `os`, `sys`.

#### Checked FFI

`Checked` is the tier for surfaces that have static type information but cannot be treated as sound as-is.
In this tier, compiler-generated wrappers must perform at least:

* Argument validation
* Return value validation
* Exception-to-`Result` conversion
* Dynamic value narrowing

APIs with JSON- or dict-based return values, such as `requests.Response.json()`, are placed in this tier. Runtime validators initially leverage existing Python typing assets and may transition to generated validators as needed. ([Typeguard][9])

Currently supported Checked modules: `requests`, `json`.

#### Unsafe / Opaque FFI

`Unsafe` designates surfaces that must not enter the sound world.
Values in this tier are isolated as opaque types such as `PyOpaque[module.Symbol]`, and field access, pattern matching, and implicit conversion are forbidden.
The only permitted operations are passing the value to another foreign call that accepts the same opaque type, and explicit checked conversion via boundary functions.

Planned for future phases: `numpy` (Checked/Opaque-first), `pandas` and `torch` (Opaque-first).

### 4.2 Type Priority at the Boundary

For the MVP, the following types are prioritized for acceptance:

* `int`, `float`, `str`, `bool`, `None`
* `list[T]`, `tuple[...]`, `dict[K, V]`
* `Literal`, `Union`, `Optional`
* `TypedDict`
* Fully-known generic classes
* Narrowly-scoped `Protocol`

Conversely, the following must not enter `Verified`:

* `Any`
* Generics without type arguments
* Partial stub packages
* `__getattr__`-dependent surfaces
* APIs whose types do not close without a plugin

### 4.3 FFI Scope for the MVP

The initial phase targets the following:

* `pathlib`, `os`, `sys` — Verified
* `json` — Checked (contains `Any`)
* `requests` — Checked
* `numpy` — Checked / Opaque-first (planned)
* `pandas`, `torch` — Opaque-first (planned)

---

## 5. Compilation Target

Asatsuyu's primary backend is **fixed to Python source code generation**.
This must not waver, at least during the first year.

The reasons are:

* Preserves high debuggability
* Preserves observability
* Maximizes CPython compatibility
* Allows humans to audit the generated output
* Builds trust as a transitional language

The runtime target is Python 3.12+.
Asatsuyu assumes Python's `match`/`case` and the new type parameter syntax / `type` statement, ensuring conciseness and naturalness of generated code. ([PEP 634][1])

---

## 6. Core Grammar and Semantics

Asatsuyu must not be a kitchen-sink language.
The core to protect first is:

### 6.1 Expression-oriented

Expressions are preferred over statements.
However, an extreme expression language is not the goal.
If readability suffers, brevity is sacrificed.

### 6.2 ADT-centric

The core is not fragmented record/tuple/enum features but **algebraic data types with variants**.

### 6.3 Pattern match-centric

`match` is not an auxiliary syntax.
It is the primary syntax for data decomposition, stronger than `if`. Since structural pattern matching is already specified on the Python side, the approach of naturally projecting Asatsuyu's `match` to Python is sound. ([PEP 634][1])

### 6.4 Nullability containment via `Option`

The equivalent of `None` must not be scattered.
Nullable values are confined to `Option` and handled in the type world.

### 6.5 Failure representation via `Result`

Failures are represented using `Result`.
Everyday try/catch culture must not be brought into Asatsuyu's internals.
`try` is limited to exception absorption at the Python boundary.

### 6.6 Desugaring completes by HIR

Syntactic sugar such as pipeline and string concatenation is permitted.
However, they must be desugared by HIR, so the type inference system can operate without knowledge of syntactic conveniences.

---

## 7. Type System Responsibilities

Asatsuyu's type system is not merely an annotation display mechanism.
The type system is the core mechanism that supports design itself.

At minimum, the type system must satisfy:

* Hindley-Milner type inference
* Occurs check
* Let-polymorphism
* ADT typing
* `match` exhaustiveness checking
* Unreachable arm detection
* Error diagnostics showing "expected type / actual type"

Asatsuyu's type system exists to raise the quality of everyday design.
Excessive theoretical apparatus for research purposes must not be brought into the first-year core.

---

## 8. Non-Goals

Asatsuyu does not target the following as first-year central goals:

* `.pyc` direct generation as the primary pipeline
* JIT / native optimization
* Classes / inheritance / trait-like abstractions
* Macro systems
* Package registry
* Multi-backend support
* Dependent types / refinement types
* Mutable variables beyond scoped locals (`let mut` is limited to local bindings)

Two entries need a precise statement, because a loose reading of either would decide
questions this charter has not decided.

**Effect systems.** What is excluded is a *type-and-effect system*: carrying effect
variables in function types and unifying them. This is a permanent exclusion, not a
deferral — effect variables leaking into type errors would contradict the first evaluation
axis in section 2, and effect polymorphism sits outside the Hindley-Milner scope that
section 7 fixes. Distinguishing pure functions from effectful ones is *not* excluded; the
mechanism for that distinction is undecided. See [ADR 0001](../adr/0001-effect-system-permanent-non-goal.md).

**Concurrency.** `async` / `await` and the built-in `Task(T)` type are implemented, and the
emitter produces `async def` and `asyncio.run`. What remains undecided is whether
`async` / `await` stays as the surface syntax or is replaced by structured-concurrency
primitives. Neither answer is a non-goal. What this charter does exclude here is the pursuit
of parallel throughput: concurrency exists to express ordering, not to gain speed
(see section 2 — speed is not an evaluation axis).

Furthermore, items explored as research tracks must not encroach on the main line.
Python bytecode direct generation, automatic stub generation, and Rust native extensions are deferred until after the MVP.

---

## 9. Target Users and Application Scope

Asatsuyu's primary target is developers who meet the following criteria:

* Use Python professionally or in personal projects
* Want type safety
* Do not want a migration as heavy as Rust
* Find the advantages of functional programming appealing
* But do not want to abandon Python assets

Domains where Asatsuyu should be strong initially:

* CLI tools
* API clients
* JSON / HTTP processing
* Data transformation
* Batch processing
* Lightweight domain logic
* Application layers that include Python library calls

Conversely, the following are not initial battlegrounds:

* Ultra-low-level processing
* Core of high-performance numerical computation
* Complex async infrastructure
* Code dependent on Python metaprogramming
* Deep integration with large frameworks

---

## 10. MVP Definition

Asatsuyu's MVP is achieved when the following are met:

* A 300–500 line CLI can be written in Asatsuyu
* `Result` / `Option` / `match` / ADT can be used practically
* `pathlib` can be called as Verified FFI and `requests` as Checked FFI
* Readable Python 3.12+ packages can be generated
* Type error quality, generated code readability, and friction when leveraging Python assets reach a sufficient level

Speed must not be used as the MVP acceptance criterion.

---

## 11. Design Decision Criteria

Future feature additions, specification changes, and optimization decisions are evaluated with the following questions:

1. Does this feature strengthen the order of failures, branches, data, and boundaries?
2. Does this feature widen the bridge to Python?
3. Does this feature preserve the transparency of generated Python?
4. Does this feature strengthen the MVP's target domains?
5. Or does it merely make the language flashy?

Features that fall under the fifth question must, as a rule, not be adopted.

---

## 12. Closing

Asatsuyu is a language that provides a static, clear foundation for thinking against the dynamic, rich real world of Python.

Asatsuyu's value is not rigor itself.
Asatsuyu's value lies in **bringing order to failures, branches, data, and boundaries, and restoring visibility**.

Changes that violate this principle, no matter how attractive they appear, are rejected in light of this charter.

[1]: https://peps.python.org/pep-0634/ "PEP 634 – Structural Pattern Matching: Specification"
[2]: https://peps.python.org/pep-0561/ "PEP 561 – Distributing and Packaging Type Information"
[3]: https://typing.python.org/en/latest/spec/special-types.html "Special types in annotations — Any"
[4]: https://typing.python.org/en/latest/spec/distributing.html "Distributing type information — typing documentation"
[5]: https://typing.python.org/en/latest/reference/protocols.html "Protocols and structural subtyping — typing documentation"
[6]: https://peps.python.org/pep-0647/ "PEP 647 – User-Defined Type Guards"
[7]: https://github.com/microsoft/pyright/blob/main/docs/typed-libraries.md "Pyright typed libraries guidance"
[8]: https://mypy.readthedocs.io/en/stable/stubtest.html "Automatic stub testing (stubtest)"
[9]: https://typeguard.readthedocs.io/en/latest/userguide.html "Typeguard user guide"
