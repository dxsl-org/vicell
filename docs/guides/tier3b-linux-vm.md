# Tier 3b Linux VM — Full Kernel Guest

> Run unmodified Linux binaries in a hypervisor-isolated VM. For legacy code, fork-heavy apps, or untrusted workloads.

---

## Overview

Tier 3b lets you run a full Linux kernel (e.g., Alpine, Busybox) inside a lightweight hypervisor. From the app's perspective, it's a normal Linux environment:

- Standard libc (musl, glibc)
- Full POSIX (fork, mmap, signals, pthreads)
- Package manager (`apk install`, `apt`)
- Any unmodified Linux binary

**Trade-off**: 10–15% performance overhead vs Tier 1; 2–10 second boot time.

---

## Platform Support

| Platform | Status | Hypervisor | Notes |
|----------|--------|-----------|-------|
| **ARM64** | ✅ Working (G2) | EL2 (Secure/Normal mode split) | RK3588, Cortex-A72+; boots Alpine |
| **x86_64** | ✅ Working (G2) | VMX (Intel) / SVM (AMD) | QEMU q35 with KVM; boots Alpine |
| **RISC-V** | ❌ Not implemented | H-ext (too new) | Deferred beyond G1 |

**G2-only**: requires real hardware or advanced QEMU (not basic RISC-V).

---

## Architecture

```
┌─────────────────────────────────┐
│ ViCell Kernel (S-mode / VMX host)
│                                 │
│  ┌──────────────────────────┐   │
│  │ Hypervisor (custom ~9K LOC)
│  │                          │   │  Trap device MMIO
│  │  ┌────────────────────┐  │   │  Emulate PL011, clint, etc.
│  │  │ Linux Guest (HS-mode / VM) │
│  │  │  /bin/app          │  │   │
│  │  │  fork() / mmap()   │  │   │
│  │  └────────────────────┘  │   │
│  └──────────────────────────┘   │
│                                 │
│  VirtIO devices:                │
│    disk  → ViCell VFS           │
│    net   → ViCell Net           │
│    console → kernel log         │
└─────────────────────────────────┘
```

---

## Running a Linux VM

### Create a VM

```bash
# Start shell and ask for a Linux VM
vm_id = sys_create_vm(4, 0x4000000)
    # args: mode (4=ARM64 HS-mode), mem (64 MiB)
    # → vm_id (u64)

# Load Linux kernel ELF
sys_vm_load_elf(vm_id, kernel_elf_data)

# Boot it
sys_vm_run(vm_id)
    # Blocks until VM exits or you call sys_vm_exit()
```

### From Shell

The shell has built-in hypervisor commands (planned):

```bash
vm create --arch arm64 --mem 64M --kernel /vmlinuz
vm run <vm_id>
vm exit <vm_id>
```

---

## Guest Filesystem Access

The guest mounts ViCell's VFS as a VirtIO block device. By default, the root filesystem is read-only FAT32 (from kernel build). You can:

1. **Create an overlay** (writable tmpfs on top)
2. **Mount a writable partition** (future: FAT32 RW)
3. **Write to /tmp** (ramdisk, shared with ViCell)

---

## Guest Network Access

The hypervisor exposes a VirtIO net device. Guest sees a standard Linux NIC:

```bash
# Inside guest
ip addr show
eth0: inet 10.0.2.15

# Connect to host services (ViCell net cell runs at 10.0.2.2)
curl -v http://10.0.2.2:8080/

# Or use sockets normally
```

Network traffic is routed through ViCell's kernel; no direct hardware access.

---

## VirtIO Devices (What's Emulated)

| Device | Status | Notes |
|--------|--------|-------|
| Block (disk) | ✅ | Read-only FAT32 (kernel.img); no write yet |
| Network | ✅ | Full NIC; routed via ViCell net cell |
| Console | ✅ | Serial output to kernel log |
| Entropy (RNG) | ✅ | `/dev/urandom` works |
| Clock (MMIO) | ✅ | `clock_gettime()` accurate |

---

## Example: Boot Alpine Linux

```bash
# Prerequisites
scripts/build-kernel-alpine.sh  # one-time, downloads/builds Alpine rootfs

# Start ViCell
./run-arm64.ps1

# From shell
vm create --arch arm64 --mem 64M --rootfs /alpine.squashfs
vm run 1
    # Alpine login prompt appears
login: root
```

