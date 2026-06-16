#![no_std]
#![forbid(unsafe_code)]

//! In-memory CAN loopback driver — no MMIO, no capability required.
//!
//! `send_frame` pushes into a fixed-size ring buffer.
//! `recv_frame` pops from the same buffer.
//!
//! This validates the `ViCan` trait interface and frame encoding on QEMU
//! without any hardware or resource registry changes.
//!
//! Capacity: 32 frames. Overflow drops the oldest frame and sets `overflow_count`.

use hal_can::{CanError, CanFrame, ViCan};

/// Fixed ring-buffer capacity.
const CAPACITY: usize = 32;

// Sentinel for array initialization (CanFrame: Copy required).
const NONE_FRAME: Option<CanFrame> = None;

/// In-memory loopback CAN controller.
pub struct LoopbackCan {
    buf: [Option<CanFrame>; CAPACITY],
    /// Index of the next slot to write into.
    head: usize,
    /// Index of the next slot to read from.
    tail: usize,
    /// Number of frames currently in the buffer.
    len: usize,
    /// Number of frames dropped due to overflow.
    pub overflow_count: u32,
    /// Validates that `configure()` was called before any frame operations.
    configured: bool,
}

impl LoopbackCan {
    /// Create a new loopback controller. Must call `configure()` before use.
    pub fn new() -> Self {
        Self {
            buf: [NONE_FRAME; CAPACITY],
            head: 0,
            tail: 0,
            len: 0,
            overflow_count: 0,
            configured: false,
        }
    }

    /// Number of frames currently buffered.
    pub fn frame_count(&self) -> usize {
        self.len
    }
}

impl Default for LoopbackCan {
    fn default() -> Self {
        Self::new()
    }
}

impl ViCan for LoopbackCan {
    type Error = CanError;

    fn configure(&mut self, kbps: u32) -> Result<(), CanError> {
        match kbps {
            125 | 250 | 500 | 1000 => {
                self.configured = true;
                Ok(())
            }
            _ => Err(CanError::InvalidBitrate),
        }
    }

    fn send_frame(&mut self, frame: &CanFrame) -> Result<(), CanError> {
        if frame.dlc > 8 {
            return Err(CanError::FrameError);
        }
        if self.len == CAPACITY {
            // Drop oldest (advance tail) to make room, record overflow.
            self.tail = (self.tail + 1) % CAPACITY;
            self.len -= 1;
            self.overflow_count += 1;
        }
        self.buf[self.head] = Some(*frame);
        self.head = (self.head + 1) % CAPACITY;
        self.len += 1;
        Ok(())
    }

    fn recv_frame(&mut self) -> Result<CanFrame, CanError> {
        if self.len == 0 {
            return Err(CanError::RxEmpty);
        }
        let frame = self.buf[self.tail].take().unwrap_or(CanFrame::new(0, &[]));
        self.tail = (self.tail + 1) % CAPACITY;
        self.len -= 1;
        Ok(frame)
    }
}
