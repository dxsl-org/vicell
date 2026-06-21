# Kết luận cốt lõi
ViCell đã chọn đúng stack cơ sở (smoltcp, RedoxFS, littlefs2, Lua 5.4, embedded-tls). Cơ hội "ăn theo" thực sự nằm ở 3 lớp:

Layer 1 — Unlock ecosystem (effort thấp, impact cực cao)
embedded-hal v1 align GPIO/SPI/PWM/ADC → mở khoá hàng trăm sensor/display/actuator driver trên crates.io. Hiện I2C đã align, còn lại chưa.
embassy-sync vào ostd → Channel, PubSubChannel, Mutex tốt hơn hand-rolled. Executor-agnostic, drop-in.
Layer 2 — Fill gaps (1–3 tuần mỗi cái)
chacha20poly1305 + sha2 → crypto primitives ViCell đang thiếu hoàn toàn (code signing, HMAC, Silo fallback)
glam + minipng + raw-window-handle + tiny-skia → graphics stack từ "custom pixel pusher" lên "real compositor"
mcutie MQTT + hickory-proto DNS → 2 networking showcase dễ làm, blog-worthy
Layer 3 — Portfolio events (tháng tới)
SQLite Cell → proof "VFS này là real OS VFS" — được reference trên HN, tech blogs
Quake 1 (-nosound) → community event lớn hơn DOOM, cùng Tier 1b path

===============


Rust no_std / OS Ecosystem Opportunities for ViCell
Scope verdict: ViCell already uses the two best choices in the ecosystem (smoltcp, RedoxFS). The remaining opportunities split cleanly into "integrate now" (embassy-sync), "align traits" (embedded-hal), and "watch / defer" (everything else).

1. Embassy — Async Runtime Ecosystem
Verdict: Three crates are immediately useful to ViCell today; the rest are Cortex-M/nRF-specific noise.

What's forkable / integrable:

embassy-sync (MIT/Apache-2.0) — executor-agnostic Channel, PriorityChannel, PubSubChannel, Mutex, RwLock, Signal, Pipe, Semaphore. Its docs explicitly state "futures from this crate can run on any executor." Dependencies: critical-section, heapless, futures-core, embedded-io-async — all already present in ViCell's ostd. This is a direct drop-in for ViCell's inter-Cell async communication layer.
embassy-time (MIT/Apache-2.0) — Instant, Duration, Timer with a pluggable time-driver trait. ViCell would implement one TimeDriver backed by the kernel's 10 MHz tick timer. Porting effort: Easy — one trait impl, ~50 LOC.
embassy-executor (MIT/Apache-2.0) — cooperative async executor. Requires a custom platform "pender" (a __pender extern callback). ViCell already has a custom runtime; this is a replacement candidate, not an add-on. Only worth adopting if ViCell ever wants to unify its executor with the embedded driver ecosystem. Porting effort: Medium — would require rewiring AppContext task dispatch.
embassy-net — wraps smoltcp with a background task + VirtIO driver channel. ViCell already has smoltcp directly. Adding embassy-net means running smoltcp inside an embassy task rather than a Cell's hand-rolled poll loop. Skip for now; the abstraction adds complexity without benefit given ViCell's Cell architecture.
Repository status: 9.4k GitHub stars, 15,650 commits, active (multiple commits per week as of June 2026). MIT/Apache-2.0. Zero adoption risk.

Porting effort summary: embassy-sync = Easy (add to ostd deps, zero architecture change). embassy-time = Easy (one driver impl). embassy-executor = Medium (rewire runtime). embassy-net = Skip.

Source: embassy-rs/embassy · embassy-sync docs

2. RTIC — Real-Time Interrupt-driven Concurrency
Verdict: The #[rtic::app] macro framework is architecturally incompatible with ViCell's SAS Cell model, but rtic-sync is a usable primitive library independently of the macro.

rtic-sync (MIT/Apache-2.0) — Channel (MPSC, no-alloc), Arbiter (FIFO mutex), Signal. These do NOT require #[rtic::app]; make_channel! creates static instances. Functionally overlaps with embassy-sync but is narrower. Given embassy-sync is already executor-agnostic and richer, prefer embassy-sync over rtic-sync.
rtic-monotonics — timer abstractions. Duplicates embassy-time functionality. Skip.
The RTIC framework itself — requires a hardware interrupt priority table, hard-coded for Cortex-M NVIC. RISC-V "support" means limited backends, not parity. Architecturally: RTIC uses hardware interrupt priority for task preemption, which conflicts with ViCell's Cellular preemption model (SAS + kernel-controlled task scheduling). Do not integrate.
Repository status: 2.3k stars, MIT/Apache-2.0, actively maintained. RISC-V is second-class (limited backend, Cortex-M is primary).

Porting effort: rtic-sync as standalone = Easy (but embassy-sync is a better choice). Full RTIC = Incompatible.

Source: rtic-rs/rtic · rtic-sync docs

3. embedded-hal — The Driver Trait Ecosystem
Verdict: ViCell already imports embedded-hal v1 in hal/traits/i2c. The opportunity is to align ALL ViCell HAL peripheral traits against embedded-hal v1 to unlock the crates.io driver ecosystem.

The scale of the prize:

embedded-hal v1.0 (released January 9, 2024) defines stable I2c, SpiDevice, OutputPin, InputPin, Pwm, Adc traits.
The awesome-embedded-rust repo lists hundreds of platform-agnostic drivers for sensors, displays, motor controllers, RTC chips, ADCs — all built on these traits.
Reverse-dependency count on crates.io is large (thousands of crates depend on embedded-hal).
Current ViCell state: hal-i2c wraps embedded-hal v1. But hal-gpio, hal-spi, hal-pwm, hal-adc, hal-can define their own trait shapes NOT derived from embedded-hal. This means no off-the-shelf driver crate works without adaptation.

Concrete action: Add embedded-hal = "1" to hal-gpio, hal-spi, hal-pwm, hal-adc and implement embedded-hal::digital::OutputPin / embedded-hal::spi::SpiDevice / etc. on ViCell's concrete driver types. Cost: Medium (one alignment per peripheral type, ~100 LOC each). Payoff: every sensor, display, and actuator driver on crates.io becomes usable inside a ViCell driver Cell.

Note on async: embedded-hal-async 1.0 defines async I2c, async SpiDevice — same story, same alignment needed.

License: MIT/Apache-2.0. Zero risk.

Source: rust-embedded/embedded-hal · embedded-hal blog

4. smoltcp — Network Stack
Verdict: ViCell is already on smoltcp 0.11 (Dec 2023). Two versions behind latest (0.13, March 2025). Upgrade is the action, not integration.

Current state:

ViCell uses smoltcp 0.11 in service-net with TCP, UDP, DHCPv4, ICMPv4, IPv4.
v0.13.0 adds IPv6 SLAAC, zero-window-probe TCP, retransmit fixes, RFC compliance improvements, Rust Edition 2024.
v0.12.0 added DNS socket support improvements.
Missing features that smoltcp will never have: TLS (covered by embedded-tls, already in ViCell's net service), multicast broadcast (not in ViCell's current feature list), QUIC, HTTP/2. These are application-layer gaps, not smoltcp's scope.

Known limitations that affect ViCell: No selective ACKs (SACK), no PLPMTU, no 802.1Q VLAN. All are G2+ concerns.

Upgrade effort: smoltcp 0.11 → 0.13 involves API changes to socket handling and interface poll. The smoltcp project has a history of minor breaking changes per release. Estimate: Easy to Medium (1-2 days of API adaptation).

Repository status: 4.5k stars, 0BSD license (maximally permissive), v0.13.0 current, active.

Source: smoltcp-rs/smoltcp

5. Rust OS Projects — Fork Candidates
5a. Redox OS Components (already partially integrated)
Verdict: RedoxFS is already vendored. The next Redox component worth evaluating is relibc.

RedoxFS — already in third_party/redoxfs. MIT license. Status: continue current vendoring.
relibc — POSIX C stdlib written in Rust. MIT license. ViCell currently builds its own C runtime (G1 DONE per project memory: libm/stdio/setjmp, 9 submodules) and is on track for mlibc (G2 Tier B). relibc is an alternative to mlibc with a Rust implementation — less production-tested than musl but more idiomatic. Given mlibc is already integrated, relibc offers no marginal benefit. Skip.
Redox kernel IPC/syscall patterns — Redox uses a URL-based resource model. Architecturally incompatible with ViCell's numeric syscall + Cell IPC model. Skip.
Source: redox-os/redoxfs · relibc

5b. Theseus OS
Verdict: Architecturally most similar to ViCell (SAS + intralingual isolation), but too tightly coupled to extract components. Better as a research reference than a code source.

3.2k stars, MIT license, ~1,263 commits on theseus_main.
Theseus's memory crate (frame_allocator, MappedPages) is architecturally interesting but assumes x86_64-only memory model and uses Theseus's own type-state system that cannot be lifted out without pulling the entire kernel.
Its libtheseus and tlibc are intimately coupled to Theseus's cell (called "module") loading system.
No practical fork candidates. Read the OSDI 2020 paper for the intralingual design rationale; ViCell already implements the same philosophy independently.
Porting effort: Hard (too coupled). Treat as prior art, not code donor.

Source: theseus-os/Theseus · OSDI 2020 paper

5c. Asterinas
Verdict: MPL-2.0 license creates a file-level copyleft obligation. The OSTD concept is interesting but architecturally solves a different problem (unsafe confinement in a Linux-ABI kernel), not ViCell's (language-based isolation in SAS).

4.7k stars, v0.18.0 (June 3, 2026), RISC-V Tier 2. Academically impressive (USENIX ATC 2025, SOSP 2025 Best Paper).
OSTD is their framekernel unsafe boundary — it confines unsafe to a TCB that other kernel code can depend on for safe abstractions. ViCell achieves the same via Cell #![forbid(unsafe_code)] + kernel/HAL split. Different mechanism, same goal.
MPL-2.0 implication: Forking any OSTD file and shipping it in ViCell means that file must remain MPL-licensed and source-disclosed on modification. ViCell is MPL-2.0 itself (per ostd/Cargo.toml and kernel/Cargo.toml) — licenses match. Technically compatible, but the conceptual overlap is low.
CortenMM (Best Paper winner) — novel page table management. Worth reading the paper. Porting effort would be Hard (x86-64-first, not RISC-V-first).
Porting effort: OSTD as inspiration = free. Direct code fork = Hard (different arch, different IPC model, MPL propagation).

