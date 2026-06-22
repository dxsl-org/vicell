"""Write all x86_64 HAL sub-modules for Cellos Phase 09."""
import os

BASE = "d:/Cellos/hal/arch/x86/src/x86_64"
os.makedirs(BASE, exist_ok=True)

# ── uart_16550.rs ─────────────────────────────────────────────────────────────
UART = """//! 16550A UART driver via x86 port I/O. COM1 base: 0x3F8.
const COM1: u16 = 0x3F8;
#[inline]
fn outb(port: u16, val: u8) {
    // SAFETY: port I/O on COM1 does not affect memory safety.
    unsafe { core::arch::asm!("out dx, al", in("dx") port, in("al") val, options(nomem, nostack)); }
}
#[inline]
fn inb(port: u16) -> u8 {
    let val: u8;
    // SAFETY: reading port I/O does not affect memory safety.
    unsafe { core::arch::asm!("in al, dx", in("dx") port, out("al") val, options(nomem, nostack)); }
    val
}
/// Initialise COM1 at 115200 8N1.
pub fn init() {
    outb(COM1 + 1, 0x00); // Disable IRQs
    outb(COM1 + 3, 0x80); // DLAB = 1
    outb(COM1 + 0, 0x01); // Divisor low  (115200 baud)
    outb(COM1 + 1, 0x00); // Divisor high
    outb(COM1 + 3, 0x03); // 8N1
    outb(COM1 + 2, 0xC7); // FIFO, 14-byte threshold
    outb(COM1 + 4, 0x0B); // IRQs enabled, RTS/DSR
}
/// Write one byte, blocking on TX hold register empty.
pub fn putchar(byte: u8) {
    while inb(COM1 + 5) & 0x20 == 0 { core::hint::spin_loop(); }
    outb(COM1, byte);
}
/// Write string, converting `\\n` to `\\r\\n`.
pub fn puts(s: &str) {
    for b in s.bytes() {
        if b == b'\\n' { putchar(b'\\r'); }
        putchar(b);
    }
}
"""

# ── gdt.rs ────────────────────────────────────────────────────────────────────
GDT = """//! x86_64 GDT + TSS. Selectors: null=0 kCS=0x08 kDS=0x10 uDS=0x18 uCS=0x20 TSS=0x28.
use core::arch::asm;

/// Minimal TSS storing only the kernel-stack pointer (RSP0).
#[repr(C, packed)]
pub struct Tss {
    _r0: u32,
    pub rsp0: u64,
    _rest: [u8; 84],
}
impl Tss { pub const fn new() -> Self { Self { _r0: 0, rsp0: 0, _rest: [0; 84] } } }

#[repr(transparent)]
#[derive(Copy, Clone)]
struct GdtEntry(u64);
impl GdtEntry {
    const NULL: Self = Self(0);
    const fn code(dpl: u8) -> Self {
        Self((1u64<<43)|(1<<44)|((dpl as u64)<<45)|(1<<47)|(1<<53))
    }
    const fn data(dpl: u8) -> Self {
        Self((1u64<<41)|(1<<44)|((dpl as u64)<<45)|(1<<47))
    }
    fn tss_low(base: u64, limit: u32) -> Self {
        let b = ((base&0xFF)<<16)|((base>>8&0xFF)<<24)|((base>>16&0xFF)<<32)|((base>>24&0xFF)<<56);
        let l = (limit as u64 & 0xFFFF) | ((limit as u64 >>16)<<48);
        Self(l|b|(0x9u64<<40)|(1<<47))
    }
    fn tss_high(base: u64) -> Self { Self((base>>32)&0xFFFF_FFFF) }
}

#[repr(C, align(16))]
struct Gdt { entries: [GdtEntry; 8] }
#[repr(C, packed)]
struct GdtPtr { limit: u16, base: u64 }

static mut GDT: Gdt = Gdt { entries: [GdtEntry::NULL; 8] };
pub static mut TSS: Tss = Tss::new();

/// Build and install the GDT + TSS.
pub fn init() {
    // SAFETY: single-threaded boot; GDT and TSS are static globals.
    unsafe {
        GDT.entries[1] = GdtEntry::code(0);
        GDT.entries[2] = GdtEntry::data(0);
        GDT.entries[3] = GdtEntry::data(3);
        GDT.entries[4] = GdtEntry::code(3);
        let b = &TSS as *const _ as u64;
        let l = (core::mem::size_of::<Tss>()-1) as u32;
        GDT.entries[5] = GdtEntry::tss_low(b, l);
        GDT.entries[6] = GdtEntry::tss_high(b);
        let ptr = GdtPtr { limit: (core::mem::size_of::<Gdt>()-1) as u16, base: GDT.entries.as_ptr() as u64 };
        asm!(
            // SAFETY: GDT pointer is valid; lgdt + far jmp reload CS; ltr loads TSS.
            "lgdt [{p}]",
            "push 0x08",
            "lea {t}, [rip+1f]",
            "push {t}",
            "retfq",
            "1:",
            "mov ax, 0x10",
            "mov ds, ax", "mov es, ax", "mov ss, ax",
            "mov ax, 0x28", "ltr ax",
            p = in(reg) &ptr, t = lateout(reg) _, options(att_syntax)
        );
    }
}
/// Set RSP0 (kernel stack for Ring3->Ring0 transition).
pub fn set_kernel_stack(sp: u64) {
    // SAFETY: TSS is static; single-threaded spawn path.
    unsafe { TSS.rsp0 = sp; }
}
"""

