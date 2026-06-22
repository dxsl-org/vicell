# Cellos Patterns
**Version**: 0.2.1-dev | **Last Updated**: 2026-06-05

Design patterns, idioms, and architectural decisions for Cellos development.
Detailed content lives in `docs/patterns/` sub-files — this file is the index.

**Philosophy**: Safety First · Zero-Copy · Explicit · Fail Fast · Portable

---

## Pattern Categories

| Category | File | Contents |
|----------|------|----------|
| **Architectural** | [patterns/architectural.md](patterns/architectural.md) | Layered architecture, trait-based abstraction, dependency injection |
| **Concurrency** | [patterns/concurrency.md](patterns/concurrency.md) | Spinlock with interrupt safety, global singleton, priority scheduler |
| **Memory** | [patterns/memory.md](patterns/memory.md) | Owned buffers for async, Arc sharing, RAII, per-cell quota |
| **Error Handling** | [patterns/error-handling.md](patterns/error-handling.md) | Result-based errors, error context, panic policy, three-level model |
| **API Design** | [patterns/api-design.md](patterns/api-design.md) | Builder, Newtype, Facade, typed IPC channels, ZST capabilities |
| **Initialization** | [patterns/initialization.md](patterns/initialization.md) | Two-phase init, init-per-module, panic recovery per cell |
| **Testing** | [patterns/testing.md](patterns/testing.md) | Unit mocks, arch validation tests, QEMU integration, fault injection |
| **Anti-Patterns** | [patterns/anti-patterns.md](patterns/anti-patterns.md) | Unsafe in cells, mod.rs, hardcoded sizes, borrowed async buffers |
| **Research Learnings** | [patterns/research-learnings.md](patterns/research-learnings.md) | Tock, Hubris, RTIC, RustyHermit, RedLeaf, Singularity, Midori — with repos & papers |

---

## Quick Reference

### The 8 Coding Laws (non-negotiable)
See [code-standards.md](code-standards.md) for full enforcement rules.

| Law | Rule |
|-----|------|
| 1 | `libs/api/` changes require 2× user confirmation |
| 2 | No `&mut [u8]` across async boundaries — use `Box<[u8]>` |
| 3 | Use `VAddr`/`PAddr`, never raw `u64` for addresses |
| 4 | `#![forbid(unsafe_code)]` in all Cells |
| 5 | No `mod.rs` — use `foo.rs` parallel to `foo/` |
| 6 | `Vi` prefix for public traits and core types |
| 7 | `dyn Trait + Send + Sync` for polymorphism |
| 8 | Implement `Drop` for all resources |

### Pattern Decision Matrix

| Scenario | Pattern | File |
|---|---|---|
| Global state | `Spinlock<Option<T>>` | [concurrency.md](patterns/concurrency.md) |
| Shared resource | `Arc<dyn Trait>` | [memory.md](patterns/memory.md) |
| Resource cleanup | RAII + `Drop` | [memory.md](patterns/memory.md) |
| Async I/O buffer | `Box<[u8]>` ownership | [memory.md](patterns/memory.md) |
| Error propagation | `Result<T, ViError>` | [error-handling.md](patterns/error-handling.md) |
| Invariant violation | `panic!` / `expect()` | [error-handling.md](patterns/error-handling.md) |
| Multi-impl interface | Trait + vtable | [architectural.md](patterns/architectural.md) |
| Type safety | Newtype (`VAddr`, `PAddr`) | [api-design.md](patterns/api-design.md) |
| Capability grant | ZST token *(Phase 26)* | [api-design.md](patterns/api-design.md) |
| IPC protocol | Typed enum *(Phase 27)* | [api-design.md](patterns/api-design.md) |

---

## Research-Driven Patterns (Planned)

Patterns not yet implemented — derived from comparative OS research:

| Phase | Pattern | Learn From |
|-------|---------|-----------|
| 25 | Static priority preemption via SWI | RTIC v2 |
| 26 | Per-cell memory grant | Tock OS |
| 26 | ZST capabilities-as-types | Midori |
| 27 | Synchronous IPC + auto-expiring Lease | Hubris |
| 27 | Typed IPC channels (enum, not bytes) | Singularity |
| 28 | RISC-V ePMP per-cell bounds | Iso-UniK |

Full details with repos and papers: [patterns/research-learnings.md](patterns/research-learnings.md)

---

**Key Takeaways**:
1. `Spinlock<Option<T>>` for all global state
2. `Box<T>` / `Arc<T>` transfer — never borrows across Cell boundaries
3. Implement `Drop` for every resource handle
4. `Result` for errors, `panic!` only for kernel invariants
5. Traits for abstraction, newtypes for type safety
