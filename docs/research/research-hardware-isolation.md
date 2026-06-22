# Hardware-Assisted Isolation cho Cellos SAS — Nghiên cứu

**Version**: 1.0
**Last Updated**: 2026-06-21
**Scope**: Bổ sung biện pháp phần cứng để cô lập Cell, hỗ trợ (không thay thế) Language-Based Isolation (LBI). Trạng thái ISA/silicon cập nhật tới 2026-Q2.
**Status**: Research — đầu vào cho [security-model.md](../security-model.md) §"Hardware Isolation Roadmap" và [project-roadmap.md](../project-roadmap.md) §G.

---

## 0. Nguyên lý chỉ đạo: tiêu chí "không flush TLB"

Một OS SAS lấy toàn bộ lợi thế từ việc **không đổi address space** giữa các Cell (IPC = vtable call ~2-3 cycle). Vì vậy tiêu chí phân loại quan trọng nhất cho mọi cơ chế phần cứng **không phải** "server vs nhúng", mà là:

> **Cơ chế đó có buộc `sfence.vma` / đổi SATP/CR3 mỗi lần chuyển Cell không?**

Cơ chế bắt flush TLB mỗi lần chuyển Cell sẽ giết chính lợi thế SAS. Đây là lý do per-Cell SATP đã bị loại (xem [security-model.md](../security-model.md)). Các cơ chế "register-switch / tag-based" (MPK, MTE, PAC, CHERI, WorldGuard) là **SAS-friendly** vì chuyển domain chỉ bằng một lệnh ghi thanh ghi, không đổi page-table.

**Con số neo (Singularity / MSR, "Deconstructing Process Isolation", 2006):** hardware isolation tốn thêm **37.7% CPU cycles** so với software isolation; chi phí cơ bản của software isolation (null-check + bounds-check) chỉ ~4.7%. Đây là citation học thuật biện minh cho kiến trúc SAS/LBI.
Nguồn: <https://cs.uwaterloo.ca/~brecht/courses/702/Possible-Readings/oses/singularity-deconstructing-process-isolation-mem-system-perf-2006.pdf>

**Hệ quả thiết kế (đồng thuận toàn ngành):** Pure LBI là đủ *khi và chỉ khi* mọi code do compiler tin cậy biên dịch từ ngôn ngữ verified-safe. Ngay khi nhận unsafe code (C driver/app, Lua FFI, native binary) → bổ sung phần cứng trở thành **bắt buộc**. Cellos ở đúng ranh giới này: Tier 1 Rust thuần dùng LBI; Tier 1b/3b cần supplement.

---

## 1. Phân loại memory-safety: spatial KHÔNG đủ

Danh sách phần cứng thường được nhắc (MPK/PKU, PAuth/PAC, MPU/PMP, MTE) **toàn bộ là bảo vệ không gian (spatial)** — giới hạn *vùng* truy cập. Đó mới là một nửa. Cần đủ ba trục:

| Trục | Câu hỏi | Cơ chế |
|------|---------|--------|
| **Spatial** | Pointer có ra ngoài bounds không? | MPU/PMP, MPK/PKU, MTE, CHERI bounds |
| **Control-flow (CFI)** | Lệnh nhảy gián tiếp có landing đúng chỗ không? | BTI, CET-IBT/Shadow Stack, Zicfilp/Zicfiss |
| **DMA / bus** | Thiết bị có ghi/đọc physical memory tùy ý không? | IOMMU/SMMU, IOPMP, WorldGuard fabric |

Bỏ trục CFI hoặc trục DMA = lỗ hổng có thể vô hiệu toàn bộ trục spatial (xem §3, §5).

---

## 2. Control-Flow Integrity (CFI) — trục bị thiếu

### 2.1 ARM: BTI + PAC

- **PAC** (Pointer Authentication, ARMv8.3): ký/xác thực return address & function pointer — phủ **backward edge**.
- **BTI** (Branch Target Identification, ARMv8.5): mọi indirect branch phải landing trên lệnh `BTI`, nếu không → Branch Target Exception — phủ **forward edge** (JOP/COP).
- **PAC chỉ phủ backward edge.** Nếu chạy PAC mà thiếu BTI → JOP còn mở toang. Hai cơ chế **orthogonal, không thừa**.
- Rust gen sẵn với `-C target-feature=+bti,+pac-ret`; không cần đụng unsafe. Linux kernel BTI từ 5.8 (2020); userspace opt-in qua ELF `GNU_PROPERTY_AARCH64_FEATURE_1_BTI`.
- Nguồn: ARM ARM DDI0487 §D8 <https://developer.arm.com/documentation/ddi0602/2024-09/Base-Instructions/BTI--Branch-target-identification-> · <https://www.kernelconfig.io/config_arm64_bti>