# ── idt.rs ────────────────────────────────────────────────────────────────────
IDT = """//! x86_64 Interrupt Descriptor Table (256 16-byte long-mode gates).
use core::arch::asm;

#[repr(C)]
#[derive(Copy, Clone)]
struct IdtEntry {
    off_lo: u16, sel: u16, ist: u8, attr: u8, off_mid: u16, off_hi: u32, _res: u32,
}
impl IdtEntry {
    fn new(handler: u64, dpl: u8) -> Self {
        Self {
            off_lo:  (handler & 0xFFFF) as u16,
            sel:     0x08,
            ist:     0,
            attr:    0x8E | ((dpl & 3) << 5),
            off_mid: ((handler >> 16) & 0xFFFF) as u16,
            off_hi:  ((handler >> 32) & 0xFFFF_FFFF) as u32,
            _res:    0,
        }
    }
}
#[repr(C, align(16))]
struct Idt { e: [IdtEntry; 256] }
#[repr(C, packed)]
struct IdtPtr { limit: u16, base: u64 }

static mut IDT: Idt = Idt { e: [IdtEntry { off_lo:0, sel:0, ist:0, attr:0, off_mid:0, off_hi:0, _res:0 }; 256] };

pub fn init() {
    // SAFETY: single-threaded boot; IDT is a static global.
    unsafe {
        for e in IDT.e.iter_mut() { *e = IdtEntry::new(x86_64_interrupt_handler as u64, 0); }
        IDT.e[0x80] = IdtEntry::new(x86_64_interrupt_handler as u64, 3);
        let ptr = IdtPtr { limit: (core::mem::size_of::<Idt>()-1) as u16, base: IDT.e.as_ptr() as u64 };
        // SAFETY: ptr is valid; lidt is safe from Ring 0.
        asm!("lidt [{p}]", p = in(reg) &ptr, options(nomem, nostack));
    }
}

/// Common interrupt handler (placeholder; all 256 vectors share this target).
#[no_mangle]
pub extern "C" fn x86_64_interrupt_handler(vec: u64, error_code: u64, rip: u64) {
    match vec {
        0x20 => { super::apic::eoi(); } // LAPIC timer
        0x0D => panic!("[x86_64] #GP rip=0x{:X} ec=0x{:X}", rip, error_code),
        0x0E => {
            let cr2: u64;
            // SAFETY: reading CR2 (page-fault address) does not modify state.
            unsafe { asm!("mov {}, cr2", out(reg) cr2, options(nomem, nostack)); }
            panic!("[x86_64] #PF rip=0x{:X} cr2=0x{:X} ec=0x{:X}", rip, cr2, error_code);
        }
        _ => { if vec >= 0x20 { super::apic::eoi(); } }
    }
}
"""

