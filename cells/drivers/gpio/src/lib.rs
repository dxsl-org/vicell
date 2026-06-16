#![no_std]
#![forbid(unsafe_code)]

//! GPIO Driver Cell — ARM PL061 on QEMU ARM virt.
//!
//! Base address: 0x0903_0000 (ARM virt machine, 4 KiB region).
//!
//! PL061 GPIODATA uses address-masked access:
//!   - Read all 8 pins:  offset 0x3FC (mask = 0xFF in addr[9:2])
//!   - Write pin N:      offset = 1 << (N+2), value = bit_N_high_or_zero
//! GPIODIR:             offset 0x400 (bit = 1 → output)
//! Interrupt registers: 0x404–0x41C (deferred to v2)

extern crate alloc;

use hal_gpio::{Edge, PinDir, ViGpio};
use ostd::mmio::{request_region, MmioRegion};
use types::{ViError, ViResult};

/// QEMU ARM virt PL061 MMIO base and size.
pub const PL061_BASE: usize = 0x0903_0000;
pub const PL061_SIZE: usize = 0x1000;

// Register offsets
const GPIODATA_ALL: usize = 0x3FC; // read all 8 pins
const GPIODIR: usize = 0x400; // direction: 1 = output
const GPIOIE: usize = 0x410; // interrupt enable
const GPIOIS: usize = 0x404; // interrupt sense: 0=edge, 1=level
const GPIOIBE: usize = 0x408; // both edges
const GPIOIEV: usize = 0x40C; // event: 0=falling, 1=rising
const GPIOIC: usize = 0x41C;  // interrupt clear (write 1 to clear)
const GPIOMIS: usize = 0x418; // masked interrupt status (read-only)

/// PL061 GPIO controller for QEMU ARM virt.
pub struct Pl061Gpio {
    mmio: MmioRegion,
}

impl Pl061Gpio {
    /// Acquire exclusive GPIO MMIO access from the kernel.
    ///
    /// Fails if the manifest did not declare `gpio = true`, or if another
    /// Cell already owns the PL061 range.
    pub fn open() -> ViResult<Self> {
        let mmio = request_region(PL061_BASE, PL061_SIZE)?;
        Ok(Self { mmio })
    }

    // ---

    /// Read the raw 8-bit GPIODATA (all pins).
    fn read_all(&self) -> ViResult<u8> {
        let v = self.mmio.read_u32(GPIODATA_ALL)?;
        Ok(v as u8)
    }

    /// Write a masked subset of GPIODATA.
    ///
    /// `pin_mask`: which pins to affect (addr bits [9:2])
    /// `value`:    new values for those pins (data bits)
    fn write_masked(&self, pin: u8, high: bool) -> ViResult<()> {
        // Address-masked write: offset encodes which pin(s) to affect
        let offset = (1usize << (pin + 2)) & 0x3FF; // addr bits [9:2]
        let data: u32 = if high { 1u32 << pin } else { 0 };
        self.mmio.write_u32(offset, data)
    }

    /// Return the masked interrupt status register (GPIOMIS).
    ///
    /// Each set bit indicates a pin whose edge IRQ fired AND was enabled.
    /// Call this after receiving a `GPIO_IRQ_NOTIFY` (opcode 0xA0) from the kernel
    /// to discover which pins triggered, then call `clear_irq(mask)` to ACK them.
    pub fn read_mis(&self) -> ViResult<u8> {
        Ok(self.mmio.read_u32(GPIOMIS)? as u8)
    }

    /// Acknowledge (clear) GPIO interrupts for the pins in `mask`.
    ///
    /// Writes GPIOIC without touching GPIOIE, so the interrupt remains enabled
    /// for the next edge.  Call after reading GPIOMIS and processing all set bits.
    pub fn clear_irq(&mut self, mask: u8) -> ViResult<()> {
        self.mmio.write_u32(GPIOIC, u32::from(mask))
    }
}

impl ViGpio for Pl061Gpio {
    fn set_direction(&mut self, pin: u8, dir: PinDir) -> ViResult<()> {
        if pin >= 8 {
            return Err(ViError::InvalidInput);
        }
        let current = self.mmio.read_u32(GPIODIR)? as u8;
        let new_val: u32 = match dir {
            PinDir::Output => (current | (1 << pin)) as u32,
            PinDir::Input  => (current & !(1 << pin)) as u32,
        };
        self.mmio.write_u32(GPIODIR, new_val)
    }

    fn read_pin(&self, pin: u8) -> ViResult<bool> {
        if pin >= 8 {
            return Err(ViError::InvalidInput);
        }
        let all = self.read_all()?;
        Ok(all & (1 << pin) != 0)
    }

    fn write_pin(&mut self, pin: u8, high: bool) -> ViResult<()> {
        if pin >= 8 {
            return Err(ViError::InvalidInput);
        }
        // Verify pin is configured as output
        let dir = self.mmio.read_u32(GPIODIR)? as u8;
        if dir & (1 << pin) == 0 {
            return Err(ViError::InvalidInput); // not an output
        }
        self.write_masked(pin, high)
    }

    fn enable_edge_irq(&mut self, pin: u8, edge: Edge) -> ViResult<()> {
        if pin >= 8 {
            return Err(ViError::InvalidInput);
        }
        let mask = 1u32 << pin;

        // Edge-triggered (not level): GPIOIS bit = 0
        let is_val = self.mmio.read_u32(GPIOIS)? & !mask;
        self.mmio.write_u32(GPIOIS, is_val)?;

        match edge {
            Edge::Both => {
                // Both edges: GPIOIBE bit = 1 (overrides GPIOIEV)
                let ibe = self.mmio.read_u32(GPIOIBE)? | mask;
                self.mmio.write_u32(GPIOIBE, ibe)?;
            }
            Edge::Rising | Edge::Falling => {
                // Single edge: GPIOIBE bit = 0, GPIOIEV selects polarity
                let ibe = self.mmio.read_u32(GPIOIBE)? & !mask;
                self.mmio.write_u32(GPIOIBE, ibe)?;
                let iev = self.mmio.read_u32(GPIOIEV)?;
                let iev_new = if matches!(edge, Edge::Rising) { iev | mask } else { iev & !mask };
                self.mmio.write_u32(GPIOIEV, iev_new)?;
            }
        }

        // Enable interrupt for this pin
        let ie = self.mmio.read_u32(GPIOIE)? | mask;
        self.mmio.write_u32(GPIOIE, ie)
    }

    fn disable_irq(&mut self, pin: u8) -> ViResult<()> {
        if pin >= 8 {
            return Err(ViError::InvalidInput);
        }
        // Clear interrupt enable and pending interrupt for this pin
        let ie = self.mmio.read_u32(GPIOIE)? & !(1u32 << pin);
        self.mmio.write_u32(GPIOIE, ie)?;
        self.mmio.write_u32(GPIOIC, 1u32 << pin)
    }
}