### 2.2 x86_64: Intel CET (Shadow Stack + IBT)

- **Shadow Stack** (backward edge): CPU push return address vào shadow stack write-protected; RET so khớp, lệch → `#CP`.
- **IBT/ENDBR** (forward edge): indirect CALL/JMP phải landing trên `ENDBR64`.
- Silicon: Tiger Lake (11th-gen Intel, 2020)+, AMD Zen 3+ (Shadow Stack). Linux: kernel IBT từ 5.18 (2022), userspace Shadow Stack từ 6.6 (2023); glibc 2024.
- Nguồn: Intel CET spec 334525-003 <https://kib.kiev.ua/x86docs/Intel/CET/334525-003.pdf> · <https://www.kernel.org/doc/html/next/x86/shstk.html>

### 2.3 RISC-V: Zicfilp + Zicfiss

- **Zicfilp** (landing pad, forward) + **Zicfiss** (shadow stack, backward). **Ratified 2024**, chưa có silicon; GCC/LLVM đang triển khai.
- Hành động: bật từ ngày đầu khi SG2044/C930 có silicon.
- Nguồn: <https://www.phoronix.com/news/RISC-V-User-Space-CFI>

### 2.4 ⚠️ CFI là ĐIỀU KIỆN TIÊN QUYẾT của MPK (không phải add-on)

`WRPKRU` là lệnh **user-mode** (ring 3). Kẻ tấn công redirect được control-flow tới một gadget `WRPKRU`/`XRSTOR` có sẵn trong image là **tự cấp mọi protection key** — không cần leo thang đặc quyền.

- **ERIM** (USENIX Security 2019) phải quét toàn bộ code segment để vô hiệu mọi `WRPKRU`/`XRSTOR` không hợp lệ + thêm CFI. <https://www.usenix.org/system/files/sec19-vahldiek-oberwagner_0.pdf>
- **"PKU Pitfalls"** (USENIX Security 2020): 10 PoC bypass thành công ngay cả app đã được ERIM bảo vệ. <https://www.usenix.org/system/files/sec20-connor.pdf>
- **WarpAttack** (IEEE S&P 2023): bypass CFI qua double-fetch. <https://nebelwelt.net/files/23Oakland3.pdf>

Kết luận corpus: **MPK là cơ chế hiệu năng, không phải security boundary — trừ khi đi kèm CFI (BTI/IBT) + binary scan.** Với Cellos, Rust thuần trong Cell trung hòa phần lớn; rủi ro quay lại ở C FFI (mlibc, DOOM), Lua dispatch, unsafe kernel — đúng chỗ gadget space = toàn bộ image.

---

## 3. DMA / peripheral isolation — lỗ hổng nghiêm trọng nhất

### 3.1 Vì sao bắt buộc, bất kể CPU-side isolation

Mọi cơ chế §2, §4, §6 đều là **CPU-side**. Một thiết bị có DMA bỏ qua hoàn toàn CPU, MMU, borrow checker:

> Trong Cellos SAS, **Cell CHÍNH LÀ driver** — nó own MMIO, tự lập trình DMA descriptor, không có kernel trung gian. Blast radius của một Cell-có-DMA bị compromise = **một bug driver kernel**, KHÔNG phải exploit user-space. Nó ghi/đọc *toàn bộ* physical memory (page table kernel, metadata scheduler, stack Cell khác) — không cần một dòng `unsafe`, chỉ cần ghi MMIO mà nó được phép ghi.

So với Linux: process user không đọc được physical memory của process khác kể cả khi không có IOMMU, vì nó không có MMIO tới DMA controller. Cellos Tier 1 Cell *là* driver. ⟹ DMA threat của Cellos ngang một **kernel driver bug của Linux**, không phải user exploit.

