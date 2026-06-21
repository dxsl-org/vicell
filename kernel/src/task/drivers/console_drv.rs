use crate::sync::Spinlock;
use alloc::collections::VecDeque;
use core::sync::atomic::Ordering;

#[allow(non_camel_case_types)]
pub struct viConsole {
    pub buffer: VecDeque<u8>,
}

impl viConsole {
    pub fn new() -> Self {
        Self {
            buffer: VecDeque::new(),
        }
    }

    /// Hard cap on buffered input bytes. A line-oriented console never needs
    /// more than this; the cap prevents a misbehaving input source (e.g. an
    /// SBI/IRQ path that returns phantom bytes every poll) from growing the
    /// VecDeque unboundedly and exhausting the kernel heap while a reader spins.
    const MAX_BUFFERED: usize = 4096;

    /// Polls input sources and pushes available characters to the buffer.
    /// Returns true if a character was received.
    pub fn poll(&mut self) -> bool {
        // Already have plenty buffered — don't poll/push more until it drains.
        if self.buffer.len() >= Self::MAX_BUFFERED {
            return false;
        }
        let mut received = false;

        // Hoist input_tid once; used for both UART relay (below) and VirtIO dispatch (§2).
        let input_tid = crate::task::drivers::virtio_input::INPUT_CELL_ID
            .load(Ordering::Relaxed);

        // 1a. Directly poll the 16550 RHR — RISC-V QEMU virt only.
        // The 16550 lives at 0x10_000_000 on RISC-V; that address is not a
        // 16550 on AArch64 (UART is PL011 at 0x0900_0000). Reading it there
        // returns garbage (0xFF), causing continuous `?` spam.
        #[cfg(any(target_arch = "riscv64", target_arch = "riscv32"))]
        while self.buffer.len() < Self::MAX_BUFFERED {
            let Some(c) = crate::task::drivers::uart::poll_rhr() else { break };
            self.buffer.push_back(c);
            if input_tid != 0 { relay_ascii_to_input(input_tid, c); }
            received = true;
        }

        // 1b. Drain any chars the UART IRQ handler buffered (when IRQs reach S-mode).
        // This path is also only relevant for RISC-V; on AArch64 IRQ-buffered
        // chars come through the PL011 path below.
        #[cfg(any(target_arch = "riscv64", target_arch = "riscv32"))]
        while self.buffer.len() < Self::MAX_BUFFERED {
            let Some(c) = crate::task::drivers::uart::getchar() else { break };
            self.buffer.push_back(c);
            if input_tid != 0 { relay_ascii_to_input(input_tid, c); }
            received = true;
        }

        // 1c. Poll PL011 UART RX on AArch64.
        // QEMU virt maps PL011 at 0x0900_0000; `-serial tcp:...` connects its
        // TX/RX to the TCP socket used by the integration-test harness.
        #[cfg(target_arch = "aarch64")]
        while self.buffer.len() < Self::MAX_BUFFERED {
            let Some(c) = crate::hal::uart_pl011::poll_rx() else { break };
            self.buffer.push_back(c);
            if input_tid != 0 { relay_ascii_to_input(input_tid, c); }
            received = true;
        }

        // 1d. Drain IRQ-filled RX buffer on x86_64.
        // vi_handle_uart_irq() (fired by IOAPIC IRQ 4 / IDT vector 0x24) pushes
        // COM1 bytes into uart::RX_BUFFER; we drain it here on every poll call
        // so the blocking file_read(fd=0) loop eventually finds a byte.
        #[cfg(any(target_arch = "x86_64", target_arch = "x86"))]
        while self.buffer.len() < Self::MAX_BUFFERED {
            let Some(c) = crate::task::drivers::uart::getchar() else { break };
            self.buffer.push_back(c);
            if input_tid != 0 { relay_ascii_to_input(input_tid, c); }
            received = true;
        }

        // NOTE: the SBI DBCN console-read fallback was removed — on this QEMU /
        // OpenSBI build it returns phantom bytes on every call, which (combined
        // with a spinning reader) grew the buffer without bound. The direct RHR
        // poll (1a) is the reliable UART input path.

        // VirtIO keyboard/mouse delivery is owned solely by
        // `virtio_input::dispatch_pending`, called from the timer tick BEFORE this
        // poll(). It drains the same event_queue with a proper SUM guard and
        // non-destructive (peek-then-pop-on-delivery) semantics. Draining it here
        // too would (a) double-consume events and (b) ipc_send into the input
        // service's U-mode buffer WITHOUT setting SUM → S-mode store page-fault
        // (scause=15) the moment an event is forwarded from the timer ISR.
        // The UART paths above use relay_ascii_to_input(), which IS SUM-safe.

        received
    }

    /// Read a byte from buffer (Non-blocking)
    pub fn read_byte(&mut self) -> Option<u8> {
        self.buffer.pop_front()
    }
}

/// Relay a UART byte to the input service as an EV_ASCII press+release pair.
///
/// Called both from the syscall path (SUM already set on RISC-V) and from the
/// timer ISR path (SUM NOT set). The RISC-V guard reads the current SUM bit and
/// only toggles it if not already set, so neither caller corrupts their SUM state.
///
/// Fire-and-forget: if the input service is not in Recv state the frames drop.
///
/// # Safety (kernel-only, Law 4)
/// Reads/writes sstatus.SUM (bit 18) on RISC-V to allow S-mode copy into the
/// U-mode IPC buffer. Safe because SUM is scoped to this function's lifetime.
fn relay_ascii_to_input(input_tid: usize, byte: u8) {
    use crate::task::drivers::input_map::WIRE_ASCII;

    // RISC-V only: SUM (sstatus bit 18) must be set for ipc_send to write into
    // the U-mode receive buffer.  Preserve the current SUM state so we do not
    // clear it mid-syscall when this is called from the file_read path.
    #[cfg(target_arch = "riscv64")]
    let sum_was_set = unsafe {
        let s: usize;
        core::arch::asm!("csrr {}, sstatus", out(reg) s);
        s & 0x4_0000 != 0
    };
    #[cfg(target_arch = "riscv64")]
    if !sum_was_set {
        // SAFETY: SUM allows S-mode access to U-mode pages; cleared on function return.
        unsafe { core::arch::asm!("csrs sstatus, {0}", in(reg) 0x4_0000usize); }
    }

    let mut msg = [0u8; 9];
    msg[0] = WIRE_ASCII;
    msg[1..5].copy_from_slice(&(byte as u32).to_le_bytes());
    msg[5..9].copy_from_slice(&1u32.to_le_bytes()); // press
    let _ = crate::task::ipc_send(0, input_tid, msg.as_ptr() as usize, 9);
    msg[5..9].copy_from_slice(&0u32.to_le_bytes()); // release
    let _ = crate::task::ipc_send(0, input_tid, msg.as_ptr() as usize, 9);

    #[cfg(target_arch = "riscv64")]
    if !sum_was_set {
        // SAFETY: restore SUM to its pre-call value.
        unsafe { core::arch::asm!("csrc sstatus, {0}", in(reg) 0x4_0000usize); }
    }
}

pub static CONSOLE: Spinlock<viConsole> = Spinlock::new(viConsole {
    buffer: VecDeque::new(),
});

pub fn init() {
    // Nothing special to init for SBI Console so far
    // But we might want to clear buffer
    let mut cons = CONSOLE.lock();
    cons.buffer.clear();
    log::info!("Console: Input Driver Initialized.");
}