Source: asterinas/asterinas · Asterinas Book

5d. DragonOS
Verdict: GPL-2.0 is a license poison pill for ViCell. Disqualified.

1.2k stars, GPL-2.0, latest release v0.4.0 (December 22, 2025).
Linux-ABI compatibility focus. Serverless/cloud target — the opposite of ViCell's embedded/SAS direction.
GPL-2.0 means any derived work must also be GPL-2.0. ViCell is MPL-2.0. Mixing in GPL code would force the entire project to GPL-2.0 at minimum. Do not fork any DragonOS code.
Source: DragonOS-Community/DragonOS

Ranked Recommendation
Rank order by immediate ROI for ViCell:

1. Align ViCell HAL traits against embedded-hal v1 (Medium effort, maximum driver ecosystem unlock)

Add embedded-hal = "1" impls to hal-gpio, hal-spi, hal-pwm, hal-adc.
Every sensor/display/actuator driver on crates.io becomes usable in a driver Cell.
2. Add embassy-sync to ostd (Easy, no architecture change)

Replace or supplement ViCell's hand-rolled IPC channel primitives with embassy-sync::channel::Channel, PubSubChannel, Mutex.
Zero license risk, executor-agnostic, already compatible with ViCell's heapless + critical-section dep tree.
3. Upgrade smoltcp 0.11 → 0.13 (Easy to Medium)

Gains TCP correctness fixes and IPv6 SLAAC. 0BSD license. Should be a routine dependency bump.
4. Implement embassy-time driver (Easy, future-proofs peripheral driver compat)

One TimeDriver impl backed by kernel's 10 MHz tick exposes embassy_time::Timer::after() to driver Cells without requiring the full embassy executor.
5. Defer everything else

RTIC macro: architecturally incompatible.
embassy-executor: only if ViCell wants to converge its runtime with the embedded ecosystem (G2+ decision).
Theseus/Asterinas code: read the papers, do not port code.
DragonOS: GPL-2.0, disqualified.
Limitations of This Research
embedded-hal driver count not confirmed to a precise number (crates.io pagination prevented a definitive count; "thousands" is directionally correct based on reverse-dep pages).
smoltcp 0.11 → 0.13 API diff not enumerated; actual breakage scope requires reading the changelogs against ViCell's service-net/src/.
embassy-sync integration feasibility assumes ViCell's critical-section impl is already registered; if not, one additional impl is needed (trivial, ~10 LOC).
RTIC RISC-V backend maturity not independently verified beyond the upstream claim of "most RISC-V devices supported."
Sources:

embassy-rs/embassy
embassy-sync docs
rtic-rs/rtic
rtic-sync docs
rust-embedded/embedded-hal
embedded-hal v1 release blog
smoltcp-rs/smoltcp
theseus-os/Theseus
asterinas/asterinas
Asterinas Book
DragonOS-Community/DragonOS
redox-os/redoxfs
Ferrous Systems — Embedded Concurrency Patterns
Agent 1 (Rust OS/embedded ecosystems) xong rồi — kết quả rất rõ. Đang chờ 4 agent còn lại...

I now have full context on ViCell's actual graphics architecture. Here is the structured research report.

Research: Graphics & UI Library Candidates for ViCell no_std
Architectural baseline: ViCell runs #![no_std] + alloc. Its compositor owns a physical framebuffer and issues Grant-backed shared ViSurface slices to app cells. ViUI renders widget trees directly to &mut [u8] via a DrawTarget → FramebufferRenderer chain. The stack already uses fontdue for glyph rasterization. No GPU pipeline exists today; wgpu is not wired in. Research covers what the team could drop in, fork, or learn from — not what is already in use.

CATEGORY A: SOFTWARE RASTERIZERS
tiny-skia is the strongest drop-in 2D rasterizer for ViCell's compositor
Verdict: no_std-capable CPU rasterizer (BSD-3) that would give ViCell's compositor anti-aliased paths, gradients, and clipping without a GPU — the exact gap between the current DrawTarget pixel-pusher and a real 2D graphics engine.

Supports no_std via default-features = false + no-std-float feature; SIMD (SSE2/AVX2, NEON, WASM SIMD128) is a separable feature
Minimal dependencies: no GPU runtime, no font engine, no codec beyond optional PNG
BSD-3 license — no viral risk, compatible with ViCell's MIT license policy
Actively maintained under linebender (same org as Vello/Masonry); 1.6k stars, ~316 commits, 2026 activity
Used as the canonical consumer by resvg — real production coverage for correctness
Binary size adds ~200 KiB — acceptable for a compositor cell, not for micro-cells
Trade-off: CPU-only, no GPU path. Rendering large dirty regions at 60 fps on QEMU will saturate a single RISC-V virtual core. Acceptable for G1 (kiosk/dashboard at 30 fps, 800×480); revisit for G2 desktop.

Source: github.com/linebender/tiny-skia · crates.io/tiny-skia

embedded-graphics is the right primitive layer for micro-cells and driver-tier widgets
Verdict: True no_std + no_alloc iterator-based 2D drawing; implements a DrawTarget trait that is structurally identical to ViUI's current renderer contract — zero-migration cost.

#![no_std], no allocator required — works on constrained cells (GPIO dashboard, status bar)
DrawTarget trait: implement once on ViSurface/FramebufferRenderer, all e-g primitives render for free
Huge ecosystem of display drivers already implement this trait (ST7789, ILI9341, etc.) — useful when ViCell cells target physical embedded boards (G1 robot goal)
MIT + Apache-2.0 dual license; actively maintained, 1k+ stars
Does not replace ViUI; it adds geometry primitives (circles, rounded rects, thick strokes) without a widget model — complementary
Trade-off: No text shaping, no animations, no retained tree. This is a draw-call API, not a widget toolkit.

Source: github.com/embedded-graphics/embedded-graphics · docs.rs/embedded-graphics

CATEGORY B: FONT RENDERING
fontdue (already in ViCell) is the correct choice — no change needed
Verdict: Already deployed. Fastest pure-Rust TrueType/OpenType rasterizer; no_std + alloc; last release 0.9.3 (Feb 2025); MIT license.

Uses ttf-parser for parsing (also no_std + alloc), giving broad OTF/TrueType coverage
Designed as replacement for rusttype and ab_glyph; lower end-to-end latency than both
The existing GlyphAtlas pattern in ViUI is exactly the correct usage model
Abandonment risk: moderate. Single-author project (mooman219). No shaping — acceptable for Latin/CJK glyph atlas; not acceptable for Arabic/Indic (irrelevant for G1)
Trade-off: If G2 requires full Unicode shaping (Arabic RTL, Indic clusters), the path is cosmic-text (harfbuzz-based), but that crate pulls in std and is not appropriate for G1.

Source: github.com/mooman219/fontdue · crates.io/fontdue 0.9.3

ab_glyph: viable fallback, not an upgrade over fontdue
Verdict: no_std + alloc; MIT; faster than rusttype for OTF. But fontdue already outperforms it end-to-end. No reason to switch.

Actively maintained (alexheretic); last release 2024
Narrower API surface than fontdue (no layout engine built in)
Would be useful only if fontdue were abandoned and as a migration target
Source: github.com/alexheretic/ab-glyph

CATEGORY C: IMAGE DECODING
minipng is the correct choice for ViCell's no_std PNG decoding
Verdict: No dependencies (not even alloc), correctly decodes all non-interlaced PNGs, ~9× smaller wasm size than the png crate; MIT license; Rust 2024 edition; no panics, minimal unsafe.

True no_std + no_alloc — caller provides the output buffer; no heap allocation path whatsoever
Handles all color types in 8-bit non-interlaced files — covers icon/UI asset loading
~2× slower than the png crate for large images; for UI assets (small PNGs) performance is equivalent
Active maintenance confirmed 2025 (Rust 2024 edition update)
Limitation: does not decode interlaced PNG; does not handle >8-bit or 16-bit depth
Trade-off against png-decoder (bschwind): png-decoder is also no_std but less documented and narrower. minipng wins on zero-alloc and community vetting.

Trade-off against zune-png: zune-png is faster and handles more formats but requires alloc and std; suitable only in a compositor cell with full heap access, not micro-cells.

Source: github.com/pommicket/minipng · docs.rs/minipng · blog.image-rs.org PNG adoption

image crate: NOT suitable for ViCell cells
Verdict: The image crate has a no_std fork but the mainline requires std; it is a multi-format kitchen-sink. YAGNI — minipng covers the actual use case.

Source: crates.io/image

CATEGORY D: GPU / ACCELERATED RENDERING
wgpu: not viable for ViCell G1; revisit for G2 with caveats
Verdict: std is enabled by default and load-bearing in wgpu's dependency tree; bare-metal is not a supported target; GPU backend (Vulkan/Metal/D3D12/GLES) must be present.

parking_lot can be disabled for no_std but this only removes one locking dependency — the rest of wgpu (surface creation, adapter enumeration, backend HALs) are deeply std-coupled
Requires raw-window-handle surface, a real GPU driver stack, and OS memory management primitives wgpu does not provide
Zero production evidence of wgpu running in a custom OS kernel context; Redox OS runs wgpu in userspace with a proper GPU driver, not bare-metal
G2 path: if ViCell G2 ships a VirtIO-GPU driver cell that exposes a wgpu-compatible surface (via raw-window-handle implementing HasWindowHandle), wgpu becomes viable — but that requires the VirtIO GPU track to complete first (see project memory on G2 platform roadmap)
Adoption risk: HIGH for G1; MEDIUM for G2 given wgpu's broad ecosystem (Bevy, Xilem/Vello, WASM target)
Source: docs.rs/wgpu · github.com/gfx-rs/wgpu

