# ViCell Cell Security: Permission Model + Hardware Attestation — Nghiên cứu

**Version**: 1.0
**Last Updated**: 2026-06-21
**Scope**: Mô hình phân quyền per-Cell (least-privilege, user/operator control) + chuỗi tin cậy phần cứng (security chip, secure/measured boot, attestation). Tham khảo capability OS + mobile/macOS + hardware RoT.
**Status**: Research — đầu vào cho [project-roadmap.md](../project-roadmap.md) §G và [security-model.md](../security-model.md). Bổ sung cho [research-hardware-isolation.md](research-hardware-isolation.md) (cô lập bộ nhớ phần cứng — orthogonal với doc này).

---

## 0. Câu hỏi và câu trả lời ngắn

> "Có nên học mobile/macOS: cấp quyền tối thiểu cho Cell (không full), cho user kiểm soát quyền Cell dùng?"

**Có** cho least-privilege + cấp tường minh + thu hồi. **Cần tái cấu trúc** ý "user control":

iOS tách bạch hai khái niệm mobile thường gộp:
- **Entitlements** — quyền *tĩnh*, ký vào binary, là **trần** (ceiling) không vượt được. → **= ELF manifest hiện tại của ViCell.**
- **TCC** — đồng ý *runtime* do user kiểm soát, thu hồi được. → **= thứ ViCell đang thiếu.**

⚠️ **Caveat headless robot (tái định nghĩa toàn bộ):** consent dialog là **UX primitive, không phải security primitive**. Robot/drone/server không có người bấm "Allow". Vì vậy:

| Stage | "User control" = |
|-------|------------------|
| **G1 (robot/nhúng, thường headless)** | **Operator/fleet POLICY đã ký** (kiểu ROS 2 SROS2), KHÔNG phải dialog |
| **G2 (desktop/HMI)** | Dialog kiểu TCC, **chỉ cho cap nhạy cảm** (camera/mic/storage), áp anti-fatigue |

Cơ chế nền (manifest + kernel enforcement + scoping + revocation) tách rời UI consent. ViCell cần phần nền; dialog chỉ tùy chọn cho G2 HMI.

---

## 1. Trạng thái hiện tại của ViCell (thành thật)

