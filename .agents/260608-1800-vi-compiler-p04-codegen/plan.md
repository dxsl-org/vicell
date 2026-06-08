# ViUI v2 P04 — vi-compiler: Expression Evaluator + Rust Codegen

**Plan ID**: 260608-1800-vi-compiler-p04-codegen
**Stage**: G2
**Priority**: P1 — required before build.rs integration (P05)
**Created**: 2026-06-08
**Depends on**: P02 (libs/viui Layer 2 API) + P03 (vi-compiler lexer + parser)
**Design Brief**: [.agents/brainstorms/260608-viui-nextgen-architecture.md](../brainstorms/260608-viui-nextgen-architecture.md)

---

## Mục tiêu

Hoàn thành vi-compiler pipeline: từ `.vi` DSL → compilable Rust source.

1. **Expression evaluator** (`eval.rs`) — parse `Expr::Raw` text into `TypedExpr` variants
2. **Rust codegen** (`codegen.rs`) — walk `ViFile` AST → emit Rust source using P02 Layer 2 API
3. **`Computed::into_parts()`** — expose inner signal from `Computed<T>` (needed by generated code)

**End-to-end**: `counter.vi` → `vi-compiler` → compilable Rust that instantiates a working `Counter` widget tree.

---

## Scope

### In scope
- Expression evaluator: 7 expression types used by counter.vi
- Codegen: component struct, `build()` method, element → widget mapping
- `Computed::into_parts()` added to `libs/viui/src/signal.rs`
- Integration test: codegen output compiles + contains correct patterns

### Out of scope
- Full Slint expression language (binary ops, ternary, property access chain)
- Multi-component files
- Import resolution
- Build.rs integration (P05)
- Diagnostics / source maps

---

## Key Design Decisions

### Manual subscription vs `into_parts()`

`Computed<T>::signal` is private → can't pass it to `Label::new(Signal<String>)` directly.

**Chosen approach**: add `into_parts() -> (Signal<T>, SubscriptionHandle)` to `Computed<T>`.
- Non-breaking addition (new method, not in `libs/api/`)
- Cleaner generated code vs verbose manual subscription
- Natural capability gap — makes `Computed` properly usable

Generated code for interpolated text:
```rust
let (text_0, _sub_text_0) = count.map(|n| alloc::format!("Count: {}", n)).into_parts();
let w_text_0 = Label::new(text_0).with_color(Color::rgb(0xff, 0xff, 0xff));
```

### no_std + alloc target for generated code

Generated Rust targets ViCell Cell context (no_std + alloc):
- `alloc::format!` not `format!`
- `alloc::vec![...]` not `vec![...]`
- `alloc::boxed::Box` explicitly qualified
- Requires `extern crate alloc;` at top of generated file

### Element → widget type mapping

| `.vi` name | Rust type | Import path |
|------------|-----------|-------------|
| `VerticalLayout` / `VBox` | `Column` | `viui::node_widgets::column` |
| `HorizontalLayout` / `HBox` | `Row` | `viui::node_widgets::row` |
| `Text` / `Label` | `Label` | `viui::node_widgets::label` |
| `Button` | `Button` | `viui::node_widgets::button` |

Unknown elements: emit `// TODO: unsupported element {name}` comment, skip.

### Property type → Rust type mapping

| `.vi` type | Rust type | Initial signal |
|------------|-----------|----------------|
| `int` | `i32` | `Signal::new(0i32)` |
| `float` | `f32` | `Signal::new(0.0f32)` |
| `string` | `String` | `Signal::new(alloc::string::String::new())` |
| `bool` | `bool` | `Signal::new(false)` |
| `color` | `Color` | `Signal::new(Color::TRANSPARENT)` |
| `length` | `f32` | `Signal::new(0.0f32)` |

---

## Phase Table

| Phase | File | Nội dung | Status |
|-------|------|----------|--------|
| P01 | [phase-01-expr-evaluator.md](phase-01-expr-evaluator.md) | TypedExpr + eval functions + unit tests | ✅ Done |
| P02 | [phase-02-codegen.md](phase-02-codegen.md) | Computed::into_parts() + CodeGen + integration tests | ✅ Done |

P02 depends on P01 (uses TypedExpr from eval.rs in codegen).

---

## Files to Create/Modify

```
tools/vi-compiler/src/
├── eval.rs        (NEW) expression evaluator
├── codegen.rs     (NEW) Rust source emitter
└── lib.rs         (MODIFY) add pub mod eval; pub mod codegen;

tools/vi-compiler/tests/
└── codegen_tests.rs   (NEW) integration tests

libs/viui/src/
└── signal.rs      (MODIFY) add Computed::into_parts()
```

---

## Generated Output Contract (counter.vi)

`vi-compiler counter.vi` must produce Rust source that:
1. Compiles without errors in a `no_std + alloc` crate importing `viui`
2. Defines `pub struct Counter { pub count: Signal<i32>, ... }`
3. Defines `Counter::build() -> (Counter, Column)`
4. Creates a `Column` with one `Label` and one `Button` child
5. `Label.text` updates reactively when `count` changes
6. `Button` click callback increments `count`

---

## Success Criteria

- [ ] `cargo test --manifest-path tools/vi-compiler/Cargo.toml` → all tests pass (P01 unit + P02 integration)
- [ ] Codegen output for counter.vi contains `Signal::new(0i32)`, `Label::new`, `Button::new`, `Column::new`
- [ ] `Computed::into_parts()` compiles: `cargo check -p viui`
- [ ] CLI: `cargo run --manifest-path tools/vi-compiler/Cargo.toml -- tests/fixtures/counter.vi` prints Rust source to stdout