vello: ruled out for same reasons as wgpu, but architecture is worth studying
Verdict: GPU compute-centric 2D renderer built on wgpu; alpha quality; requires std + GPU. Not usable today. Its scene-graph model (encode once, replay on GPU) is the right mental model for ViCell's compositor when GPU arrives.

Linebender's design: Scene (retained draw list) → GPU compute dispatch — eliminates per-frame CPU rasterization cost at scale
Once ViCell has a VirtIO-GPU driver, vello is the natural upgrade path from tiny-skia for the compositor
Alpha stability: API breaks across minor versions
Source: github.com/linebender/vello

glam: adopt now as the math utility layer
Verdict: no_std + libm mode available; MIT/Apache-2.0; SIMD on x86_64 and AArch64; actively maintained (v0.34.2, March 2026). Provides Vec2/Vec3/Mat4 types ViUI currently re-implements ad hoc.

default-features = false --features libm gives full no_std compatibility
12M downloads, used by Bevy — lowest abandonment risk of any crate in this survey
ViUI's layout engine likely has manual f32 geometry — replacing with glam::Vec2 reduces bugs and adds SIMD for free on ARM64 boards (G1 robot target)
Source: github.com/bitshifter/glam-rs · crates.io/glam

CATEGORY E: WINDOWING / DISPLAY ABSTRACTIONS
raw-window-handle: implement the trait, do not use the windowing side
Verdict: no_std-compatible; MIT/Apache-2.0; the correct interoperability shim — ViCell should implement HasWindowHandle on ViSurface to make any renderer that accepts RWH (femtovg, wgpu, softbuffer) work with zero extra glue.