# ── context.rs ────────────────────────────────────────────────────────────────
CTX = """//! x86_64 CPU context (callee-saved registers + RSP for cooperative switch).
use core::arch::asm;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct CpuContext {
    pub r15: u64, pub r14: u64, pub r13: u64, pub r12: u64,
    pub rbx: u64, pub rbp: u64, pub rsp: u64, pub rip: u64,
}

/// Cooperative context switch.
///
/// # Safety
/// Both pointers must point to valid, aligned `CpuContext` structs.
pub unsafe fn switch(old: *mut CpuContext, new: *const CpuContext) {
    // SAFETY: caller guarantees valid, aligned CpuContext pointers.
    unsafe {
        asm!(
            "mov [{o}+0*8], r15",  "mov [{o}+1*8], r14",
            "mov [{o}+2*8], r13",  "mov [{o}+3*8], r12",
            "mov [{o}+4*8], rbx",  "mov [{o}+5*8], rbp",
            "mov [{o}+6*8], rsp",
            "lea rax, [rip+1f]",   "mov [{o}+7*8], rax",
            "mov r15, [{n}+0*8]",  "mov r14, [{n}+1*8]",
            "mov r13, [{n}+2*8]",  "mov r12, [{n}+3*8]",
            "mov rbx, [{n}+4*8]",  "mov rbp, [{n}+5*8]",
            "mov rsp, [{n}+6*8]",
            "jmp [{n}+7*8]",
            "1:",
            o = in(reg) old, n = in(reg) new, out("rax") _,
            options(att_syntax),
        );
    }
}
"""