- **Thunderclap** (NDSS 2019, Cambridge): PCIe device bypass IOMMU của macOS/Linux/FreeBSD *ngay cả khi đã bật*. <https://thunderclap.io/wp-content/uploads/2024/01/thunderclap-paper-ndss2019.pdf>

### 3.2 ⚠️ Trạng thái Cellos hiện tại: IOMMU passthrough = ZERO isolation

Đã verify ở mức code (2026-06-21): IOMMU RISC-V + VT-d shipped ở **passthrough mode** — RISC-V qua `DDTP.MODE=1` ([iommu_riscv.rs:74](../../kernel/src/task/drivers/iommu_riscv.rs#L74), bare, tắt dịch); x86 qua VT-d **bật dịch** (`TES`) nhưng cả 256 BDF map vào context entry `TT=0b10` passthrough chung một domain ([iommu_x86.rs:98-107](../../kernel/src/task/drivers/iommu_x86.rs#L98-L107)). Cả hai cho IOVA==PA không bảng permission, và `iommu::map_dma()` là identity no-op trả nguyên `phys` ([iommu.rs:31](../../kernel/src/task/drivers/iommu.rs#L31)). Passthrough = về chức năng tương đương **không có IOMMU**. Đây tạo *cảm giác an toàn giả*. Chính comment trong code đã thừa nhận: *"BARE MODE IS NOT SAFE ON REAL HARDWARE"*.

**Cần phân biệt trong kernel:** **MMIO ownership ≠ DMA authorization**. Resource Registry hiện enforce một Cell giữ một vùng MMIO, nhưng Cell giữ MMIO của NIC thì DMA được, giữ MMIO của UART thì không. Phải track **DMA capability tách biệt** và cài entry IOMMU/IOPMP per-device per-Cell:
- Chuyển IOMMU khỏi passthrough → translate mode với bảng IOVA→PA per-device.
- `sys_grant_dma(device, phys, size)` chỉ map đúng các trang được cấp.

### 3.3 Lưu ý nhúng (mục tiêu G1)

Nhiều DMA controller on-chip (DMAC/PDMA) đi qua bus nội bộ **bỏ qua cả SMMU** — bật SMMU vẫn không đủ. Đây là lý do **IOPMP** (RISC-V) tồn tại: checker ở bus fabric, không liên quan TLB của hart. ARM SMMU thường chỉ phủ PCIe (và đôi khi USB).

- Cơ chế: Intel VT-d / AMD-Vi, ARM SMMU v3, RISC-V **IOPMP** (draft v0.9.2-RC3, 1/2025).
- Nguồn: ARM SMMU v3 (pKVM) <https://lwn.net/Articles/1001952/> · IOPMP <https://github.com/riscv-non-isa/iopmp-spec>

---

## 4. x86/server: MPK/PKU + PKS

| Đặc tính | Chi tiết |
|----------|----------|
| Cơ chế | 4-bit key/PTE + thanh ghi PKRU (2 bit/key: access-disable, write-disable) |
| Chi phí switch | `WRPKRU` ~20 cycle (vs process switch 1K-10K cycle); **không syscall, không TLB flush, không IPI** |
| Giới hạn | **16 key (4-bit)** — không gán per-Cell khi có hàng chục Cell |
| Khả dụng | Intel Skylake+; AMD PKU user-mode từ Zen 1 (2017) |

**Intel PKS** (Protection Keys for **Supervisor**): 16 key phía kernel (Ice Lake+), lý tưởng bảo vệ Cell Registry / Frame Allocator / grant table khỏi syscall handler bị lợi dụng. Linux 5.13 (2021). **AMD KHÔNG có PKS** → feature-gate.

**Giới hạn 16 key** → nhóm theo **tier** (Key1=kernel-trusted, Key2=service, Key3=app/untrusted) khớp mô hình tier Cellos; hoặc multiplex kiểu **libmpk** (có chi phí eviction). **Đừng** per-Cell key.

**Corpus nghiên cứu intra-AS isolation (biết trước khi quyết định):**

| Hệ thống | Venue/Năm | Đóng góp |
|----------|-----------|----------|
| ERIM | USENIX Sec 2019 | Binary scan vô hiệu gadget + trusted gate; ~2% overhead nginx |
| Hodor | USENIX ATC 2019 | WRPKRU chỉ reachable qua trampoline (single-step/breakpoint) |
| libmpk | USENIX ATC 2019 | Virtualize 16 key → domain tùy ý (key multiplexing) |
| Donky | USENIX Sec 2020 | Mở rộng RISC-V/ARM; chỉ ra ERIM scan unsound dưới JIT |
| xMP | IEEE S&P 2020 | MPK cho kernel-side data isolation |
| IskiOS | EuroSec 2020 | Shadow stack qua MPK |
| Cerberus | ASPLOS 2022 | Browser renderer compartments + constrained JIT |

**Adoption production:** OpenSSL 3.0+ (bảo vệ private key), Firefox/SpiderMonkey (JIT page 2020), Linux BPF JIT (5.2+), V8 Sandbox (2023, thử nghiệm). Hầu hết là **heap hardening**, không phải full compartmentalization. glibc expose `pkey_alloc/free/mprotect` từ 2.27.

**ARM không có MPK analogue** → trên RK3588 dùng PAC+BTI + MTE + Stage-2.

Nguồn: ERIM (trên) · libmpk <https://www.usenix.org/conference/atc19/presentation/park-soyeon> · Donky <https://www.usenix.org/conference/usenixsecurity20/presentation/schrammel> · Intel SDM Vol 3A §4.6.

---

## 5. Embedded: MPU/PMP và mô hình dual-tier

### 5.1 Tham khảo trực tiếp: Tock + Hubris

- **Tock OS** (Rust nhúng): **capsule** (trusted, in-kernel) = Rust type system, **zero** MPU; **process** (untrusted, C app) = ARM MPU / RISC-V PMP reconfigure mỗi context switch. ⭐ Đây là mô hình map thẳng cho Cellos: Tier 1 Rust thuần = capsule (đừng trả giá MPU vô ích); Tier 1b (C/Lua/DOOM) = process (gate sau MPU/PMP). "Grant" của Tock: kernel cấp heap *bên trong* memory của process nhưng MPU chặn process đọc — đúng pattern owned-buffer.
  - TickTock (SOSP 2025) verify formal isolation invariants bằng Verus — xác nhận safe Rust *một mình* không đủ làm *proof* nếu không verify unsafe boundary.
  - Nguồn: <https://book.tockos.org/doc/threat_model/capsule_isolation> · <https://ranjitjhala.github.io/static/sosp25-ticktock.pdf>
- **Hubris** (Oxide): task tĩnh compile-time, MPU region baked vào image, reconfigure khi switch. Oxide nói thẳng: *"kể cả Rust, memory protection vẫn thiết yếu."* IPC "lease" = đúng Law 2 owned-buffer; supervisor restart = đúng mô hình never-die Cellos.
  - Nguồn: <https://hubris.oxide.computer/reference/> · <https://oxide.computer/blog/hubris-and-humility>

### 5.2 PMP dưới Bare mode (tin tốt cho Cellos)

Theo RISC-V priv spec: `sfence.vma` chỉ cần sau khi ghi PMP CSR **nếu page-based VM được implement**. Dưới `satp=Bare` (Cellos SAS) → ghi PMP **không cần** sfence.vma → SAS-safe. Nhược: O(N) ghi CSR cho N entry mỗi switch.

**Smepmp** (ePMP, **ratified** ~2022): thêm `mseccfg.MML`/`MMWP` để M-mode tự-từ-chối / default-deny — củng cố self-protection. Không thêm số entry, không switch nhanh hơn.

---

## 6. RISC-V: menu BEYOND PMP

Dưới `satp=Bare`, các cơ chế dưới đây hầu hết không flush TLB. Xếp theo độ phù hợp SAS:

| Cơ chế | Ratify (6/2026) | Cô lập gì | Flush TLB khi switch? | Phù hợp SAS |
|--------|-----------------|-----------|----------------------|-------------|
| PMP / Smepmp | ✅ Ratified | Vùng physical, M-mode | Không (Bare) | Đang dùng; O(N) CSR/switch |
| sPMP / Sspmp | ❌ Chưa (target 12/2026) | S-mode physical | Không (Bare) | 1 bitmask CSR; chưa silicon |
| Pointer Masking (Zjpm/Ssnpm/Smnpm) | ✅ **Ratified v1.0** | substrate tagging | Không | Nền cho tagging, **không tự cô lập** |
| Zimt (Memory Tagging ~MTE) | 🔶 Draft v0.2 (9/2025) | UAF/overflow | Không (in-line) | Hardening, không phải access-control |
| **Smmtt** (Smsdid + Smmpt) | 🔶 Draft | **Physical domain per-SDID** | Chỉ MTT-fence (nhẹ hơn SATP) | ⭐ Rất mạnh, dài hạn |
| **WorldGuard** (Smwg/Smwgd/Sswg) | 🔶 Draft v0.4 (QEMU 4/2025) | Tag WID mọi bus transaction | **Không — 1 CSR write** | ⭐⭐ Khớp nhất + phủ DMA |
| IOPMP | 🔶 Draft v0.9.2 (1/2025) | DMA / bus master | N/A (bus-side) | Bắt buộc đi kèm |
| CoVE / AP-TEE | 🔶 Draft | Confidential VM (trên Smmtt) | (sai tầng cho SAS) | Tiêu thụ Smmtt trực tiếp |

**Hai ứng viên nổi bật:**

- **WorldGuard** (gốc SiFive → RISC-V Int'l): mỗi hart mang **WID** (≤32 world); mọi transaction (kể cả instruction fetch, page-walk) bị tag WID; checker mỗi vùng nhớ so WID với ACL. **Switch Cell = ghi 1 CSR, không sfence, không đổi page-table** — domain switch nhanh nhất. WID propagate qua bus fabric ⟹ **phủ luôn DMA**. Hạn chế: ≤32 world, cần SoC hỗ trợ fabric checker (SiFive P/E series). QEMU support 4/2025.
  - Nguồn: <https://lists.riscv.org/g/security/attachment/685/0/worldguard_rvia_spec.pdf> · <https://www.sifive.com/press/sifive-gives-worldguard-to-risc-v-international-to>
- **Smmtt / Smsdid** (Supervisor Domains): SDID = danh tính Cell; bảng MPT (radix theo physical page) quy định SDID nào truy cập trang nào, hardware enforce mọi access. Switch = ghi SDID CSR + MTT-fence (nhẹ hơn full SATP, **không đổi virtual address space**). Memory-grant API Cellos nên thiết kế quanh permission-theo-physical-page để map thẳng vào MTT.
  - Nguồn: <https://github.com/riscv/riscv-smmtt/blob/main/CHARTER.adoc>

**Lưu ý:** Pointer Masking & Zimt **không tự cô lập** — Cell tự chọn tag bất kỳ; là công cụ memory-safety (UAF), không phải access-control. sPMP chậm ratify hơn WorldGuard cho cùng lợi ích. CoVE là sai tầng cho native OS (tiêu thụ Smmtt trực tiếp).

---

## 7. Confidential Computing — threat model MỚI (chống compromise kernel/hypervisor)

LBI **tin tưởng kernel**. TDX/SEV-SNP/CCA **không**. Đây là threat model riêng biệt, không trùng — đích đến cho Tier 3 + multi-tenant attestation.

| Công nghệ | Bản chất | Trạng thái |
|-----------|----------|------------|
| **AMD SEV-SNP** | VM mã hóa bộ nhớ + RMP chống hypervisor alias | GA Google Cloud/Azure 2024 |
| **Intel TDX** | Trust Domains, hypervisor không đọc TD memory | Sapphire/Emerald Rapids; GA cloud 2024-25 |
| **ARM CCA / RME / GPT** | Phân vùng physical granule 4 world (Normal/Secure/Root/Realm) ở **tầng bus**, EL3 firmware quản | ARMv9.3; **chưa silicon đại trà** — Fujitsu Monaka ~FY2027 |
| **RISC-V CoVE** | ABI confidential VM trên Smmtt | Draft, cần H-ext |

- Overhead (SIGMETRICS 2025, TUM): 0-15%, compute-bound <3%. <https://dl.acm.org/doi/10.1145/3700418>
- **ARM CCA** đặc biệt mạnh: enforce ở physical granule (GPT do EL3 quản, OS/hypervisor bị chiếm cũng không vượt) — superset của MPK/MTE, boundary đúng cho Tier 3 "không tin OS". Linux `arm64/arm-cca` đã merge. <https://www.arm.com/architecture/security-features/arm-confidential-compute-architecture>
- **Liên hệ Cellos:** VMM custom ~9K-LOC hiện tại phù hợp G1/G2 bare-metal. Nên thiết kế **`VmHandle` ABI trung lập** ngay để sau backing bằng TDX/SEV-SNP/CCA mà không redesign protocol Cell↔VM — mở khóa **remote attestation** (mở rộng nguyên tắc Silo cho multi-tenant).

---

## 8. CHERI — endgame phần cứng cho SAS

CHERI capability thay **cả** MMU-isolation **lẫn** language-isolation: mọi pointer là (address, bounds, permissions, tag); hardware check mọi load/store; tag chỉ set bởi hardware (không giả được). **CheriOS** (Cambridge 2021) đã chứng minh đúng mô hình Cellos: MMU dùng cho hiệu năng (TLB cache), capability dùng cho isolation, **không TLB flush khi switch domain**, chạy được cả C.

**Trạng thái 2026 (quan trọng — sửa kỳ vọng cũ):**

| Nền tảng | Trạng thái | Loại |
|----------|-----------|------|
| **CHERIoT-IBEX** (Sonata FPGA ~$412, SCI ICENI silicon EAP 2025) | ✅ Mua được; Rust no_std fork active (cập nhật hằng tuần từ 2/2026) | **RV32E** (embedded) |
| RISC-V "Zcheri" extension | ❌ **Chưa ratify** (target đầu 2026, đã trượt) | — |
| RV64 CHERI silicon | ❌ Không có (COSMIC nhắm secure-enclave 3/2028, chưa tape-out) | FPGA/research |
| **ARM Morello** | ❌ **ARM tuyên bố khai tử** — không sản phẩm, không kế thừa có tên | EoL |

- CHERIoT compartment switch: **209-452 cycle** (empty–256B stack) — nhanh hơn null syscall (SOSP 2025). Switcher = stub assembly ~355 lệnh audited, không page-walk/TLB flush/ring transition.
- **Kết luận:** Phase 31 CHERIoT-IBEX cho **Nano profile (RV32)** là đúng và de-risked. **Đừng** plan RV64 CHERI cho 2026-Q4 — không silicon, không Rust target, ISA chưa ratify. Realistic cho RV64: 2028-2030 nếu "Zcheri" ratify + silicon ship.
- Nguồn: <https://riscv.github.io/riscv-cheri> · <https://www.arm.com/architecture/cpu/morello> · <https://rust.cheriot.org/2026/02/15/status-update.html> · CheriOS UCAM-CL-TR-961 <https://www.cl.cam.ac.uk/techreports/UCAM-CL-TR-961.pdf>

---

## 9. Cảnh báo về các cơ chế "đã biết"

- **MTE không phải security boundary.** **TikTag (2024)** phá MTE bằng speculative gadget. MTE = phát hiện UAF/overflow theo xác suất (1/16 miss), **không** chống forgery có chủ đích. Vị trí đúng: hardening layer. <https://arxiv.org/html/2406.08719v1>
- **Prior art tagging:** **SPARC ADI** (Oracle M7, 2015) đã làm memory tagging 4-bit production trước MTE cả thập kỷ (granule 64B). Cùng giới hạn 4-bit/16-class. <https://www.kernel.org/doc/html/v5.7/sparc/adi.html>
- **Spectre vẫn là vấn đề mở** cho mọi hệ LBI/capability. RedLeaf punt sang hardware tương lai; Theseus lập luận structurally ít phơi nhiễm hơn (không secret giữa trust level); CheriOS không có lời giải. IOMMU (chống DMA) là supplement giá-trị-cao nhất gần hạn.

---

## 10. So sánh OS SAS/LBI khác (cách dùng phần cứng)

| OS | Cô lập bằng | Phần cứng | Bài học cho Cellos |
|----|-------------|-----------|--------------------|
| **Tock** | Capsule=Rust; Process=MPU/PMP | ARM MPU / RISC-V PMP | ⭐ Dual-tier: Rust thuần không cần MPU; C-tier mới cần |
| **Hubris** (Oxide) | Rust + MPU per-task tĩnh | ARM MPU (bắt buộc) | "lease"=Law 2 owned-buffer; supervisor=never-die |
| **RedLeaf** (OSDI'20) | Rust domain, no page-table | Không | **RRef\<T\>** ownership enforce bởi IDL → cân nhắc Cellos IDL |
| **Theseus** (OSDI'20) | LBI thuần, Ring 0 | Không (cố ý) | Cảnh báo: không chạy được C bên thứ ba — Tier 1b/3b Cellos trung thực hơn |
| **Singularity/SIP** (MSR) | Sing# + sealed + channel contract | Không | 37.7% number; channel-contract FSM (tránh discriminant collision) |
| **Mungi/Opal/Nemesis** (SAS 90s) | Password/segment capability | TLB chỉ để dịch địa chỉ | Capability handle bất khả giả ở mức type |
| **CheriOS** | CHERI capability | CHERI | Endgame G3: chạy mọi ngôn ngữ không cần Stage-2 |

Nguồn: RedLeaf <https://www.usenix.org/conference/osdi20/presentation/narayanan-vikram> · Theseus <https://www.usenix.org/system/files/osdi20-boos.pdf> · Singularity <https://www.microsoft.com/en-us/research/wp-content/uploads/2005/10/tr-2005-135.pdf> · Mungi <https://trustworthy.systems/publications/papers/Heiser_ERV_94.pdf>

---

## 11. Khuyến nghị theo tier (bám tiêu chí flush-TLB)

**Embedded (ARM/RISC-V MCU, G1):**
- Tier 1 Rust thuần → **LBI only** (capsule model của Tock; không trả giá MPU).
- Tier 1b (C/Lua/DOOM) → gate sau **MPU (ARM) / PMP (RISC-V)** (process model Tock+Hubris).
- **IOPMP** bắt buộc trong BOM nếu có thiết bị DMA.
- Nano RV32 → **CHERIoT-IBEX** (Phase 31).

**Server/PC (G2):**
- x86: **MPK/PKU coarse-tier (3 key)** + **PKS** (Intel) bảo vệ metadata kernel — **CHỈ khi đã bật CET (IBT+Shadow Stack)** vì gadget WRPKRU. AMD G2 không có PKS → feature-gate.
- ARM64 (RK3588): không MPK → **PAC + BTI** (full CFI) + **MTE** (hardening) + **Stage-2** cho Tier 3.
- RISC-V server: theo dõi **WorldGuard** (gần hạn), **Smmtt/Smsdid** (dài hạn); bật **Zicfilp/Zicfiss** khi có silicon.

**Bắt buộc mọi tier:** **DMA isolation** — chuyển IOMMU khỏi passthrough; tách DMA-capability khỏi MMIO-ownership.

**Tier 3 / multi-tenant (G2→G3):** VMM ABI trung lập để slot **TDX/SEV-SNP/ARM CCA** (attestation).

---

## 12. Xếp hạng lỗ hổng (cho codebase hiện tại)

| # | Lỗ hổng | Mức | Tại sao |
|---|---------|-----|---------|
| 1 | **IOMMU passthrough = zero DMA isolation** | 🔴 CRITICAL | Đã ship, cảm giác an toàn giả. Cell-có-DMA = own physical memory |
| 2 | **Chưa có forward-edge CFI (BTI/IBT)** | 🟠 HIGH | MPK chỉ đúng khi có CFI; C-tier mở rộng gadget surface |
| 3 | **MMIO-ownership ≠ DMA-authorization** | 🟠 HIGH | Phải là 2 capability tách biệt trong kernel |
| 4 | **PAC chưa ghép BTI** | 🟡 MED | Chỉ phủ backward edge; chênh 1 compiler flag + ELF marking |
| 5 | Zicfiss/Zicfilp RISC-V | 🟢 LOW (tương lai) | Ratified 2024, chờ silicon — bật từ ngày đầu khi có |
| 6 | ARM CCA / RME | 🟢 LOW (tương lai) | Chưa silicon <2027; chỉ cần ABI sẵn sàng |

---

## 13. Tham chiếu chéo

- [security-model.md](../security-model.md) — STRIDE, Layer 1/2/3, CHERI roadmap (đã cập nhật từ doc này)
- [project-roadmap.md](../project-roadmap.md) §G — Security Platform backlog
- [specs/04-hardware.md](../specs/04-hardware.md) — Multi-arch HAL
- [specs/12-reliability.md](../specs/12-reliability.md) — never-die, isolation strategy decision
- Memory: `project-iommu-pcie-nic-track-b` (IOMMU passthrough status), `project-tier3-hypervisor-strategy`, `project-g2-riscv-server-strategy`
