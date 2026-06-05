# Phase 05 — Integration Tests & spawn_pinned API

**Status**: 📋 PLANNED  
**Priority**: P1  
**Effort**: 3 days  
**Depends on**: Phases 01–04 all complete

---

## Context Links

- Integration tests: `tests/` or `kernel/src/` (grep for `#[test]` and `integration`)
- Spawn syscall: `kernel/src/task/syscall.rs`
- Spawn from mem: `kernel/src/task.rs`
- TCB: `kernel/src/task/tcb.rs`
- Scheduler: `kernel/src/task/scheduler.rs`

---

## Overview

Three goals in this final phase:

1. **Integration tests** — verify the priority scheduler works correctly end-to-end
2. **Regression guard** — all 65 existing tests continue to pass
3. **`spawn_pinned(core_id)` API** — future-SMP hook; on single-core, `core_id != 0` returns `NotSupported`

---

## Integration Tests to Write

### Test 1: `rt_preempts_normal`
Scenario: spawn a Normal cell that runs an infinite loop, then spawn a RealTime cell. Verify the RT cell runs within ≤10 ms (one timer tick) and the Normal cell resumes after.

```rust
// Pseudo-code for integration test
fn test_rt_preempts_normal() {
    let t0 = hal::timer::read_mtime();
    let _normal_id = spawn_from_mem(NORMAL_LOOP_ELF, "loop", Normal);

    // Normal cell starts running (cooperative, loops forever)
    // Spawn RT cell — SSIP fires, preempts Normal
    let rt_id = spawn_from_mem(RT_RECORD_ELF, "rt_recorder", RealTime);

    // RT cell records its start time and exits
    let t1 = wait_for_exit(rt_id);

    // RT cell ran within 10ms of spawning
    assert!((t1 - t0) <= TICKS_PER_10MS * 2,
        "RT cell waited too long: {} ticks", t1 - t0);
}
```

### Test 2: `background_never_starves_normal`
Scenario: spawn Background + Normal cells. Verify Normal completes in bounded time even with a Background cell in the queue.

```rust
fn test_background_never_starves_normal() {
    spawn_from_mem(BG_SPIN_ELF, "bg_spin", Background);
    let normal_id = spawn_from_mem(NORMAL_COUNT_ELF, "normal_count", Normal);

    // Normal cell counts to 1000 and exits
    let elapsed = wait_for_exit_with_timeout(normal_id, TIMEOUT_TICKS);
    assert!(elapsed < TIMEOUT_TICKS, "Normal cell starved by Background");
}
```

### Test 3: `rt_heap_alloc_dealloc`
Verify RT pool allocates and frees correctly, pool does not exhaust on repeated spawn/exit.

```rust
fn test_rt_heap_alloc_dealloc() {
    for _ in 0..10 {
        let id = spawn_from_mem(RT_NOOP_ELF, "rt_noop", RealTime);
        wait_for_exit(id);
        // If pool was not freed, 11th spawn would OOM
    }
    // Reaches here → pool correctly recycled
}
```

### Test 4: `spawn_pinned_single_core`
Verify `spawn_pinned(0)` succeeds and `spawn_pinned(1)` returns `NotSupported`.

```rust
fn test_spawn_pinned_single_core() {
    let r0 = sys_spawn_pinned("/bin/shell", 0);
    assert!(r0.is_ok(), "spawn_pinned(0) must succeed on single-core");

    let r1 = sys_spawn_pinned("/bin/shell", 1);
    assert_eq!(r1, Err(ViError::NotSupported),
        "spawn_pinned(1) must fail on single-core");
}
```

### Test 5: `timer_tick_increments`
Verify `system_ticks()` advances after sleep — basic timer-wiring smoke test.

```rust
fn test_timer_tick_increments() {
    let before = system_ticks();
    sys_sleep(20); // sleep 20 ms = ~2 timer ticks
    let after = system_ticks();
    assert!(after >= before + 2,
        "timer ticks did not advance: before={} after={}", before, after);
}
```

---

## spawn_pinned API

### Syscall addition

Add `SYS_SPAWN_PINNED` opcode in `kernel/src/task/syscall.rs`:

```rust
// Opcode: pick next available (e.g. 0x1F or TBD)
SYS_SPAWN_PINNED => {
    let path_ptr = frame.a0;
    let path_len = frame.a1;
    let priority  = frame.a2 as u8;
    let core_id   = frame.a3 as usize;

    if core_id != 0 {
        // SMP not yet implemented — only core 0 exists
        frame.a0 = SyscallResult::Err(ViError::NotSupported) as usize;
        return;
    }

    // For single-core, spawn_pinned(0) == spawn_from_path
    let path = /* validate path from ptr+len */;
    match crate::loader::spawn_from_path_with_priority(path, priority) {
        Ok(tid) => frame.a0 = SyscallResult::Ok(tid) as usize,
        Err(e)  => frame.a0 = SyscallResult::from_err(e) as usize,
    }
}
```

### ostd binding

Add `sys_spawn_pinned(path: &str, core_id: usize) -> SyscallResult` in `libs/ostd/src/syscall.rs`.

---

## Related Code Files

### Create
- `tests/priority_scheduler_tests.rs` (or add to existing test module)

### Modify
- `kernel/src/task/syscall.rs` — add `SYS_SPAWN_PINNED` handler
- `libs/ostd/src/syscall.rs` — add `sys_spawn_pinned` binding

---

## Todo List

- [ ] Write `test_timer_tick_increments` integration test
- [ ] Write `test_rt_preempts_normal` integration test
- [ ] Write `test_background_never_starves_normal` integration test
- [ ] Write `test_rt_heap_alloc_dealloc` integration test
- [ ] Write `test_spawn_pinned_single_core` integration test
- [ ] Add `SYS_SPAWN_PINNED` syscall handler in `syscall.rs`
- [ ] Add `sys_spawn_pinned` in `libs/ostd/src/syscall.rs`
- [ ] Run all 65 existing integration tests — confirm zero regressions
- [ ] Run 5 new priority tests — all pass
- [ ] Update `docs/project-roadmap.md` Phase 25 status

---

## Success Criteria

- [ ] `test_timer_tick_increments` passes: ticks advance after sleep
- [ ] `test_rt_preempts_normal` passes: RT cell runs within ≤20 ms of spawn
- [ ] `test_background_never_starves_normal` passes: Normal completes in bounded time
- [ ] `test_rt_heap_alloc_dealloc` passes: 10× spawn/exit cycle without OOM
- [ ] `test_spawn_pinned_single_core` passes: core 0 OK, core 1 → NotSupported
- [ ] All 65 pre-existing tests pass (no regressions)
- [ ] `cargo test --all --release` green

---

## Risk Assessment

| Risk | Likelihood | Mitigation |
|------|-----------|------------|
| RT preemption test is timing-sensitive (QEMU speed varies) | Medium | Use 2× timer-tick margin (20 ms) instead of exact 10 ms |
| Background starvation test requires long timeout | Low | 100× tick timeout is generous; only fires on actual bug |
| spawn_pinned opcode conflicts with existing syscalls | Low | Audit existing opcode table before assigning new number |
| Test ELF binaries needed (RT_NOOP_ELF etc.) | Medium | Use existing `user_hello` pattern for tiny smoke-test cells |