# ── paging.rs ─────────────────────────────────────────────────────────────────
PAGING = """//! x86_64 4-level page table (PML4->PDPT->PD->PT), 4KB and 2MB pages.
use hal_paging::{PageFlags, PageTableTrait};
use types::*;
use core::arch::asm;

pub const PAGE_SIZE: usize = 4096;

const PTE_P:  u64 = 1<<0;
const PTE_RW: u64 = 1<<1;
const PTE_US: u64 = 1<<2;
const PTE_PS: u64 = 1<<7;
const PTE_NX: u64 = 1<<63;

#[repr(C, align(4096))]
pub struct PageTable { entries: [u64; 512] }
impl PageTable { pub const fn zero() -> Self { Self { entries: [0u64; 512] } } }

impl PageTableTrait for PageTable {
    fn init(&mut self) -> ViResult<PhysAddr> {
        self.entries = [0u64; 512];
        Ok(self as *mut _ as PhysAddr)
    }
    fn map(&mut self, virt: VAddr, phys: PhysAddr, flags: PageFlags,
           alloc_fn: &mut dyn FnMut() -> Option<PhysAddr>) -> ViResult<()> {
        let i3 = (virt>>39)&0x1FF; let i2=(virt>>30)&0x1FF;
        let i1 = (virt>>21)&0x1FF; let i0=(virt>>12)&0x1FF;
        let pdpt = self.get_or_alloc(i3, alloc_fn)?;
        let pd   = pdpt.get_or_alloc(i2, alloc_fn)?;
        let pt   = pd.get_or_alloc(i1, alloc_fn)?;
        let mut e = phys as u64 | PTE_P;
        if flags.bits()&PageFlags::WRITE   !=0 { e|=PTE_RW; }
        if flags.bits()&PageFlags::USER    !=0 { e|=PTE_US; }
        if flags.bits()&PageFlags::EXECUTE ==0 { e|=PTE_NX; }
        pt.entries[i0] = e;
        Ok(())
    }
    fn unmap(&mut self, virt: VAddr) -> ViResult<()> {
        let e0=self.entries[(virt>>39)&0x1FF];
        if e0&PTE_P==0 { return Err(ViError::NotFound); }
        let pdpt: &mut PageTable = unsafe { &mut *((e0&!0xFFF) as *mut PageTable) };
        let e1=pdpt.entries[(virt>>30)&0x1FF];
        if e1&PTE_P==0 { return Err(ViError::NotFound); }
        let pd: &mut PageTable = unsafe { &mut *((e1&!0xFFF) as *mut PageTable) };
        let e2=pd.entries[(virt>>21)&0x1FF];
        if e2&PTE_P==0 { return Err(ViError::NotFound); }
        let pt: &mut PageTable = unsafe { &mut *((e2&!0xFFF) as *mut PageTable) };
        pt.entries[(virt>>12)&0x1FF] = 0;
        // SAFETY: invlpg flushes only the one virtual address from the TLB.
        unsafe { asm!("invlpg [{v}]", v=in(reg) virt, options(nomem)); }
        Ok(())
    }
    fn translate(&self, virt: VAddr) -> Option<PhysAddr> {
        let e0=self.entries[(virt>>39)&0x1FF]; if e0&PTE_P==0 {return None;}
        let pdpt: &PageTable = unsafe { &*((e0&!0xFFF) as *const PageTable) };
        let e1=pdpt.entries[(virt>>30)&0x1FF]; if e1&PTE_P==0 {return None;}
        let pd: &PageTable = unsafe { &*((e1&!0xFFF) as *const PageTable) };
        let e2=pd.entries[(virt>>21)&0x1FF]; if e2&PTE_P==0 {return None;}
        if e2&PTE_PS!=0 { return Some(((e2&!0x1F_FFFF)+(virt&0x1F_FFFF) as u64) as PhysAddr); }
        let pt: &PageTable = unsafe { &*((e2&!0xFFF) as *const PageTable) };
        let e3=pt.entries[(virt>>12)&0x1FF]; if e3&PTE_P==0 {return None;}
        Some(((e3&!0xFFF)+(virt&0xFFF) as u64) as PhysAddr)
    }
    unsafe fn activate(&self) {
        let cr3 = self as *const _ as u64;
        // SAFETY: CR3 write activates new PML4; caller ensures identity mapping covers
        // the instruction pointer so execution continues after the write.
        unsafe { asm!("mov cr3, {}", in(reg) cr3, options(nomem, nostack)); }
    }
}

impl PageTable {
    fn get_or_alloc(&mut self, idx: usize, alloc_fn: &mut dyn FnMut()->Option<PhysAddr>)
        -> ViResult<&mut PageTable> {
        if self.entries[idx]&PTE_P==0 {
            let f = alloc_fn().ok_or(ViError::OutOfMemory)?;
            // SAFETY: f is a freshly-allocated 4KB frame; identity-mapped pre-paging.
            unsafe { core::ptr::write_bytes(f as *mut u8, 0, PAGE_SIZE) };
            self.entries[idx] = f as u64 | PTE_P | PTE_RW;
        }
        let next = (self.entries[idx]&!0xFFF) as PhysAddr;
        // SAFETY: identity-mapped; next is a valid page table frame.
        Ok(unsafe { &mut *(next as *mut PageTable) })
    }
}
"""