Inside the VM, you have a full Linux shell:

```bash
# Install packages
apk update
apk add curl vim

# Run C++ code
apk add g++ make
g++ -o myapp main.cpp
./myapp

# Fork works!
for i in {1..10}; do (sleep 1 & echo "background job $i") done

# exit to return to ViCell shell
exit
```

---

## Performance Characteristics

| Operation | Tier 1 Rust | Tier 1 + SDK | Tier 3b Linux |
|-----------|-------------|--------------|--------------|
| Syscall latency | ~1 μs | ~2 μs (IPC) | ~10–20 μs (trap) |
| App startup | <1 ms | <1 ms | 2–5 s (kernel boot) |
| I/O throughput | Native | ~90% native | ~80% native (VirtIO) |
| Memory overhead | ~10 KiB | ~50 KiB | ~64 MiB (guest kernel) |

**Use Tier 3b when**: boot time and startup latency don't matter, but compatibility and ease-of-deployment do.

---

## Limits & Constraints

❌ **No nested VMs** — guest cannot create sub-VMs.  
❌ **No direct hardware access** — I/O goes through ViCell drivers.  
❌ **No DMA to host memory** — disk/network buffers are copied.  
⚠️ **Slow boot** — 2–10 seconds for full Linux init.  
✅ **Full fork() / pthreads** — anything Unix-like works.  
✅ **Package managers** — `apk` / `apt` work in writable layers.  

---

## Hypervisor Internals (Advanced)

The hypervisor is a custom minimal VMM (~9K lines of Rust), not a fork of Crosvm or KVM. It:

1. **Boots the guest** — loads ELF, sets up page tables, enters guest mode
2. **Emulates MMIO** — traps device accesses (PL011 UART, CLINT timer, etc.)
3. **Passes through VirtIO** — disk/net queues bypass emulation (direct DMA)
4. **Isolates faults** — guest page faults, invalid instructions trapped; host continues

For details, see [system-architecture.md](../system-architecture.md) § Tier 3 Hypervisor.

---

## Building a Custom Alpine Rootfs

```bash
cd scripts
./build-kernel-alpine.sh  # ~30 min, downloads+cross-compiles

# Output: alpine.img (FAT32 with /bin, /etc, /lib, /usr)
# Loaded as VirtIO block device by hypervisor
```

---

## When to Use Tier 3b

✅ Existing Linux C/C++ code (no rewrite)  
✅ Apps that fork() heavily (e.g., nginx, Java)  
✅ Package managers essential (`apt install opencv`)  
✅ Untrusted code (isolated in VM)  
✅ Learning Linux internals without rewriting  

❌ Performance-critical (use Tier 1 Rust)  
❌ Real-time (VM jitter unacceptable)  
❌ Embedded systems with 4 MiB RAM (VM needs 64+ MiB)  
❌ RISC-V (not implemented yet)  

---

## Canonical Example

See [cells/guests/silo-guest/](../../cells/guests/silo-guest/) — the Silo guest firmware is also a micro-VM example (much smaller, ~5 KiB).

For a full Alpine Linux VM, see kernel build logs (`scripts/build-kernel-alpine.sh` output).

---

## Troubleshooting

**VM boot hangs?**  
→ Check guest ELF load address matches hypervisor's page table setup. Kernel messages usually print; check serial output.

**Disk writes don't persist?**  
→ Rootfs is read-only FAT32 (mounted via VirtIO). Write to `/tmp` (tmpfs) or request writable partition.

**Network unreachable?**  
→ ViCell net cell may not be running. Check `net-tools` in `/bin/`. Guest IP should be 10.0.2.15, ViCell host at 10.0.2.2.

**Slow network?**  
→ VirtIO performance is ~90% native on QEMU. Real hardware faster. No tuning levers exposed yet.

---

## Next Steps

- See [system-architecture.md](../system-architecture.md) § Tier 3 for hypervisor design.
- For ARM64 EL2 MMU setup: see kernel/arch/arm64/ (Stage-2 paging).
- For x86 VMX: see kernel/arch/x86_64/ (EPT).
- Build Alpine: `scripts/build-kernel-alpine.sh`.