v0.6 introduces safe handle types (WindowHandle<'_>) with lifetime-bounded borrows — eliminates the soundness holes of v0.5's raw pointer passing
RawDisplayHandle is an enum with a Other variant that can carry a custom platform handle — ViCell can define ViCellWindowHandle pointing to its grant-mapped framebuffer pixels
This is a trait implementation task, not a dependency adoption — ~50 lines of code, enables the entire Rust renderer ecosystem to plug into ViCell surfaces
softbuffer (the original query item) is not useful for ViCell: it wraps platform window systems (Win32, X11, Wayland) to produce a CPU framebuffer — ViCell already owns the physical framebuffer directly, making softbuffer's entire value proposition redundant
Source: github.com/rust-windowing/raw-window-handle · docs.rs/raw-window-handle

CATEGORY F: DISPLAY SERVERS / PROTOCOLS
Smithay: reference architecture only — do not adopt
Verdict: Smithay is a framework for building Wayland compositors on Linux (depends on libwayland, DRM/KMS via drm-rs, udev). It is fundamentally tied to the Linux kernel ABI.

Requires std, libc, and a working DRM/KMS kernel driver stack — none of which ViCell provides
drm-rs has a long-standing no_std issue (#25, open since 2018) with no progress
Redox OS's Wayland port uses Smithay running over a compatibility layer — it took months and is described as "not adequate" in performance (June 2025 Redox status)
What IS useful: Smithay's damage-tracking model (dirty region coalescing), surface role semantics, and compositor state machine design are directly applicable to ViCell's compositor spec (§1 of 06-graphics.md) — study the source, do not link the crate
Source: smithay.github.io · phoronix.com/news/Wayland-On-Redox-OS · redox-os.org/news/this-month-250630

CATEGORY G: WIDGET TOOLKITS (EXTERNAL)
Xilem/Masonry: ruled out for ViCell — correct architecture to mirror in ViUI
Verdict: Xilem and Masonry are std-required, depend on winit + wgpu + Vello — the entire rejected dependency chain. However, Masonry's retained widget tree + accessibility model is the closest published art to what ViUI G2 should be.

Masonry: Platform-independent widget tree manager with test harness, focus management, accessibility (AccessKit) — all concepts ViUI will need for G2 desktop mode
Xilem's reactive layer (view tree diff → widget tree patch) is architecturally analogous to ViUI's Signal tree — convergent design
License: MIT/Apache-2.0; Apache Software Foundation stewardship (2024) — reference freely without concern
Recommendation: read Masonry's widget lifecycle docs and layout protocol as a design reference when implementing ViUI's G2 desktop mode (ListView virtual scroll, FlexBox); do not fork
Source: github.com/linebender/xilem · docs.rs/masonry

LVGL Rust bindings: ruled out — unsound and stalled
Verdict: lv_binding_rust is stuck on LVGL v8 with known dangling-pointer UB (issue #166), SIGSEGV on SDL init (#180), and no active maintenance. The LVGL project itself opened an issue acknowledging the stall (lvgl/lvgl#7298).

Alternative wrappers (oxivgl, rlvgl) address safety but are small-community experiments with no shipping track record
LVGL itself is C (MIT), runs on 16 KB RAM, is the dominant embedded UI in the industry — but ViCell would need to write safe Rust FFI bindings from scratch, manage LVGL's tick callback and task scheduler within the SAS model, and handle its display flush callback pattern
The FFI integration complexity is higher than maintaining ViUI, with no #![forbid(unsafe_code)] in cells (Law 4 violation)
Verdict: reject for ViCell. The existing ViUI is already further along than any Rust LVGL binding.
Source: github.com/lvgl/lv_binding_rust · lvgl/lvgl#7298

microui: reference design for ViUI's immediate-mode facade, not an adoption target
Verdict: Pure Rust, zero-allocation immediate-mode GUI; no_std compatible; MIT. Correct architecture for the "egui-compatible" facade ViUI spec commits to (06-graphics.md §4).

The microui crate's design (custom non-alloc containers, single-pass render command list) is the right blueprint for ViUI's immediate-mode layer
It is ~1 KLOC — small enough to fork and adapt rather than depend on; API is minimal
microui-redux extends this but adds std types — reject for cells
Source: crates.io/microui · docs.rs/microui

Ranked Recommendation
Priority order for actual adoption work:

Rank	Action	Crate	Effort	Impact
1	Adopt	minipng	Drop-in, 1 day	Unlocks PNG asset loading in all cells
2	Adopt	glam (no_std mode)	1–2 days refactor	SIMD geometry in ViUI layout, fewer manual bugs
3	Implement trait	raw-window-handle v0.6 on ViSurface	2 days	Opens entire renderer ecosystem to ViCell surfaces
4	Adopt in compositor cell only	tiny-skia (no_std mode)	1 week	Anti-aliased paths, gradients, clipping — real compositor quality
5	Study, do not adopt	embedded-graphics DrawTarget	Read source	Validate ViUI's DrawTarget contract against proven API
6	Defer to G2	wgpu / vello	Months	Requires VirtIO-GPU driver cell first
7	Reject	lv_binding_rust, softbuffer, smithay, Xilem	—	Wrong environment, unsound, or std-only
The single most actionable finding: implementing HasWindowHandle on ViSurface (raw-window-handle v0.6, ~50 lines) is the highest-leverage unblocking move — it makes tiny-skia, femtovg (if a GL backend appears), and eventually wgpu all pluggable into the compositor without per-crate glue code, at zero runtime cost.

Limitations
femtovg no_std status not confirmed: its README and docs.rs page contain no no_std mention; it was not independently verified to compile without std. Treat as std-required until confirmed otherwise.
tiny-skia alloc depth not measured: the no-std-float path compiles under no_std but the exact heap allocation volume at runtime for complex path operations was not benchmarked.
GPU path not researched in depth: virtio-gpu WGPU integration on RISC-V QEMU (the actual G2 test target) has no published case studies; the G2 GPU recommendation is based on wgpu's stated architecture, not a tested configuration.
cosmic-text / harfbuzz shaping not evaluated: full Unicode shaping (Arabic, Indic) was excluded as a G1 non-requirement. If the G2 desktop mode targets RTL locales, this category needs a dedicated follow-up.
Sources:

softbuffer - crates.io
femtovg - crates.io
fontdue - GitHub
lvgl/lv_binding_rust - GitHub
lv_binding_rust stall issue
ab_glyph - GitHub
minipng - GitHub
png-decoder - GitHub
wgpu - docs.rs
raw-window-handle - GitHub
smithay - GitHub
Redox Wayland port - Phoronix
Xilem - GitHub
masonry - docs.rs
tiny-skia - GitHub
embedded-graphics - GitHub
glam - GitHub
vello - GitHub
microui - crates.io
image-png blog - Chromium adoption
Redox 2025 priorities
Agent 3 (graphics stack) xong — kết quả rất chi tiết. Đang chờ 3 agent còn lại (language runtimes, filesystems/crypto, killer apps)...

Now I have everything needed for a thorough, honest ranking. The spec explicitly marks WASM Tier 2 as a wrong path for now, and the existing Lua is vendored PUC-Rio C via cc. This changes the ranking significantly.

Runtime Candidates for ViCell Managed Cells — Research Findings
Framing constraint from docs/specs/05-application.md §6: WASM Tier 2 is explicitly classified as a "Wrong Path" until G2 multi-tenant surfaces. Lua (C vendor via cc) is already live. Any new runtime must justify adding a second managed tier.

Piccolo (pure-Rust Lua 5.4 VM)
Verdict: Not no_std today; anyhow + thiserror v1 pull in std, but with thiserror v2 (Nov 2024) and error_in_core stable since Rust 1.81 (Sep 2024), a port is now mechanically possible — not done upstream.

Deps: anyhow 1.0, thiserror 1.0, gc-arena 0.5, hashbrown 0.14, rand 0.8 — none of these carry libc, but anyhow requires std unless you use no_std anyhow feature; thiserror v1 is std-only; thiserror v2 added no_std behind default-features = false
gc-arena uses allocator-api2, which is alloc-only-compatible — the GC itself could work in no_std + alloc
Stars: 2.1k; last release: v0.3.3 June 2024; single maintainer (kyren); 4-year pause history — abandonment risk is real
Stackless design is architecturally superior for an OS: coroutines/preemption work without touching the Rust stack; DoS-resistant
No stdlib modules (io, os, package) — you write Cell-side bindings for VFS/Net IPC; this is the desired isolation story
Porting cost to no_std: fork + swap thiserror v1 → v2 + audit rand (needs a SmallRng seed from sys_get_random) — estimated 2–3 days; not a community-supported path
Sources: piccolo GitHub · piccolo Cargo.toml · thiserror v2 no_std · Rust 1.81 error_in_core

Vendored PUC-Rio Lua 5.4 (existing — cells/runtimes/lua)
Verdict: Already live in ViCell via Tier 1b C-library pattern; this is the baseline all alternatives must beat.

cells/runtimes/lua/src/c/ = vendored PUC-Rio Lua 5.4 C source; vi_shim.c / vi_stdio_shim.h = ViCell POSIX glue
Link path: cc crate compiles C → static .a → linked into Lua cell; malloc/free route through posix.rs → ostd allocator
Isolation story: #![forbid(unsafe_code)] on the Rust wrapper; C code runs in same SAS — only compiler boundary, not memory boundary
Restricts stdlib (no os, io.popen, debug per docs/ project memory); VFS bindings must migrate OP_* → typed IPC (known gap)
Memory budget: full Lua 5.4 C sources (~25K LOC C) compile to ~180–220 KB stripped binary on RISC-V/ARM64
Source: d:\ViCell\cells\runtimes\lua\ · d:\ViCell\docs\specs\05-application.md

wasmi v1.1 (no_std WASM interpreter, Rust)
Verdict: The only candidate with a documented, tested, production-used no_std path; but the spec explicitly downgrades WASM Tier 2 to "wrong path" — use only under the wasm-experimental feature flag.

default-features = false gives no_std + alloc; uses spin for synchronization primitives in no_std; btree vs hashbrown selectable for environments without random seed
Fuel metering built-in: set a fuel quota per Store, execution yields when exhausted, and "refueled resumable calls" let the scheduler resume paused Wasm jobs — this is the OS scheduling hook
100% Wasm spec compliance; 2× professional security audits (2023, 2024); Google OSS-Fuzz enrolled
Stars: 2.1k; last release: v1.1.0 June 12, 2026; active single-maintainer (Robin Freyler, ex-Parity); previously had Parity/Substrate production backing
Binary size penalty: 2025 embedded paper (treVM, IEEE DCOSS-IoT 2026) confirms wasmi has the largest code size among WASM interpreters tested on Cortex-M / RISC-V / Xtensa — WAMR is 5–8× smaller on flash
Isolation: Wasm module cannot touch host memory by default; host function imports are the only escape hatch — this maps cleanly to ViCell syscall bridge
Cell wrapper pattern: wasmi::Engine + wasmi::Store<CellCtx> per Cell; CellCtx holds tid, fuel quota, VFS/Net capability tokens; host functions call sys_send IPC
Sources: wasmi GitHub · wasmi v1.0 blog · usage docs · treVM paper

Boa (JavaScript, Rust)
Verdict: No no_std support; requires std; 94% ES2022 conformance is impressive but the memory overhead makes it unsuitable for Cell deployment today.

Depends on icu_* crates (Unicode), chrono, bitflags, num-* — all std-linked; no no_std feature flag exists; issue tracker has no open no_std PR
7.3k stars; MIT + Unlicense; active team (~8 contributors)
NaN-boxing reduces JsValue size but does not eliminate std dependency
Memory: full ES engine — realistically 5–10 MB heap minimum for non-trivial programs; incompatible with ViCell's 16 MiB cell quota budget
No sandboxing API; no fuel/gas metering built-in
Sources: Boa GitHub · docs.rs/boa_engine · boa v0.21 blog

rquickjs / QuickJS (JavaScript, C binding)
Verdict: QuickJS C core requires libc (printf for float, fenv, malloc); the rust-alloc feature replaces the allocator but the fenv/printf float dependency remains — not truly no_std without patching the C source.

QuickJS itself: ~210 KB x86 .text; ES2020; Fabrice Bellard's bellard/quickjs is feature-frozen (intentionally minimal); active community forks diverge
rquickjs: high-level Rust bindings; rust-alloc feature uses Rust global allocator; still links to quickjs.c which calls printf/strtod → libc
Stars: ~2.5k; MIT; maintained but not audited for security
Porting to ViCell: requires patching float I/O out of quickjs.c and providing fenv stubs — similar complexity to the existing vi_shim.c approach but harder (float formatting is deeper)
ES2020 (not just 5.1), closures, generators, async/await: richer language than Lua at 2× the complexity cost
Sources: rquickjs GitHub · QuickJS which libc issue · rquickjs-sys docs

JerryScript (JavaScript, C, ES5.1 + partial ES2020)
Verdict: Purpose-built for MCUs with <128 KB RAM; C99 with a mature RTOS embedding story; but it is a C library requiring the same Tier 1b shim treatment as the existing Lua, offers no advantage over Lua in ViCell's model.

258 KB ARM Thumb-2 binary; ships without libc heap by default (custom pool allocator); C99 only
v3.0.0 December 2024; ~84% Test262 conformance; Samsung Tizen RT / ESP-IDF production history
No Rust binding of any note; would require cc + custom shim identical to vi_shim.c
ES5.1 semantics (no async/await, no generators) — weaker developer ergonomics than QuickJS/Boa
Isolation: no built-in sandboxing API; host controls what global functions are exposed
Sources: JerryScript GitHub · JerryScript RIOT-OS

RustPython (Python 3)
Verdict: Hard no — 22k stars but completely std-dependent, requires OpenSSL, not production-ready by its own admission, and Python semantics are fundamentally incompatible with no_std.

libc = "0.2" in build deps; openssl optional but default; rustpython-stdlib build script uses cargo::rustc-check-cfg (std-only cargo facilities)
No no_std issue open; no embedded roadmap; WASM target uses wasm32-unknown-emscripten (which provides a libc shim)
Python memory model (reference counting + cycle GC) requires allocator support far beyond alloc crate; no path to fit in 16 MiB cell quota
Status: explicitly "not totally production-ready"
Sources: RustPython GitHub · issue #5388

BLisp (statically typed Lisp, Rust)
Verdict: Explicitly no_std; but 235 stars, unclear maintenance, a non-standard language with no ecosystem — adoption risk is near-total.

MIT license; no_std + alloc; Hindley–Milner type inference; algebraic data types; effect system
~7K SLoC; fits in a cell trivially
Last commit date unknown from public sources; 235 stars; 11 forks — niche academic project
No REPL tooling, no package manager, no standard library to speak of
The effect system is architecturally interesting for OS scripting but requires users to learn a completely novel language
Sources: BLisp GitHub · lib.rs/blisp

Forth (liorforth, rtforth, and others)
Verdict: Forth is the right fit for embedded control scripting but no production-quality, no_std, Rust-native Forth interpreter exists; all candidates are either GPL-encumbered, <10 stars, or "work in progress."

liorforth: 3 stars; GPL-3.0; no_std listed as future goal, not implemented; missing fm/mod, um*, um/mod
rtforth: designed for real-time applications; targets Linux with std; not no_std
A Forth implementation from scratch in ViCell-idiomatic Rust is ~800–2000 LOC and tractable, but that is implementation work, not a library adoption decision
Sources: liorforth GitHub · rtforth GitHub

wasm3 (C WASM interpreter)
Verdict: Smallest WASM binary footprint (vs wasmi), 7.9k stars, MIT — but entered minimal-maintenance mode; requires C build path identical to existing Lua; and WASM Tier 2 is already declared a wrong path in the spec.

~64 KB flash, ~10 KB RAM; 4–15× slower than native; runs on ESP8266/ESP32/Arduino
v0.5.0 last tagged June 2021; PRs merged on-request but no new features
No Rust-native bindings of quality; would need cc + glue identical to vi_shim.c
For the WASM-experimental path: wasmi is strictly preferred (audited, Rust-native, no_std verified)
Sources: wasm3 GitHub · treVM paper

Trade-Off Matrix
Runtime	no_std+alloc	License	Stars	Abandonment Risk	Isolation Story	ViCell Fit
Lua 5.4 (C vendor)	N/A (Tier 1b)	MIT	N/A — upstream	Low (PUC-Rio stable)	LBI only	Already live
wasmi	Yes (verified)	MIT/Apache	2.1k	Low-medium	Wasm SFI + fuel metering	WASM-experimental only
piccolo	Fork needed	MIT/CC0	2.1k	Medium-high	Pure Rust, stackless	Second Lua path if forked
rquickjs	No (C libc dep)	MIT	2.5k	Low	None built-in	Tier 1b only, harder shim
Boa	No	MIT	7.3k	Low	None	Not viable
JerryScript	Tier 1b via cc	Apache 2	~8k	Low	None	Duplicate of Lua pattern
RustPython	No	MIT	22k	Low	None	Hard no
BLisp	Yes	MIT	235	High	Type system only	Niche/academic
Forth (custom)	Yes (DIY)	N/A	N/A	N/A (owned)	Minimal	RT scripting only
wasm3	Tier 1b via cc	MIT	7.9k	High	None beyond Tier 1b	Superseded by wasmi
Ranked Recommendation
Rank 1 — Keep the existing vendored PUC-Rio Lua 5.4 as the sole managed runtime for G1.

Rationale: It is already working, the isolation boundary is explicit (restricted stdlib, no os/io.popen), and the shim pattern is understood by the team. The documented gaps (typed IPC migration for VFS bindings) are engineering work, not architectural blockers. No other candidate improves meaningfully on this for G1's use case (trusted scripting on robot/embedded). Adding a second managed tier before G2 violates YAGNI.

Rank 2 — wasmi under feature = "wasm-experimental" for G2 multi-tenant.

If ViCell G2 becomes a multi-tenant platform (third-party workloads from the internet), wasmi is the only candidate with verified no_std, fuel metering for OS-level scheduling integration, a Wasm SFI isolation boundary stronger than LBI-only, two security audits, and a Rust-native API. The cell wrapper is clean: wasmi::Store<CellCtx> holds the Cell's capability tokens; host functions bridge to sys_send IPC. The binary-size penalty on flash is real but irrelevant for G2 server/PC targets. Activate per the existing phase = "wasm-experimental" note already in the spec.

Rank 3 — piccolo if a pure-Rust Lua runtime is ever needed.

The stackless design is architecturally superior for preemption in an OS. The no_std blocker is now mechanically solvable (thiserror v2 + Rust 1.81) — a focused fork could land it in 2–3 days. Track upstream for a maintainer-led no_std PR before investing. Do not adopt piccolo to replace the existing Lua runtime unless the C-vendor approach causes a concrete problem (e.g., a CVE in PUC-Rio Lua, or compiler safety audit requirements).

All JS options (Boa, rquickjs, JerryScript), RustPython, BLisp, Forth libraries: Do not adopt. They either cannot reach no_std, duplicate the existing Lua Tier 1b pattern with worse ergonomics, or have unacceptable abandonment risk.

Limitations of This Research
wasmi binary size on ViCell targets: The treVM paper benchmarked Cortex-M / RISC-V / Xtensa RTOS, not bare-metal QEMU virt with ViCell's 16 MiB quota. For G2 server/PC the flash-size concern evaporates.
piccolo no_std feasibility: Requires actually building it against #![no_std] with thiserror v2 to confirm — no upstream has done this; the analysis is from dependency graph reasoning, not a working build.
QuickJS float libc dependency: Bellard's upstream intends to remove it ("expected that this dependency on the libc will be removed at some point"); community forks may have already done so — not verified.
Wasm GC proposal: wasmi's GC support is still in development; if ViCell ever needs managed-language runtimes compiled to Wasm (e.g., C#/Kotlin → Wasm), verify GC proposal support before committing.
Sources:

Piccolo GitHub
Piccolo Cargo.toml source
wasmi GitHub
wasmi v1.0 blog post
wasmi usage docs
treVM: Tiny Rust Embedded VMs with WASM paper
Boa GitHub
rquickjs GitHub
QuickJS libc issue
JerryScript GitHub
RustPython GitHub
BLisp GitHub
wasm3 GitHub
liorforth GitHub
Rust 1.81 error_in_core stabilization
thiserror v2 no_std adoption in reth
Agent 2 (language runtimes) xong — đang chờ 2 agent còn lại (filesystems/crypto và killer apps)...

Now I have a comprehensive picture. Here is the full analysis:

Current ViCell Game/App Portfolio Baseline
Verdict: ViCell already has working DOOM (boots, renders first frame), Tetris (native Rust), and an httpd Cell — the baseline is stronger than most hobbyist OSes at this stage.

DOOM via doomgeneric: working on QEMU RV64, uses picolibc malloc sbrk wrapper + compositor Grant surface. Full 6-hook integration complete (cells/demos/doom/src/main.rs)
Tetris: native Rust no_std, cells/demos/tetris/ — already a clean showcase
httpd: native Rust no_std at port 8080, httparse parser, VFS-backed static files (cells/services/httpd/)
Existing demos: hello, sdk-demo, robot-demo, sensor-demo, periph-demo, adc/can/pwm demos — robotics G1 angle already populated
What is absent: any networking application that a non-OS person finds compelling; any developer tool that generates credibility with the server/cloud audience Source: cells/demos/doom/Cargo.toml, cells/services/httpd/src/main.rs, cells/demos/tetris/Cargo.toml
Angle 1: Classic Game Ports — What Actually Moves Community Needles
Verdict: Quake 1 (not Quake 3) is the correct next game target; Wolf3D is marginal; Duke3D is a dead end.

SerenityOS: Quake ran early (Tyr-Quake port, repo SerenityPorts/SerenityQuake), Half-Life followed later — both generated sustained HN/Reddit waves. The lesson: each new game port is a standalone press event
Quake 1 (TyrQuake/QuakeSpasm): requires malloc/free, fopen/fread/fclose, printf, sin/cos/sqrt — all reachable via Tier 1b posix.rs + picolibc. WAD reads can go through ViCell VFS. No fork, no dynamic linking, no mmap required. C89, ~100K LOC — smaller than DOOM+doomgeneric
Quake 1 requires a software renderer (GLQuake is irrelevant at G1 — no GPU rasterizer). TyrQuake has a software path; WinQuake has the original software path. Both are single-file-per-subsystem C, compilable with -DSOFTWARE flags
Quake 3: requires dynamic library loading (dlopen) for game logic VM + proper filesystem hierarchy + OpenGL. Incompatible with ViCell SAS constraints at G1. Confirmed by Porting Quake3 writeup: needs POSIX threads + dynamic linker
Wolf3D source (1995 id release): has had a 48 KB embedded port (pbrook/ewolf3d) proving malloc-free static-buffer compile is achievable. But Wolf3D is visually dated — community reaction is "cool" not "wow". Marginal ROI versus effort
Duke3D (EDuke32): needs fork() for multiplayer + Linux-specific paths. Build2 engine is a non-trivial C++ codebase. Not worth the porting effort at G1
The Powder Toy: SerenityOS ported this (particle physics simulator). Pure C, software render, no syscalls beyond time + display. 50K stars on GitHub. High HN appeal, low porting cost Source: SerenityPorts/SerenityQuake, SerenityOS 4th year, Porting Quake3, Wolfenstein 3D 48KB port
Angle 2: Networking Applications
Verdict: A working DNS resolver Cell (via hickory-proto no_std) has higher community ROI than porting a web server. ViCell's httpd already covers the HTTP angle.

httpd already exists at cells/services/httpd/ — single-connection HTTP/1.1, VFS-backed, no TLS yet. This is correct for G1 but will not impress the server/networking crowd until it serves concurrent connections or has TLS
nginx/lighttpd: both require fork(), epoll/kqueue, mmap, POSIX threads, and a mature libc (full stdio, regex, signal handling). Tier 1b posix.rs explicitly returns -1 for fork. Not portworthy at G1 without Tier 3b Linux VM (which already exists and can just apt install nginx)
Tier 3b answer: Tier 3b Linux VM already boots Alpine and apt install nginx works per the architecture doc. For the networking showcase this is G2's answer — it is already possible and not research-needed
hickory-dns no_std: PR #2104 (merged Nov 2024, requires Rust 1.81+) makes hickory-proto crate available in no_std. This means a ViCell DNS resolver Cell is feasible today. A DNS Cell would be the first no_std DNS implementation inside a custom OS — a genuine first that generates technical blog traffic
DNS resolver Cell vs DNS server: resolver (client) is far simpler — hickory-proto provides it with no_std. A full authoritative DNS server still requires std in hickory-server. Target: recursive resolver Cell (caches results, forwards to upstream) — achievable in ~500 LOC Rust
MQTT broker (rumqttd): rumqttc explicitly states no_std is not supported. rumqttd is std-only (tokio-based). Mosquitto port would need fork+threads. Not G1-portworthy via Tier 1 or 1b. Only option is Tier 3b Linux VM, which trivially works. Not a showcase item
mcutie: no_std MQTT client (October 2025). Client only, but a working MQTT client Cell connecting to a remote broker over ViCell's net stack would be meaningful for robotics demos Source: hickory-dns PR #2104, rumqtt no_std limitation, [httpd main.rs file:20]
Angle 3: Developer Tools as Cells
Verdict: SQLite is the single highest-ROI developer tool port. redb and sled both require std and cannot be ported to Tier 1 without significant surgery.

SQLite: The entire database is a single-amalgamation C file (3.47 MB, ~150K LOC). Requires: malloc, fopen/fwrite/fread, pthread mutex (can be compiled away with SQLITE_THREADSAFE=0), time. ViCell already has posix.rs + picolibc for malloc and a VFS with file read/write. The only missing piece is a custom SQLite VFS layer (sqlite3_vfs) that redirects all file I/O through ViCell's sys_send(VFS_ENDPOINT, OP_READ/WRITE). This is 200–400 LOC of C. Successful precedents: FreeRTOS (NXP community), Raspberry Pi Pico (pico-vfs project May 2024), STM32 H7 (community threads). Tier 1b mlibc path is the right tier (Tier A posix.rs would need fseek/ftell stubs similar to DOOM's fixes)
SQLite community signal: SQLite runs on "any OS that can provide these functions." A ViCell SQLite Cell is the proof point that ViCell's VFS API is complete enough to be a real OS
redb: requires std::io::Write, std::io::Read, std::fs, memory-mapped files (mmap). Hard std dependency. Not portworthy to Tier 1. Could run inside Tier 3b Linux VM trivially
sled: uses tokio, std::sync, crossbeam. std-only by design. Same verdict as redb — Tier 3b only
mini-redis (tokio-rs): explicitly a tokio tutorial app, std-only. The pattern it teaches (in-memory key-value store) is trivially implemented from scratch in native Rust no_std Cell in ~300 LOC. Worth building as a native Cell from scratch rather than porting — call it vicell-kv
A native Rust key-value store Cell (ViKV) would be more impactful than porting mini-redis: demonstrates ViCell IPC as a data service interface, useful for config/cache in robot demo Source: SQLite VFS docs, SQLite custom builds, pico-vfs May 2024, redb, mini-redis
Angle 4: Robotics / Embedded Specific
Verdict: micro-ROS is too heavy; a native Zenoh-pico client Cell is the right robotics middleware play; OpenCV is G2-only.

micro-ROS: requires POSIX compliance (threading, semaphores, real-time clocks), XRCE-DDS middleware (C, ~50K LOC), and a transport layer. Supported RTOSes are FreeRTOS, Zephyr, NuttX — all of which required non-trivial integration effort (they have POSIX plugins). ViCell SAS has no fork, no pthreads, no signals. Port would require writing a ViCell transport layer + POSIX shim for XRCE-DDS. Feasible at Tier 1b (similar to mlibc path) but 3–6 months of work. Upside: ROS2 compatibility is a huge G1 robotics credibility marker
Zenoh-pico (ROS2 alternative): picozenoh / zenoh-pico is the MCU-targeted RMW. C library, ~30K LOC, far smaller than full DDS stack. Uses UDP multicast for discovery. Requires: sockets (ViCell net Cell has TCP/UDP), malloc, basic time. ROS2 is phasing DDS out in favor of Zenoh (release scheduled May 2024). A ViCell zenoh-pico Cell connecting to a ROS2 graph over the network is achievable in 2–4 weeks, and is more forward-looking than micro-XRCE-DDS
OpenCV: requires full C++ STL, std::filesystem, threading, simd intrinsics, and ~250MB of build dependencies. G1 is impossible. G2 feasible only via Tier 3b Linux VM. Even then, the GPU pipeline (Wayland + GL) is not present in G2 yet. Not a G1 item
MQTT client (mcutie, no_std, Oct 2025): this is the right level for G1 robotics telemetry. A ViCell MQTT client Cell sending sensor readings (from SHT3x I2C sensor already working) to a remote broker is a demo-ready robotics showcase requiring ~1 week of work
Sensor fusion patterns: already present in cells/demos/sensor-demo — the I2C/SPI peripheral track is done. The gap is shipping the data somewhere (MQTT, Zenoh, HTTP POST to httpd) Source: micro-ROS supported RTOSes, einfochips micro-ROS porting guide, ROS2 Zenoh RMW, mcutie
Angle 5: What Hobbyist OSes Port First (Credibility Ladder)
Verdict: The pattern across SerenityOS, Redox, and Haiku is a 4-stage credibility ladder. ViCell is at stage 1-2 and should target stage 3 next.

Stage 1 — "It runs code" (hello world, basic shell): done
Stage 2 — "It runs something impressive visually" (DOOM/Tetris): done for ViCell. SerenityOS used Quake for this step; Redox used emulators
Stage 3 — "It can be a development tool or server" (text editor, web server, database): Redox ported Nano + Helix editors and Apache httpd. This is where ViCell needs to go. The credibility jump from "runs DOOM" to "runs SQLite and serves HTTP with a database behind it" is substantial
Stage 4 — "It can replace Linux for a workload" (desktop GUI, package manager, full compiler): Haiku, SerenityOS. G2 territory for ViCell
Redox credibility pattern (2024): COSMIC Desktop port + RustPython by default in desktop builds + nano/helix. Their porting strategy explicitly relies on SDL + Orbital for games and winit/softbuffer for Rust GUI apps. This is structurally similar to ViCell's compositor/ViUI path
SerenityOS Half-Life impact: the Half-Life port was a "personal highlight" milestone that drove media coverage in Sweden and international HN front-page. The pattern: each game with 3D graphics that runs proves a new OS subsystem (3D renderer, sound, input) is complete
The Powder Toy (SerenityOS port): pure C physics simulation with software framebuffer. GitHub star count ~50K. Ported easily. This class of app (visually impressive, pure software render, minimal syscalls) is the sweet spot for stage 2→3 transition Source: SerenityOS 4th year, SerenityOS Half-Life port, Redox March 2024, Redox porting strategy, Haiku Beta 5 2024
Trade-Off Matrix
App	Tier	Porting effort	Infra required	Community impact	G1/G2	Blocker
Quake 1 (TyrQuake, software render)	1b (posix.rs)	3–4 weeks	picolibc + compositor	Very High	G1	sound (no audio Cell yet)
The Powder Toy	1b (posix.rs)	1–2 weeks	compositor framebuffer	High	G1	none
SQLite Cell	1b (mlibc)	2–3 weeks	VFS write + sqlite3_vfs C shim	Very High	G1	OP_WRITE mkdir stubs
ViKV (native Redis-compatible KV)	1 (native Rust)	1 week	Net Cell TCP	Medium	G1	none
hickory-proto DNS resolver Cell	1 (native Rust)	1–2 weeks	Net Cell UDP	Medium-High	G1	PR #2104 no_std merged
mcutie MQTT client Cell	1b or 1	1 week	Net Cell TCP + sensor demo	High (robotics)	G1	mcutie API stability
Zenoh-pico Cell (ROS2 compat)	1b (mlibc)	3–4 weeks	Net Cell UDP + multicast	High (robotics)	G1	multicast bc/mc gap in Net Cell
nginx / lighttpd	3b (Linux VM)	0 (apt install)	Tier 3b ARM64 (done)	Low (trivial)	G2	already works
OpenCV	3b (Linux VM)	0 (apt install)	Tier 3b + GPU	Low (trivial)	G2	GPU not in G2 yet
micro-ROS full stack	1b (mlibc)	3–6 months	pthreads shim + XRCE-DDS	Very High	G1.5	pthreads
Wolf3D	1b (posix.rs)	1 week	compositor	Low	G1	low novelty
Ranked Recommendation
Rank 1 — SQLite Cell (Tier 1b, mlibc path): This is the single highest ROI port. It proves ViCell's VFS is real-OS grade, unlocks robot data logging, robot configuration storage, and web API backends (httpd + SQLite = full stack demo). The porting pattern is well-documented across FreeRTOS, Pico, STM32. Work: (a) add fseek/ftell/mkdir to posix.rs or use mlibc, (b) implement a 200–400 LOC sqlite3_vfs C file redirecting to ViCell VFS IPC, (c) wrap in a Tier 1b Cell. No new kernel syscalls needed. Blocker: VFS OP_WRITE and directory creation must be working (check current status before starting).

Rank 2 — Quake 1 (TyrQuake software renderer, Tier 1b): The step-function community event after DOOM. Requires the same picolibc + compositor path as DOOM. Critical gap: Quake needs audio (_write to /dev/dsp or similar) — ViCell has no audio Cell. Options: (a) port with sound disabled (WinQuake -nosound flag) as a stage-2 showcase, (b) implement a minimal audio Cell (PL011-based PCM output on ARM64 QEMU) in parallel. Recommendation: port now with -nosound, schedule audio Cell separately. WAD: shareware pak0.pak (free, redistributable as ID_SHAREWARE).

Rank 3 — mcutie MQTT client Cell (Tier 1 native): Lowest-effort, highest robotics credibility. A Cell that reads SHT3x I2C sensor data (already working in sensor-demo) and publishes via MQTT to a broker is the canonical IoT demo. This is a complete robot telemetry pipeline: hardware sensor → Cell → MQTT → dashboard. Expected effort: 1 week. Dependency: mcutie API should be pinned (it is Oct 2025, potentially unstable).

Rank 4 — hickory-proto DNS resolver Cell (Tier 1 native Rust): A pure-Rust no_std DNS recursive resolver Cell is a genuine first in the custom-OS space. hickory PR #2104 merged Nov 2024 with no_std support for hickory-proto. Requires Rust 1.81+. Uses ViCell Net Cell for UDP transport. Generates blog-worthy "first no_std DNS resolver in a custom OS" narrative. Not robotics-critical but critical for G2 server credibility.

Rank 5 — The Powder Toy (Tier 1b): Pure C particle physics, software framebuffer, no audio, no networking. Porting effort 1–2 weeks. The visual "wow" factor on par with DOOM for non-gaming audiences (scientists, educators, HN crowd). Gap: requires a reasonably complete stdio (fopen for config save/load) and math.h. Both available in posix.rs + picolibc.

Not recommended for G1:

Zenoh-pico: blocked on multicast/broadcast gap in ViCell's Net Cell (project memory: mc/bc left as deferred). Unblock multicast first, then Zenoh-pico becomes straightforward.
micro-ROS: pthreads requirement makes this 3–6 months of infrastructure work. YAGNI for G1 given Zenoh-pico is a better-fit alternative.
nginx/lighttpd as native port: fork + epoll incompatible with SAS. Tier 3b Linux VM already handles this trivially — no incremental work needed.
Wolf3D, Duke3D: diminishing returns. Wolf3D is less visually impressive than DOOM; Duke3D needs fork. Neither advances ViCell's technical narrative.
Infrastructure Gaps (What Must Ship Before Rank 1–3 Can Land)
VFS OP_WRITE directory creation (mkdir): SQLite creates ./saves/ directories. Currently mkdir() is stubbed to return 0 in the DOOM cell. Real mkdir via VFS IPC needed for SQLite to be trustworthy.
Audio Cell (optional for Quake): No audio service exists. Quake -nosound works around this for now but is a visible omission. A minimal PCM output Cell (ARM64 QEMU PL041/AC97 or PL011 abused for buzzer audio on real hardware) is a parallel workstream.
Net Cell multicast/broadcast: Needed for Zenoh-pico (ROS2 discovery). Already noted in project memory as deferred. Not a blocker for MQTT client (MQTT uses unicast TCP).
TLS in httpd: For the "full-stack demo" (httpd + SQLite + HTTPS), TLS is needed. Project memory shows a TLS plan (embedded-tls 0.19, GetRandom syscall 214). This is a G1.5 item but must land before ViCell can claim to serve secure web traffic.
Limitations of This Research
Audio Cell requirements were not researched in depth. The audio subsystem (what PL041/AC97 QEMU emulates vs. what real ARM64 boards have) needs a separate research pass before scheduling the Quake audio track.
The Powder Toy's exact libc surface (specifically whether it uses dlopen for plugins or any thread primitives) was not verified against the actual source. A 30-minute code audit of powder-toy/src/ is needed before committing to the port estimate.
mcutie API stability was not verified. The crate is from October 2025 and may not have reached 1.0. Pin verification and changelog review required before starting.
hickory-proto no_std completeness was sourced from PR metadata only — the actual feature set of the merged no_std path (does it include the resolver logic or only the wire protocol?) was not verified by reading the merged code.
Sources:

doomgeneric GitHub
OSDev.org libC requirements for DOOM
SQLite VFS documentation
SQLite custom builds
hickory-dns no_std PR #2104
SerenityOS 4th year recap
SerenityOS Half-Life port
SerenityPorts/SerenityQuake
Redox OS March 2024 update
Redox OS porting strategy
Haiku Beta 5 2024
micro-ROS supported RTOSes
einfochips micro-ROS porting guide
ROS2 alternative middleware report
rumqtt no_std limitation
Wolfenstein 3D 48KB embedded port
pico-vfs SQLite on Raspberry Pi Pico (May 2024)
SQLite FreeRTOS porting forum
4/5 xong rồi — đang chờ agent cuối (filesystems + crypto)...

Now I have a complete picture. Let me compile the full analysis.

Research: no_std Filesystem, Storage, and Cryptography Libraries for ViCell
Project context read: ViCell already vendors littlefs2 = "0.7.2", fatfs (git, alloc), redoxfs (third_party, default-features=false), embedded-tls = "0.19", rand_chacha, rand_core, smoltcp. The silo cell carries zero crypto deps — it relies on hardware SiloHandle, not software crypto.

Filesystems
littlefs2 v0.7.2 — already in tree, correct choice for /data
Verdict: Production-ready, no_std, power-loss-safe; ViCell already uses the latest release (v0.7.2, June 2026). No action needed.

Maintained by trussed-dev (the Trussed firmware crypto framework), not abandoned. 96 stars, 40 forks, 14 releases.
no_std with two caveats: resolver = "2" (already in workspace Cargo.toml — covered), and c-stubs feature needed if strcpy is unavailable at link time (bare-metal without libc).
Binds C littlefs v2.9.3 via FFI (littlefs2-sys). Not pure Rust — requires a C toolchain for the target. ViCell already cross-compiles C (DOOM, mlibc), so this is not new friction.
License: Apache-2.0 / MIT (crate) + BSD-3 (upstream littlefs C). All compatible with ViCell MIT.
ViCell gets: wear-leveling, power-loss resilience, small RAM footprint (~500 bytes) for /data on NAND/eMMC. Absolutely correct for G1 robot board storage.
Action: Verify c-stubs feature is active if linking without picolibc/mlibc. No version upgrade needed.
Source: https://github.com/trussed-dev/littlefs2; https://crates.io/crates/littlefs2

fatfs (git, rafalh) — already in tree, /mnt/sd interop
Verdict: Correct for FAT32 SD-card interop; no_std+alloc-only build works on stable Rust. No action needed.

v0.3.6 on crates.io; ViCell pins to git (rafalh/rust-fatfs) with alloc feature — this is the recommended pattern, as the published crate's no_std support historically required nightly.
29,800 downloads/month; used by 32+ crates. Not high-traffic but stable.
MIT license.
ViCell gets: FAT12/16/32 read-write with LFN/VFAT. No journaling. Correct role: /mnt/sd (SD interop) after /data migrates to littlefs.
Gap: exFAT is not supported (OEM-Name detection logged as warning per spec — this is correct behavior).
Source: https://github.com/rafalh/rust-fatfs; project VFS spec confirms current usage.

ext2/ext4 — NOT recommended for ViCell
Verdict: Only read-only no_std option exists; GPL-2 write implementations are license-incompatible. Not viable.

ext4-view (read-only, no_std+alloc): pure Rust, reads ext2/ext3/ext4, MIT. Updated Apr 2026. Would only serve mounting Linux rootfs images for inspection — not a general storage backend.
lwext4_rust: C FFI wrapper around lwext4 (GPL-2), riscv64/aarch64/x86_64 support. GPL-2 is license-incompatible with ViCell MIT unless kept in a strictly isolated binary. Not worth the friction.
ext4fs (write capable, lib.rs): no_std compatible per lib.rs listing; maintained. But ext4 write correctness without a journal is dangerous; the project is low-star, unclear audit status.
Decision: ext4 is a reader tool for linux-vm compatibility, not a native ViCell storage layer. Skip for now (YAGNI). The RedoxFS/G2 path covers the CoW+checksum need.
Source: https://lib.rs/crates/ext4-view; https://github.com/elliott10/lwext4_rust

SQLite / sqlite-rs-embedded — NOT recommended
Verdict: Pre-release, unsafe bindings, no official releases. Not appropriate for OS-level use.

sqlite-rs-embedded (vlcn-io): no_std + WASM compatible. 49 stars, 120 commits, zero releases on crates.io. Explicitly warns: "bindings are not entirely safe — statement object will clear returned values out from under you if you step/finalize while references exist." This violates ViCell's Law 4 (no unsafe in Cells) and Law 2 (owned buffers only).
rusqlite: requires std. Not usable in Cells.
What ViCell needs instead: a key-value store (e.g., ekv — embedded key-value, no_std) for configuration persistence, not a relational DB. SQLite is a G3 concern at most (Tier 3 Linux VM can use it natively).
Source: https://github.com/vlcn-io/sqlite-rs-embedded

TFS (TheFileSystem, redox-os/tfs) — confirmed dead upstream
Verdict: Officially replaced by RedoxFS; README says "no longer maintained." ViCell's 2026-06-10 decision to drop viFS2/TFS was correct.

GitHub repo is a mirror, read-only. README explicitly: "TFS was replaced by RedoxFS and is no longer maintained."
"While many components are complete, TFS itself is not ready for use" — never shipped.
No action needed. ViCell's ADR at docs/specs/09b-vfs-native-fs-adr.md already captures this.
Source: https://github.com/redox-os/tfs

RedoxFS (vendored, third_party/redoxfs v0.9.0) — correct G2 strategy
Verdict: Properly vendored with default-features=false; no_std-capable when std feature is off. Ready when NVMe ships.

Cargo.toml inspection confirms: std feature gates libc/getrandom/fuser; without it, the core library compiles with only aes, argon2, base64ct, lz4_flex, seahash, uuid, xts-mode, bitflags, endian-num — all no_std-capable or alloc-only.
Built-in features: AES-XTS encryption (transparent), LZ4 compression, seahash checksums, CoW B-tree, snapshots. This is the most feature-rich pure-Rust no_std FS available.
RedoxFS already runs on RISC-V hardware in production (Redox OS). Production-proven.
Risk: aes = "0.8" in vendored redoxfs vs any newer RustCrypto aes used elsewhere — check for version conflicts when the crypto stack expands.
ViCell gets: CoW, checksums, native encryption, crash recovery for /srv on NVMe. Zero additional effort needed until NVMe is ready.
Source: d:\ViCell\third_party\redoxfs\Cargo.toml; https://github.com/redox-os/redoxfs

Storage Abstraction
embedded-storage v0.3.1 — medium value, not urgent
Verdict: The de-facto trait standard for NOR/NAND flash in embedded Rust (60+ reverse dependencies), but ViCell's current disk abstraction is already sufficient for G1. Adopt when adding real board flash drivers.

Defines ReadStorage, Storage, Region traits plus NorFlash sub-traits. No_std, Apache-2.0/MIT, MSRV 1.50.
embedded-storage-async v0.4.1 (Dec 2023) adds async variants — relevant because ViCell's VFS is async-by-design.
Implementors include: stm32h7xx-hal, nrf-softdevice, esp-idf-svc, embassy-stm32, spi-memory, embedded-sdmmc — strong ecosystem.
ViCell gap: ViCell's current disk block I/O uses a custom grant-based DMA API (syscalls 208-212). The embedded-storage traits don't map cleanly to async Grant I/O. A thin adapter ViDiskAdapter: NorFlash would bridge them when needed.
When to act: G1 tail, when adding real NAND/eMMC drivers on board hardware. For QEMU VirtIO this doesn't apply.
Source: https://docs.rs/embedded-storage; https://github.com/rust-embedded-community/embedded-storage

embedded-sdmmc — situationally useful
Verdict: Pure Rust no_std no_alloc SD+FAT32 stack for microcontrollers. Useful if ViCell ever drives a raw SPI SD slot (MMC HAL path). Skip until needed.

Implements its own BlockDevice trait (distinct from embedded-storage); reads/writes 512-byte sectors. No heap. Proven on Cortex-M and RISC-V MCUs.
ViCell overlap: The MMC subsystem plan (.agents/260607-1600-mmc-subsystem/) targets VirtIO + SDHCI PIO. embedded-sdmmc fits the raw SPI path only.
Source: https://docs.rs/embedded-sdmmc

Cryptography
RustCrypto primitives — HIGH VALUE, adopt now
Verdict: The only viable no_std crypto suite for ViCell. All major primitives are no_std+alloc with stable APIs and active maintenance. Start with chacha20poly1305 + sha2 + p256.

Individual crate status:

Crate	Version	no_std	License	Notes
aes-gcm	0.10.3	yes, alloc optional	MIT/Apache	HW accel (AES-NI/CLMUL); heapless+arrayvec features for stack-alloc
chacha20poly1305	0.10.1	yes, alloc optional	MIT/Apache	heapless feature for no-heap AEAD; preferred for embedded (no HW requirement)
sha2	0.11.0	yes	MIT/Apache	SHA-256/384/512; pure Rust or HW-accelerated backend
p256	0.13.2	yes (default-features=false)	MIT/Apache	ECDSA+ECDH feature-gated; unaudited — no independent audit
aes	(via redoxfs 0.8)	yes	MIT/Apache	Already in tree via RedoxFS vendor
crypto-bigint	latest	yes	MIT/Apache	Constant-time big integers; used by p256 internally
ed25519-dalek	3.0.0-rc.1 (was 2.1.1 stable)	yes, default-features=false	MIT/Apache	v3 removed std feature; #[no_std] is now the default mode
chacha20poly1305 preferred over aes-gcm for ViCell Cells: no AES hardware needed on RISC-V; simpler implementation; software constant-time is cleaner.
p256 audit caveat: "The elliptic curve arithmetic has never been independently audited." Acceptable for ViCell's Silo (hardware-backed key) but not for standalone software ECDSA in security-critical paths.
RustCrypto trait system (aead, digest, signature, cipher crates) enables swapping implementations without changing call sites. This is architectural leverage.
ViCell gets: The entire crypto stack for TLS (already used by embedded-tls), storage encryption (RedoxFS uses aes crate), and future key exchange. No rolling-your-own.
Source: https://docs.rs/aes-gcm; https://docs.rs/chacha20poly1305; https://docs.rs/p256; https://docs.rs/sha2

ring — REJECTED for ViCell
Verdict: Explicitly fails to compile for thumbv7em-none-eabi and similar bare-metal targets. Uses C/asm that requires a libc. Hard no for no_std OS kernels.

Requires std or at minimum a C runtime. Acknowledged design goal to support MCUs "eventually" — not there as of 2025.
CVE-2025-4432 noted in advisories (see RustSec).
Conclusion: ring is for server-side Rust only. embedded-tls uses RustCrypto crates, not ring — ViCell's path is already correct.
Source: https://users.rust-lang.org/t/rust-crypto-library-for-cortex-m/44362; CVE search results

embedded-tls v0.19 — already in tree, correct
Verdict: The only viable TLS 1.3 client for no_std Rust. ViCell is already on the correct version. Known limitations must be understood.

Cipher suites supported: TLS_AES_128_GCM_SHA256, TLS_AES_256_GCM_SHA384 — covers 95%+ of real servers.
Dependencies: aes-gcm, ecdsa, sha2, p256, embedded-io. All already resolved by embedding in service-net.
Critical limitation: CertVerifier (webpki-based cert chain validation) only works with std feature. In the current no_std deployment, cert verification is disabled. This is an acceptable tradeoff for a private embedded system talking to known servers, but must be documented as a security assumption — blind TLS is susceptible to MITM.
rustls-rustcrypto: exists as an experimental provider but still requires std (confirmed). Not an option for ViCell Cells.
ViCell gets: TLS 1.3 client capability for HTTPS cells (cells/demos/https-demo). Already proven in tree.
Source: https://docs.rs/embedded-tls; https://github.com/drogue-iot/embedded-tls

rustls — NOT for ViCell Cells
Verdict: Requires std; even the rustls-rustcrypto provider doesn't support no_std yet. Correct only for G2 Linux VM (Tier 3) where std exists.

rustls-rustcrypto is described as "experimental" and "requires std." Future no_std expansion is architecturally planned but not delivered.
Source: https://github.com/RustCrypto/rustls-rustcrypto

ed25519-dalek v3.x — HIGH VALUE for Silo + code signing
Verdict: Fully no_std by default in v3.x. Use for Cell signature verification and VFS integrity signing. Do not pin to 2.x.

v3.0.0-rc.1 is current; v2.1.1 is last stable. v3 removes the std feature entirely — no_std is the only mode. alloc is only pulled in by optional features (batch, pem, pkcs8).
Moved to dalek-cryptography/curve25519-dalek monorepo.
ViCell use case: Signing Cell ELF binaries at build time; verifying signatures in the loader before spawning a Cell. Aligns with Law 4 (no unsafe in Cells) since dalek is pure safe Rust.
Audit status: The original curve25519-dalek received an audit (NCC Group, 2019). ed25519-dalek itself has not been independently audited post-2.0.
Source: https://github.com/dalek-cryptography/curve25519-dalek/tree/main/ed25519-dalek

age encryption (str4d/rage) — NOT for ViCell
Verdict: Requires std (async I/O, file handles, WebSys). No no_std path exists or is planned.

age's Rust implementation uses async encryption over streaming I/O and pulls in multiple std-dependent crates. The WASM path uses web-sys. No embedded/no_std story.
What ViCell needs instead: For file encryption at rest, RedoxFS's built-in AES-XTS transparent encryption is the right tool (already vendored). For ephemeral key agreement, use x25519-dalek + chacha20poly1305 directly.
Source: https://github.com/str4d/rage; https://lib.rs/crates/age

yubihsm-rs — NOT for ViCell Cells
Verdict: Requires std (USB/HTTP connectors via rusb and tiny_http). Useful only in host-side tooling, not OS Cells.

Latest v0.42.1. Community crate, not official Yubico. USB support via rusb which requires OS-level USB stack.
ViCell alternative: The Silo (hardware Security Enclave, cells/services/silo) IS ViCell's HSM equivalent. It's already implemented as a hardware capability for G2. YubiHSM is redundant.
Source: https://docs.rs/yubihsm

PKCS#11 (cryptoki crate) — NOT for ViCell
Verdict: PKCS#11 is a dynamic-library loading protocol — fundamentally requires std and OS dynamic linking. Not applicable to no_std Cells.

cryptoki is the best Rust PKCS#11 client. Updated June 2025. But it dlopen()s the PKCS#11 module — requires std file I/O.
ViCell alternative: The Silo capability API (SiloHandle::sign(), ::init_key(), ::get_pub()) is ViCell's native key management interface. It's purpose-built for the SAS model.
Source: https://docs.rs/cryptoki

Trade-Off Matrix
Library	no_std	License	Maturity	Adoption Risk	ViCell Fit	Action
littlefs2 v0.7.2	yes (c-stubs needed)	Apache/MIT + BSD-3	Production	Low — trussed-dev maintains	/data NAND	Already in tree; verify c-stubs
fatfs (git)	yes (alloc)	MIT	Stable	Low	/mnt/sd interop	Already in tree
RedoxFS v0.9.0	yes (default-off std)	MIT	Production	Low-medium; Redox uses it	/srv CoW+encrypt	Already vendored; activate on NVMe
ext4-view	yes (read-only)	MIT	Pre-stable	Medium	VM image reader only	Defer (YAGNI)
sqlite-rs-embedded	yes (unsafe)	MIT?	Pre-release	HIGH — 0 releases	None viable	Reject
TFS	N/A	MIT	Abandoned	N/A	Replaced by RedoxFS	Already dropped
embedded-storage	yes	Apache/MIT	Stable	Low	Flash driver interface	Add when board hw arrives
chacha20poly1305	yes	MIT/Apache	Stable	Low	AEAD for Cells	Add to silo/crypto
aes-gcm	yes	MIT/Apache	Stable	Low	RedoxFS already uses aes	Indirect; add if needed directly
sha2	yes	MIT/Apache	Stable	Low	Digests everywhere	Add to crypto layer
p256	yes	MIT/Apache	Stable (unaudited)	Low-medium	ECDH in embedded-tls	Already indirect via embedded-tls
ed25519-dalek v3	yes (default)	MIT/Apache	Stable (v2) / RC (v3)	Low-medium (v3 RC)	Cell code signing	Add at v3 stable
ring	NO	non-FOSS	Mature	High — no_std blocks	Reject	Reject
embedded-tls v0.19	yes (std feature off)	MIT/Apache	Beta	Medium — WIP	TLS client	Already in tree
rustls	NO (needs std)	MIT/Apache	Production	N/A for Cells	Tier 3 VM only	No action for Cells
age (rage)	NO	MIT	Mature	N/A for no_std	Reject	Use RedoxFS AES-XTS instead
yubihsm-rs	NO	MIT	Stable	N/A for Cells	Host tooling only	Reject for Cells
PKCS#11 (cryptoki)	NO	Apache	Mature	N/A for no_std	Reject	Reject
Ranked Recommendations (Highest Value First)
1. RustCrypto chacha20poly1305 + sha2 — Add immediately to a new libs/crypto or expose via ostd.

Zero adoption risk, no_std first-class, heapless/arrayvec support for stack buffers. These primitives are needed for: Silo software fallback, config file integrity, future Cell-to-Cell authenticated channels.
chacha20poly1305 over aes-gcm because RISC-V has no AES hardware; ChaCha20 is constant-time in pure software.
2. ed25519-dalek (pin 2.1.1 stable now, migrate to v3.0 when released) — Use for ELF Cell signing.

Enables a code-signing chain: build-time sign Cell ELFs, loader verifies before spawning. This closes a real security gap — the loader currently trusts any ELF in VIFS1.
no_std by default in v3. The v3 RC is stable enough to track; wait for 3.0.0 GA before locking in.
3. embedded-storage-async v0.4.1 — Adopt when writing the first real board flash driver (NAND/eMMC).

Becomes the standard interface between VFS littlefs backend and hardware. 60+ crates already implement it; don't reinvent the trait.
Not needed for QEMU VirtIO path. Do not add until G1 board hardware target is concrete.
4. ext4-view (read-only) — Low priority; add only if ViCell needs to read Linux rootfs images in the Tier 3 VMM.

Conditional on Tier 3b ARM64 VMM progress (project-tier3b-arm64-vmm-plan.md). Until then, YAGNI.
5. RedoxFS (already vendored) — No additional work needed. Trigger activation at G2 NVMe milestone per ADR.

What not to do:

Do not add ring, rustls, age, yubihsm, or PKCS#11 to any Cell. None are no_std-compatible.
Do not attempt SQLite in Cells. Use the ekv crate (embedded key-value, no_std) for config persistence if needed.
Do not upgrade littlefs2 — you are already on the latest (v0.7.2).
Limitations of This Research
p256 audit gap: No independent audit has been performed on the elliptic curve arithmetic. RustCrypto's own docs acknowledge this. If ViCell ever uses p256 for something stronger than embedded-tls handshakes (e.g., root-of-trust ECDSA), this should be tracked as a risk item.
embedded-tls cert verification: The no_std path disables webpki cert chain validation. This was not independently verified against the current v0.19 source — the limitation is documented in the README but may have partial fixes in recent commits.
RedoxFS no_std build: Confirmed by reading Cargo.toml feature flags — the std feature is clearly opt-in. However, a full no_std compile against ViCell targets has not been smoke-tested here; the ADR plan includes this as a step.
ed25519-dalek v3 RC stability: v3.0.0-rc.1 is a release candidate as of May 2026. API may still shift before GA. Treat as "track but don't ship" until 3.0.0 final.
ekv / embedded KV alternatives: Not researched here. If config persistence beyond FAT32 is needed before littlefs lands, an embedded KV store comparison would be a separate research task.
Sources:

littlefs2 — trussed-dev/littlefs2
fatfs — rafalh/rust-fatfs
TFS upstream status
RedoxFS — redox-os/redoxfs
ext4-view — crates.io
sqlite-rs-embedded — vlcn-io
embedded-storage — rust-embedded-community
aes-gcm — docs.rs
chacha20poly1305 — docs.rs
sha2 — docs.rs
p256 — docs.rs
ed25519-dalek — curve25519-dalek monorepo
embedded-tls — drogue-iot
rustls-rustcrypto — RustCrypto
yubihsm — docs.rs
cryptoki — PKCS#11 — docs.rs
Awesome Rust Cryptography
ring embedded failure — Rust Forum