| Khía cạnh | Hiện tại | Khoảng trống |
|-----------|----------|--------------|
| Granularity | `flags: u8`, 8 bit boolean ([manifest.rs:23-69](../../libs/api/src/manifest.rs#L23-L69)), **đã dùng hết** | GPIO = *mọi* pin; NETWORK = *mọi* packet; không tham số |
| Nguồn quyền | Cell tự khai trong ELF manifest | init không siết thêm được; `/bin/` path-grant = ambient authority |
| Routing | Kernel đọc manifest, cấp all-or-nothing lúc spawn | Không delegation cha→con, không scoping trung gian |
| Revocation | Không — cap sống tới khi Cell chết | Không thu hồi NETWORK của Cell hư lúc runtime |
| Consent/control | Không có | Không có user/operator control |

→ Tương đương **mô hình install-time của Android *trước* 6.0** (Felt et al., CCS 2011, đã chứng minh hỏng về informed-consent). Manifest đụng `libs/api/` ⇒ **Law 1** (2x confirmation) khi đổi struct.

Đã có (roadmap §G): Cell signing (planned), KMS Cell (planned), Silo (done, ARM64/x86 G2), audit ring ([audit.rs](../../kernel/src/audit.rs) — có event spawn, không có content hash), syscall allowlist `__ViCell_syscalls`.

---

## 2. LUỒNG A — Mô hình phân quyền phần mềm

### 2.1 Bài học từ capability OS

| OS | Cơ chế cốt lõi | Bài học cho ViCell |
|----|----------------|---------------------|
| **seL4** | Capability là primitive duy nhất; CDT (Capability Derivation Tree) revoke đệ quy; **badge** 64-bit phân biệt client trên cùng endpoint; mint chỉ subset rights | Lấy pattern **badge** (token có tham số), không lấy độ phức tạp CDT |
| **Fuchsia/Zircon** ⭐ | 2 tầng: `zx_handle_t` (kernel, rights bitmask, downgrade đơn điệu, close = revoke tức thì) + **Component Framework**: `.cml` manifest khai `use`/`offer`/`expose`, cap **route tường minh** parent→child, **không ambient authority, không global namespace** (`/dev`,`/proc` không tồn tại) | Khớp gần 1:1 với 2 tầng ViCell (kernel ZST + ELF manifest). **Reference chính** |
| **Genode** | Session = capability **có tham số** (IO_MEM = dải PA cụ thể & exclusive, ROM = label); routing whitelist trong `init.xml`; child ⊆ parent; reference-PD chống quota-steal | Tham số scope hóa cap; `init` làm routing table |
| **Capsicum/CloudABI** | `cap_enter()` bỏ ambient authority; `cap_rights_limit(fd, rights)` thu hẹp per-fd, **chỉ giảm không tăng** (one-way ratchet); CAP_IOCTL có whitelist | Quyền **giảm đơn điệu** khi delegate xuống; tham số trong cap (ioctl whitelist) |

**4 bất biến ở MỌI hệ** (ViCell hiện vi phạm cả 4):
1. **Không ambient authority** — không có quyền ngầm theo tên/namespace.
2. **Delegation tường minh** — chỉ nhận cap được trao rõ ràng.
3. **Downgrade đơn điệu** — delegate xuống chỉ thu hẹp, không mở rộng.
4. **Revocable** — thu hồi được.

Nguồn: seL4 manual <https://sel4.systems/Info/Docs/seL4-manual-latest.pdf> · Fuchsia capabilities <https://fuchsia.dev/fuchsia-src/concepts/components/v2/capabilities> · `.cml` ref <https://fuchsia.dev/reference/cml> · Genode Foundations 25.05 <https://genode.org/documentation/genode-foundations-25-05.pdf> · Capsicum (USENIX Sec 2010) <https://www.cl.cam.ac.uk/research/security/capsicum/>

### 2.2 Bài học từ mobile/desktop (consent-based)

**Android** — 3 tầng compose: manifest khai → PackageManager cấp (normal = install-time, **dangerous = runtime prompt** từ 6.0/2015) → **SELinux MAC** enforce ở kernel bất kể userspace nghĩ gì → AppOps audit. One-time permission (11), auto-revoke app không dùng >3 tháng (11). Bài học: **không phải mọi quyền rủi ro ngang nhau** (normal vs dangerous); MAC không bị consent override.

**iOS** — **Entitlements** (ký, kernel check lúc exec, là trần) vs **TCC** (`tccd`, prompt per-resource, lưu `TCC.db`, revoke bất kỳ lúc nào). **Purpose strings** bắt buộc (`NSCameraUsageDescription` — app phải *giải thích tại sao*). App Sandbox (Seatbelt) = BSD MAC. Privacy manifest (17+).

**macOS** — TCC giống iOS nhưng yếu hơn (nhiều bypass); App Sandbox optional ngoài App Store; **SIP** = sàn cứng kernel (root cũng không ghi `/System`).

Nguồn: Android permissions <https://developer.android.com/guide/topics/permissions/overview> · Felt CCS 2011 <https://dl.acm.org/doi/10.1145/2046707.2046779> · iOS entitlements <https://developer.apple.com/documentation/bundleresources/entitlements> · macOS App Sandbox <https://developer.apple.com/documentation/security/app_sandbox>

### 2.3 ⚠️ Pitfall PHẢI tránh (có CVE thực)

- **Consent fatigue** — user bấm "Allow" phản xạ dưới tải nhận thức (Egelman, CHI 2013). Android: xin lúc cài → 40% từ chối; xin đúng lúc dùng → từ chối thấp 3×. → G2 prompt **cực hạn chế**.
- **TCC bypass qua injection** — CVE-2020-9771, CVE-2021-30713 (XCSSET lợi dụng grant của Zoom), CVE-2023-26818 (dylib injection vào process entitled). Gốc rễ: **TCC check binary, không verify code đang chạy** → inject sau khi đã cấp. → **ViCell mạnh hơn ở đúng điểm này**: LBI + Rust type system ngăn code injection ⇒ lỗ "permission laundering" của TCC **không tồn tại** trong ViCell.
- **Permission ≠ enforcement** — SE-Android (NDSS 2013): permission userspace vô nghĩa nếu thiếu MAC ở kernel. → ViCell enforce ở **syscall boundary (Law 1)** là đúng; consent phải **feed vào** điểm enforce đó.
- **Granularity vs usability** — Android permission-groups (gộp để dễ hiểu → over-permission); iOS phải 7 năm sau mới thêm "Limited Photos". → Định granularity ở **cap-type level** (camera/network/gpio-bank), tinh hơn để sau.

Nguồn: Egelman CHI 2013 · Wardle/Objective-See TCC CVE annual · SE-Android NDSS 2013.

### 2.4 Lộ trình tiến hóa cho ViCell (u8 bitmask = tầng 1/3)

**Tầng 1 — Parameterized capabilities** (ngay; additive qua ELF section mới `__ViCell_cap_args` → **KHÔNG cần Law 1**, backward-compat: vắng = full-scope):
```
GPIO     → pin_mask: u64        (chỉ pin 14-17)
BLOCK_IO → lba_range: [u64;2]   (chỉ dải LBA này)
NETWORK  → proto_mask + host/port allowlist (chỉ mqtt:1883)
UART     → port_mask: u8
MMIO     → (PAddr, len)         (= Genode IO_MEM session)
```
Kernel lưu `(cap_type, params)` thay vì ZST boolean; mọi syscall enforce check `params`. = Genode session-args + Capsicum CAP_IOCTL whitelist. ⚠️ Cân nhắc chi phí validate per-syscall ở hot path (GPIO toggle, block read).

**Tầng 2 — Spawn-time intersection (delegation)** (đổi protocol init):
`sys_spawn(path, granted_caps)` → kernel giao `min(granted_caps, cap_của_cha)`, dư bị strip. Cell không cấp con quyền nó không có → chain-of-custody, diệt confused-deputy (Genode/Fuchsia monotonic downgrade). `init` giữ routing table (kiểu `init.xml`/`.cml`).

**Tầng 3 — Runtime revocation** (sau, khi có use case): `CapHandle` kernel object; grantor gọi `sys_cap_revoke(handle)` → kernel set `task.cap = None`; syscall kế tiếp trả `ViError::CapRevoked`; Cell nhận `AppEvent::CapRevoked`. Đơn giản hơn seL4 CDT (ViCell chưa có cap-to-cap derivation).

### 2.5 Tầng consent (trả lời "user control")

**G1 — operator policy model** (như ROS 2 SROS2 nhưng ở kernel level):
```
1. Cell ELF manifest khai cap cần (giữ nguyên cơ chế)
2. Operator ký POLICY FILE (TOML/JSON + Ed25519) lúc fleet provision:
   cell "sensor-reader" MAY USE [gpio::bank0, i2c::bus1, net::mqtt]
3. Kernel verify chữ ký policy với fleet root CA (lưu VIFS1) lúc boot
4. Spawn: cap = min(manifest ∩ policy)
5. Không dialog. Revoke = push policy mới + hot-revoke cap bit
6. Audit: kernel log cap-use (kiểu AppOps) → forward fleet endpoint
```
Nguồn: ROS 2 DDS-Security / SROS2 <https://design.ros2.org/articles/ros2_dds_security.html>

**G2 — consent-broker Cell** (TCC-analog, chỉ cap nhạy cảm): kernel gửi `CapConsentRequest` IPC tới consent-broker Cell (trusted) lúc first-use → render dialog (purpose string bắt buộc) → lưu signed consent-db. Anti-fatigue: chỉ first-use, one-time option, auto-revoke sau N ngày, batch cap liên quan (camera+mic).

### 2.6 Bất biến cứng (không vi phạm)
1. Manifest là **trần, không phải sàn** — không cap nào ngoài manifest kể cả khi consent (iOS entitlement).
2. **Chỉ kernel enforce** — quyết định consent cập nhật cap bitmap trong kernel, không nằm ở daemon bypass được (chỗ TCC chết).
3. **Purpose string bắt buộc** cho cap nhạy cảm — auditable kể cả không có user.
4. Revoke **đồng bộ**, không grace window.
5. **One-time caps** (G2) — flag `cap::Temporary` + TTL (= iOS one-time permission).

### 2.7 Cái KHÔNG map sang ViCell
| Pattern mobile/desktop | Vì sao không hợp |
|------------------------|-------------------|
| Consent dialog first-use | Không có user trên robot headless; block execution |
| App Store/Gatekeeper authority | Không có distribution platform; ViCell cần PKI riêng |
| TCC enforce ở process boundary | SAS không có process boundary → enforce ở syscall |
| UID-based SELinux sandbox | SAS không có UID; Rust type + cap bit thay thế |
| Permission laundering qua injection | LBI ngăn injection — attack surface này không tồn tại |

---

## 3. LUỒNG B — Phần cứng: chip bảo mật + attestation

### 3.1 Hardware Root of Trust — ranking open-source-first cho RISC-V

| Công nghệ | Cung cấp | Open | RISC-V | Verdict |
|-----------|----------|------|--------|---------|
| **OpenTitan** (Earl Grey/Darjeeling) ⭐ | Device identity, measured boot, attestation, key storage (OTBN ECC), OTP lifecycle | ✅ Apache 2.0 RTL→FW | ✅ Ibex RV32 | **#1 — backing tự nhiên cho Silo.** Silicon production (Chromebook 2025); CHERIoT/ICENI build trên Earl Grey |
| **Caliptra** (OCP: MS/Google/AMD/NVIDIA) | DICE CDI, đo firmware SoC lúc boot, ML-DSA hậu-lượng-tử | ✅ Apache 2.0 | ✅ VeeR EL2 | #2 — bổ sung (boot-time measurement); datacenter-first; cho custom SoC G3 |
| **RISC-V Keystone** | TEE dùng PMP, nonce attestation | ✅ BSD | ✅ native | #3 — bridge phần mềm; ⚠️ M-mode TCB bloat (DORAMI, USENIX'25), trần PMP entry; academic |
| **Apple SEP** | (tham chiếu thiết kế) | ❌ | ❌ | **Silo ViCell = tương đương SEP** (coproc riêng, mailbox, UID không rời) |
| ARM TrustZone/OP-TEE | TEE, GP TEE API | ⚠️ OP-TEE BSD | ❌ ARM | Fallback ARM64 thiếu EL2; ⚠️ chung CPU → side-channel xuyên world |
| Google Titan M2 | (bằng chứng RISC-V security chip rời khả thi) | ❌ | ✅ | Proprietary, Android-coupled |
| Microsoft Pluton | RoT trên die CPU | ❌ | ❌ | Bài học "co-locate, bỏ bus"; không portable |
| TPM 2.0 (d/f) | PCR measured boot, sealed storage, Quote | spec mở, silicon ❌ | ❌ ecosystem | Không có path bare-metal RISC-V (cần UEFI); fTPM có faulTPM (S&P'23) |

**Kết luận:** Silo ViCell (Stage-2 fence, P-256, key an toàn dù kernel bị chiếm) **tương đương Apple SEP về kiến trúc**. Tiến hóa: giữ API `ostd::silo::SiloHandle`, đổi backend Stage-2 mailbox → **OpenTitan** (Earl Grey rời qua SPI, hoặc Darjeeling IP nhúng SoC G3). Silo hiện là xấp xỉ phần mềm; OpenTitan là hiện thực phần cứng.

Nguồn: OpenTitan <https://opentitan.org/> · ACM TECS 2024 <https://dl.acm.org/doi/full/10.1145/3690823> · Caliptra <https://github.com/chipsalliance/Caliptra> · Keystone (EuroSys'20) <https://keystone-enclave.org/> · DORAMI (USENIX Sec'25) · Apple Platform Security <https://help.apple.com/pdf/security/en_US/apple-platform-security-guide.pdf> · OP-TEE <https://optee.readthedocs.io/> · faulTPM <https://arxiv.org/abs/2304.14717>

### 3.2 Robot/IoT cần gì từ RoT (4 yêu cầu không thể thiếu)
1. **Device identity bất biến** — key pair fuse phần cứng (EK/AK; OpenTitan Creator Identity) chứng minh "device #42, không phải clone/emulator", sống qua firmware update.
2. **Secure + Measured boot** — mỗi tầng ký + đo; robot bị tamper phải bị phát hiện trước khi rejoin fleet.
3. **Fleet attestation** — verifier từ xa xác minh state không cần chạm device.
4. **Sealed storage** — bí mật fleet (TLS key, calib) bind vào device identity + boot state; tự bất khả truy nếu firmware bị thay/clone.

### 3.3 Secure Boot vs Measured Boot
- **Secure Boot** (enforce): mỗi tầng từ chối chạy code chữ ký sai vs key fuse (ROTPK). Cho **prevention**.
- **Measured Boot** (observe): mỗi tầng hash tầng kế, extend PCR/hash-chain, chạy tiếp bất kể. Cho **evidence** (audit trail không giả được).
- Robot cần **cả hai**. ARM64: TF-A TBBR (BL1→BL2→BL31, ROTPK trong eFuse). RISC-V: OpenSBI + U-Boot FIT image (RSA sign, key trong U-Boot DTB) — RISC-V **không có chuẩn ROTPK eFuse**, OpenTitan/board OTP lấp chỗ này.

Nguồn: TF-A TBBR <https://trustedfirmware-a.readthedocs.io/en/latest/design/trusted-board-boot.html> · U-Boot FIT <https://docs.u-boot.org/en/latest/usage/fit/signature.html>

### 3.4 DICE/RIoT — attestation phân tầng KHÔNG cần TPM (đúng cho robot)
Robot thường không TPM. **DICE** (TCG) chỉ cần **UDS** fuse OTP + hàm một chiều:
```
CDI_n = HKDF-SHA512(IKM: UDS_or_CDI_{n-1}, info: HASH(firmware_n) || metadata_n)
```
Mỗi tầng đo tầng kế, dẫn xuất CDI + **AliasKey** ký cert (chain rooted ở device identity). Firmware đổi → CDI đổi → cả downstream đổi. UDS chỉ truy cập được lúc reset trước mutable code đầu tiên, rồi lock.

**RIoT** (MS Research) = impl nhúng canonical (<1KB trusted code): nhận CDI → DeviceID keypair (identity ổn định) + AliasKey (attestation cho firmware state) → cert chain (X.509/CWT) → trao OS, xóa CDI.

Impl mở: `google/open-dice` (Apache 2.0), TF-M DPE (2024). **Chưa có Rust no_std crate** → tự implement bằng `hkdf` + `ed25519-dalek` + `coset`. Android bắt buộc DICE (VINTF 2023+) → low abandonment risk.

Nguồn: Open Profile for DICE <https://pigweed.googlesource.com/open-dice/+/refs/heads/main/docs/specification.md> · TCG DICE Layering <https://trustedcomputinggroup.org/wp-content/uploads/DICE-Layering-Architecture-r19_3june2020.pdf> · DICE★ formally verified (USENIX'21) <https://www.usenix.org/system/files/sec21fall-tao.pdf> · TF-M DPE <https://trustedfirmware-m.readthedocs.io/projects/tf-m-extras/en/latest/partitions/dice_protection_environment/dice_protection_environment.html>

### 3.5 Remote attestation — RATS (RFC 9334) + EAT (RFC 9711)
3 vai: **Attester** (device, ký Evidence) → **Verifier** (server, đối chiếu Reference Values, ra Attestation Result) → **Relying Party** (quyết định tin). 2 mô hình: Passport (device lấy token trước) / Background-check (RP forward Evidence cho Verifier). **EAT** = CWT/JWT chứa claims (UEID, measurements, nonce freshness); AliasKey từ DICE ký EAT. Verifier open-source: **ARM Veraison** (Apache 2.0, hỗ trợ PSA token + CoRIM). Fleet coordinator chạy Veraison → chỉ cấp credential cho device có Evidence khớp known-good.

Nguồn: RFC 9334 <https://www.rfc-editor.org/info/rfc9334/> · RFC 9711 EAT <https://datatracker.ietf.org/doc/rfc9711/> · Veraison <https://github.com/veraison>

### 3.6 Per-Cell measurement (mở rộng measured boot vào runtime) — mô hình Linux IMA
IMA chặn mọi `execve`/`mmap(PROT_EXEC)`: hash file → append measurement list → extend PCR 10 → (tùy chọn) appraise reject nếu hash sai. EVM ký metadata chống thay binary local.

ViCell có hook đúng chỗ ([`loader.rs spawn_from_path()`](../../kernel/src/loader.rs) đọc ELF trước spawn). Thiếu (~10 dòng tại hook đã có):
```
spawn_from_path(path):
  elf = read(path)
  measurement = SHA256(elf)              # ← THIẾU
  extend_measurement_log(path, hash)     # ← THIẾU (audit.rs có event, không content hash)
  verify_ed25519(elf)                    # ← Cell signing (PLANNED, chưa làm)
  ... grant caps (manifest, §2) ...
```
Log append-only này là thứ ký vào EAT. Nguồn: Linux IMA <https://linux-ima.sourceforge.net/>

### 3.7 Sealed storage không cần TPM
Khóa AEAD dẫn xuất `HKDF(CDI_final, "sealing-key", ctx)` → bí mật tự bất khả giải nếu chuỗi boot đổi (CDI đổi). Hạn chế DICE: CDI nằm RAM khi OS chạy → kernel bị chiếm đọc được trước khi xóa. → **Giữ khóa trong Silo** (ARM64 đã có): Silo cô lập khỏi normal-world kernel ⇒ đóng lỗ "CDI-in-memory", tương đương dùng TPM làm key store.

### 3.8 Runtime attestation của Cell (Cell↔Cell, Cell↔remote)
SAS: Cell không cô lập phần cứng (LBI, không MMU) → Cell **không tự ký claim phần cứng** như SGX. Kernel là root-of-integrity. Local attestation route qua kernel:
1. Cell A xin "identity cert" qua syscall.
2. Kernel tra measurement log của A, ký `{path, elf_hash, tid, ts}` bằng key dẫn xuất CDI (giữ trong Silo).
3. Trả cert cho A; A trình cho B qua IPC; B verify bằng kernel public key (đã tin vì cùng kernel spawn).
= mô hình OP-TEE (TEE OS ký thay TA). Remote: cùng cert + DICE chain → EAT cho RP từ xa. Nguồn: SGX local attestation <https://sgx101.gitbook.io/sgx101/sgx-bootstrap/attestation/inter-process-local-attestation>

---

## 4. Phần mềm + phần cứng kết hợp: bức tranh trust phân tầng

```
L0 HW RoT     : UDS/ROTPK trong OTP (OpenTitan / board eFuse)        ← THIẾU (chưa HW plan)
L1 Secure+Measured Boot : TF-A (ARM) / U-Boot FIT (RISC-V) + DICE CDI ← THIẾU
L2 Kernel     : ViCell verify bởi bootloader; CDI_kernel             ← THIẾU
L3 Cell       : Ed25519 sign (PLANNED) + SHA256 measure (THIẾU)
                + capability grant (manifest CÓ → nâng cấp §2.4)
L4 Attest+Seal: kernel ký Cell identity cert (CDI-key trong Silo); EAT; sealed storage
                ← Silo CÓ (ARM64); phần còn lại THIẾU
L5 Remote     : EAT → fleet Verifier (Veraison) → cấp credential     ← THIẾU (server-side)
```

**Điểm nối hai luồng:** Capability (phần mềm, enforce ở syscall — **Law 1**) là **điểm enforce**; phần cứng (signing + measured + DICE) cung cấp **identity + integrity evidence** để biết *manifest có đáng tin không*. Manifest nói "cell này dùng GPIO 14-17"; chữ ký + measurement nói "và đây đúng là binary đã duyệt, chạy trên đúng device này".

---

## 5. Khuyến nghị + thứ tự ưu tiên (map vào roadmap §G)

| P | Item | Stage | Ghi chú |
|---|------|-------|---------|
| **P1** | Parameterized caps (`__ViCell_cap_args` section) | G1 | Additive, **no Law 1**; mở khóa GPIO pin / LBA range scoping ngay |
| **P2** | Spawn-time cap intersection (delegation) | G1 | 1 thay đổi kernel, không đổi ABI; diệt confused-deputy |
| **P3** | Per-Cell measurement (SHA256 + log) | G1 | Hoàn thiện cùng Cell-signing đã planned |
| **P4** | DICE layer (sw) + sealed storage qua Silo | G1/G2 | syscall 220+; mở khóa TLS key persistence + device identity |
| **P5** | Operator policy model (signed policy) | G1 | Kernel intersect manifest ∩ policy; SROS2-style |
| **P6** | Consent-broker Cell + auto-revoke | G2 HMI | Chỉ cap nhạy cảm; sau khi ViUI HMI ổn |
| **P7** | Remote attestation (EAT + Veraison) | G2 | Pure userspace trên P3-P4 |
| **HW** | OpenTitan backing cho Silo; secure boot eFuse | G2 | Không test được trên QEMU → đừng block G1 |

**Law 1:** nâng manifest (`u8`→`u16` / thêm field) đụng `libs/api/` ⇒ 2x confirmation. **Né:** dùng ELF section *mới* `__ViCell_cap_args` cho tham số (additive, backward-compat, không đụng struct ABI cũ).

**Triển khai cần plan riêng** (`/hc-plan`) vì đụng kernel + ABI + multi-phase.

---

## 6. Tham chiếu chéo
- [research-hardware-isolation.md](research-hardware-isolation.md) — cô lập bộ nhớ phần cứng (MPK/CFI/IOMMU/CHERI…); **orthogonal**: doc đó là "Cell không đọc được bộ nhớ Cell khác", doc này là "Cell chỉ làm được việc được cấp phép + chứng minh danh tính".
- [security-model.md](../security-model.md) — STRIDE, Layer 1/2/3, capability gap.
- [project-roadmap.md](../project-roadmap.md) §G — Security Platform backlog.
- [specs/01-core.md](../specs/01-core.md) — ZST capability token spec.
- Memory: `project-syscall-allowlist-and-build-pitfalls`, `project-service-id-registry`, `project-reliability-and-satp-decision` (Silo).
