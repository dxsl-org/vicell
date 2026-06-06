# Research — First-App Demo Candidates for ViCell (Lua / MicroPython / Rust)

**Ngày**: 2026-06-06
**Loại**: Research (feasibility + ranked shortlist)
**Câu hỏi**: app nào hay/port được sang ViCell làm first-app demo, viết bằng Lua / MicroPython / Rust?

---

## 0. Capability envelope (HARD constraints)
- **Headless**: chỉ serial console (ANSI escape OK). KHÔNG GUI/framebuffer/SDL/ncurses-termios.
- **Không thread/fork/dynamic-link/stdin**. `input()` / `io.stdin:read()` → treo cell.
- **Net**: TCP+UDP+DNS (SLIRP, gateway 10.0.2.2); HTTP/1.0 client+server + MQTT client đã có (Rust). ⚠️ **`vnet.listen/accept` CHƯA expose cho Lua/MicroPython** → script chỉ làm **client-mode**, không server.
- **Storage**: FAT16 `/data` + RamFS `/tmp`, file nhỏ (IPC ~480B/write). ⚠️ Lua script-load qua GetFile phục vụ RamFS/kernel-embedded (không FAT16 /data) → bake script qua format-disk.ps1.
- **Decoupling**: **script Lua/Py = decoupled nhất** (chỉ là data trên disk, interpreter đã ship → không rebuild kernel). Rust cell mới = disk-loaded /bin (build cell, không rebuild kernel).

## 1. Bảng tổng ứng viên (rút gọn)

### Lua (single-file only — `require` là stub)
| # | App | Effort | Demo value | Dùng |
|---|---|---|---|---|
| **L1** | **Conway's Game of Life** | **Low (30')** | ANSI animation, proof-of-life | compute |
| L2 | LuaForth (interpreter-in-interpreter) | Low | "ngôn ngữ chạy ngôn ngữ" | compute+VFS |
| L3 | TCP echo/line server | Low-Med | network service | net ⚠️listen |
| L4 | JSON pretty-printer / jq-lite | Low | embedded config tool | VFS+compute |
| L5 | HTTP telemetry poster | Low | sensor→HTTP host | VFS+net |

### MicroPython (json/struct/hashlib + vnet client)
| # | App | Effort | Demo value | Dùng |
|---|---|---|---|---|
| **P1** | **MQTT sensor simulator** | Med (~120 LOC MQTT framing) | canonical robot demo | VFS+json+net |
| P2 | Conway's Life | Low | visual | compute |
| P3 | HTTP metrics server | **High (blocked: no vnet.listen)** | — | — |
| P4 | Data logger (CSV→/data) | Low | sensor-logger persistence | VFS+struct+random |
| P5 | DNS/HTTP health-probe | Low | outbound net debug tool | net |

### Rust native cell (no_std + alloc)
| # | App | Effort | Demo value | Dùng |
|---|---|---|---|---|
| **R1** | **Telemetry aggregator + MQTT publisher** | Low (mở rộng mqtt.rs) | full robot pipeline | VFS+json+net |
| R2 | Tiny Forth cell | Med | language-in-OS | VFS+compute |
| R3 | Key-value config store (IPC service) | Low | real OS service | VFS+IPC server |
| R4 | Bench + sysinfo → POST results | Low (80% có sẵn) | perf credibility | compute+net |
| R5 | IRC bot / chat bridge | Med | "ViCell live on IRC" | net (stateful) |

## 2. Traps (trông port được nhưng KHÔNG)
- Python/Lua snake/2048 dùng **curses/termios/colorama/readchar** → không có tty layer.
- **umqtt.simple** không compile trong build → `import umqtt` fail.
- `input()` / stdin → treo.
- MicroPython **httpd** → `vnet.listen/accept` không có trong modvnet.c.
- `os.execute("clear"/"sleep")` trong Lua map sang command-runner, không phải subprocess → dùng `io.write("\027[2J")` + busy-wait.
- `sorth` crate (Rust Forth): maturity chưa rõ → tự viết ~200 LOC an toàn hơn.

## 3. Khuyến nghị — bộ first-app 2 tầng

**Tầng A — "Proof of life" (quick win, decoupled nhất):**
→ **L1 Conway's Game of Life (Lua)**. 30 phút, 0 dep, ANSI animation trên serial, chỉ cần bake .lua vào disk (không rebuild kernel/cell). Demo quay video được, chứng minh Lua cell + ANSI + loop end-to-end. Không phụ thuộc các câu hỏi chưa giải (không cần net/listen).

**Tầng B — "Embedded credibility" (G1 reference-demo software-half = graduation gate):**
→ **R1 Telemetry aggregator + MQTT publisher (Rust)** HOẶC **P1 MQTT sensor simulator (MicroPython)**.
- R1: mở rộng [mqtt.rs](../../cells/apps/net-tools/src/bin/mqtt.rs) + đọc config VFS + vòng publish; ~150 LOC, dùng serde-json-core (no_std). Vững nhất, đúng "robot telemetry pipeline".
- P1: ~120 LOC MQTT framing tay trong Python; demo "script điều khiển robot" hấp dẫn nhưng tốn công + heap 256KB cần tính.

→ Khớp đúng **G1 graduation gate** (sensor→compute→actuator + MQTT telemetry) ở [project-roadmap.md](../../docs/project-roadmap.md); phần GPIO thật chờ peripheral track, phần software (net+MQTT) làm được NGAY.

## 4. Câu hỏi cần xác minh trước khi cook tầng B
1. Lua/MicroPython có expose `vnet.listen/accept`? (researcher: MicroPython KHÔNG; Lua chưa chắc) → tầng B dùng **client-mode** (publish/connect) nên không chặn.
2. Heap MicroPython 256KB đủ cho json+MQTT frame+vnet buffer đồng thời? (tính trước nếu chọn P1).
3. Lua script-load từ FAT16 /data (GetFile FAT16 fallback) — hay phải RamFS/embedded? Ảnh hưởng cách ship script.

## 5. Verdict
- **Bắt đầu ngay**: L1 Conway (Lua) — rủi ro ~0, decoupled tuyệt đối, demo đẹp.
- **Tiếp theo**: R1 telemetry (Rust) — đóng góp trực tiếp G1 graduation gate, credibility embedded cao nhất.
- Tránh: mọi thứ server-mode trong script (listen/accept), curses/stdin, umqtt.
