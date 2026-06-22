# Cellos Input Service API

The Input Service Cell (`cells/services/input/`) translates raw VirtIO input
events into structured `InputEvent`s and dispatches them to the focused cell.

---

## Architecture

```
Hardware (QEMU VirtIO keyboard/mouse)
      │ IRQ
      ▼
Kernel virtio_input driver
      │ IPC Send (raw EV_KEY message)
      ▼
Input Cell (cells/services/input/)
      ├─ layout_us_qwerty: scancode → KeySym + Unicode char
      ├─ modifier_state:   Shift / Ctrl / Alt / Lock tracking
      ├─ dispatcher:       routes InputEvent to focused cell
      ▼
Focused Cell (shell, compositor surface, …)
      │ IPC Recv (encoded InputEvent)
      ▼
  handles KeyEvent variants
```

---

## Inbound IPC (kernel → input cell)

The kernel VirtIO input driver sends a fixed 64-byte message per event.

| Offset | Size | Field | Description |
|--------|------|-------|-------------|
| 0 | 1 | opcode | `0x00` = EV_KEY; `0x20` = SetFocus |
| 1 | 4 | scancode | evdev EV_KEY code (u32 LE) |
| 5 | 4 | value | 0=release, 1=press, 2=repeat (u32 LE) |

**SetFocus message** (`opcode = 0x20`):

| Offset | Size | Field | Description |
|--------|------|-------|-------------|
| 0 | 1 | opcode | `0x20` |
| 1 | 8 | endpoint | Target cell task ID (u64 LE) |

---

## Outbound IPC (input cell → focused cell)

The input cell sends a 65-byte message for each translated event.

| Offset | Size | Field | Description |
|--------|------|-------|-------------|
| 0 | 1 | opcode | `0x10` = INPUT_EVENT_OPCODE |
| 1 | 64 | payload | `encode_event()` output (see below) |

### `encode_event` payload format

Discriminant byte at `payload[0]` selects the variant:

**`InputEvent::Key` (discriminant = 0):**

| Offset | Size | Field |
|--------|------|-------|
| 0 | 1 | discriminant = 0 |
| 1 | 8 | timestamp_ticks (u64 LE) |
| 9 | 4 | scancode (u32 LE) |
| 13 | 4 | keysym as u32 (u32 LE) |
| 17 | 4 | character (Unicode, u32 LE; 0 = non-printable) |
| 21 | 1 | modifiers bitmask |
| 22 | 1 | state (0=released, 1=pressed, 2=repeated) |

**`InputEvent::MouseMove` (discriminant = 1):**

| Offset | Size | Field |
|--------|------|-------|
| 0 | 1 | discriminant = 1 |
| 1 | 4 | x absolute (i32 LE) |
| 5 | 4 | y absolute (i32 LE) |
| 9 | 4 | dx relative (i32 LE) |
| 13 | 4 | dy relative (i32 LE) |

**`InputEvent::MouseButton` (discriminant = 2):**

| Offset | Size | Field |
|--------|------|-------|
| 0 | 1 | discriminant = 2 |
| 1 | 1 | button (0=Left, 1=Right, 2=Middle) |
| 2 | 1 | state (0=released, 1=pressed) |

---

## Modifier Bitmask

| Bit | Flag | Key(s) |
|-----|------|--------|
| 0 | SHIFT | Left Shift / Right Shift |
| 1 | CTRL | Left Ctrl / Right Ctrl |
| 2 | ALT | Left Alt / Right Alt |
| 3 | META | Left Meta / Right Meta |
| 4 | CAPS_LOCK | Caps Lock (toggle) |
| 5 | NUM_LOCK | Num Lock (toggle) |
| 6 | SCROLL_LOCK | Scroll Lock (toggle) |

---

## KeySym Values

| Value | Name | Notes |
|-------|------|-------|
| 0x0001 | Escape | |
| 0x0002 | Return | Enter key |
| 0x0003 | Backspace | |
| 0x0004 | Tab | |
| 0x0005 | Delete | Del key |
| 0x0010–0x0013 | Up/Down/Left/Right | Arrow keys |
| 0x0020–0x0023 | Home/End/PageUp/PageDown | |
| 0x0101–0x010C | F1–F12 | |
| 0x8000 | Printable | `character` field contains Unicode |

---

## Focus Routing

At boot, the Input Cell forwards all events to the **shell cell** (default
endpoint ID 3).  The Compositor (Phase 16) changes focus by sending a
`SetFocus` message to the Input Cell.

Rules:
- When focus changes, **transient modifiers** (Shift/Ctrl/Alt/Meta) are cleared
  to prevent "stuck key" state.
- Lock keys (Caps/Num/Scroll Lock) survive focus changes.
- If the focused cell's IPC queue is full, the event is dropped with a log warning.

---

## OSTD Convenience (future)

Phase 17 will expose a typed `read_key() -> KeyEvent` helper in
`libs/ostd/src/input.rs`.  Until then, consumer cells decode the raw 65-byte
IPC message using `api::input::InputEvent`.

---

## Files

| File | Purpose |
|------|---------|
| `libs/api/src/input.rs` | Type definitions (`InputEvent`, `KeyEvent`, `Modifiers`, …) |
| `cells/services/input/src/lib.rs` | Cell entry point + IPC receive loop |
| `cells/services/input/src/layout_us_qwerty.rs` | Scancode → KeySym table |
| `cells/services/input/src/modifier_state.rs` | Modifier state machine |
| `cells/services/input/src/dispatcher.rs` | Focus-based event routing |
| `tests/integration/input_dispatch.rs` | QEMU-driven integration tests |