# ── apic.rs ───────────────────────────────────────────────────────────────────
APIC = """//! Local APIC (0xFEE0_0000) and I/O APIC (0xFEC0_0000) MMIO drivers.
const LAPIC:  usize = 0xFEE0_0000;
const IOAPIC: usize = 0xFEC0_0000;

fn lw(reg: usize, v: u32) {
    // SAFETY: LAPIC MMIO is identity-mapped; write does not affect memory safety.
    unsafe { core::ptr::write_volatile((LAPIC + reg) as *mut u32, v); }
}
fn iow(idx: u8, v: u32) {
    // SAFETY: IOAPIC MMIO is identity-mapped.
    unsafe {
        core::ptr::write_volatile(IOAPIC as *mut u32, idx as u32);
        core::ptr::write_volatile((IOAPIC + 0x10) as *mut u32, v);
    }
}

/// Initialise LAPIC and configure periodic timer at ~100 Hz (vector 0x20).
pub fn init_lapic() {
    lw(0x0F0, 0x1FF);          // SVR: enable LAPIC, spurious vector 0xFF
    lw(0x3E0, 0x3);            // Timer divide-by-16
    lw(0x320, 0x20 | (1<<17)); // LVT_TIMER: periodic mode, vector 0x20
    lw(0x380, 1_000_000 / 16); // Initial count (~100 Hz at 1 GHz LAPIC clock)
}

/// Signal End-of-Interrupt to the LAPIC.
pub fn eoi() {
    lw(0x0B0, 0);
}

/// Redirect IOAPIC IRQ to IDT vector on CPU 0 (edge-triggered, active-high).
pub fn ioapic_redirect(irq: u8, vec: u8) {
    iow(0x10 + irq * 2 + 1, 0);       // destination: CPU 0
    iow(0x10 + irq * 2,     vec as u32); // vector, unmasked
}
"""

# ── timer.rs ──────────────────────────────────────────────────────────────────
TIMER = """//! LAPIC periodic timer wrapper (delegates to apic::init_lapic).
pub fn init() { super::apic::init_lapic(); }
/// No-op: LAPIC periodic timer reloads automatically.
pub fn reset() {}
"""

# ── syscall.rs ────────────────────────────────────────────────────────────────
SYSCALL = """//! x86_64 SYSCALL/SYSRET MSR configuration.
//! EFER.SCE=1, STAR (segment selectors), LSTAR (entry point), FMASK.
use core::arch::asm;

const IA32_EFER:  u32 = 0xC000_0080;
const IA32_STAR:  u32 = 0xC000_0081;
const IA32_LSTAR: u32 = 0xC000_0082;
const IA32_FMASK: u32 = 0xC000_0084;

fn rdmsr(msr: u32) -> u64 {
    let lo:u32; let hi:u32;
    // SAFETY: rdmsr from Ring 0 does not affect memory safety.
    unsafe { asm!("rdmsr", in("ecx") msr, out("eax") lo, out("edx") hi, options(nomem,nostack)); }
    (hi as u64)<<32 | lo as u64
}
fn wrmsr(msr: u32, val: u64) {
    let lo=val as u32; let hi=(val>>32) as u32;
    // SAFETY: wrmsr to a valid MSR from Ring 0 does not affect memory safety.
    unsafe { asm!("wrmsr", in("ecx") msr, in("eax") lo, in("edx") hi, options(nomem,nostack)); }
}

/// Initialise SYSCALL/SYSRET path.
pub fn init() {
    wrmsr(IA32_EFER, rdmsr(IA32_EFER)|1); // SCE=1
    // STAR: user CS=0x20 (sysret CS=0x23, SS=0x23+8=0x2B=uDS),
    //       kernel CS=0x08 (syscall CS=0x08, SS=0x10=kDS)
    wrmsr(IA32_STAR, (0x0020_u64<<48)|(0x0008_u64<<32));
    extern "C" { fn syscall_entry(); }
    wrmsr(IA32_LSTAR, syscall_entry as u64);
    wrmsr(IA32_FMASK, 0x0300); // clear IF + DF on syscall entry
}

/// Placeholder Rust syscall dispatcher (called from syscall_entry asm).
#[no_mangle]
pub extern "C" fn x86_64_syscall_dispatch() {
    // TODO: forward to the kernel syscall table.
    // Returns usize::MAX (ENOSYS) in RAX via the asm wrapper.
}
"""

data = {
    "uart_16550.rs": UART,
    "gdt.rs": GDT,
    "idt.rs": IDT,
    "context.rs": CTX,
    "paging.rs": PAGING,
    "apic.rs": APIC,
    "timer.rs": TIMER,
    "syscall.rs": SYSCALL,
}

for name, content in data.items():
    path = f"{BASE}/{name}"
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(content)
    print(f"  wrote {name}")

print("Done.")
