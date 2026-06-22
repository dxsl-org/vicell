// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#![no_std]

// 1. Re-export các định nghĩa cốt lõi từ thư viện dùng chung
pub use types::*;

pub mod traits {
    pub use hal_arch_trait::*;
    pub use hal_display::*;
    pub use hal_hypervisor::*;
    pub use hal_interrupt::*;
    pub use hal_paging::*;
    pub use hal_timer::*;
    pub use hal_traits_mmc::*;
    pub use hal_uart::*;
}
pub use traits::*; // Khớp với cấu trúc hal/traits của mày

// 2. Bộ điều phối kiến trúc (The Facade)

// Hỗ trợ RV64 (Jarvis)
#[cfg(feature = "riscv64")]
pub use hal_riscv::common;
#[cfg(feature = "riscv64")]
pub use hal_riscv::rv64::*;

// Hỗ trợ RV32 (Robot Nano)
#[cfg(feature = "riscv32")]
pub use hal_riscv::common;
#[cfg(feature = "riscv32")]
pub use hal_riscv::rv32::*;

// Hỗ trợ ARM (AArch64 + AArch32)
#[cfg(any(feature = "aarch64", feature = "arm"))]
pub use hal_arm::*;

// Hỗ trợ x86_64 + x86_32
#[cfg(any(feature = "x86_64", feature = "x86"))]
pub use hal_x86::*;

// Chặn lỗi khi quên chọn mục tiêu
#[cfg(not(any(
    feature = "riscv64",
    feature = "riscv32",
    feature = "aarch64",
    feature = "arm",
    feature = "x86_64",
    feature = "x86",
)))]
compile_error!(
    "Mày phải chọn 'riscv64', 'riscv32', 'aarch64', 'arm', 'x86_64', hoặc 'x86' thì ViCell mới chạy được!"
);
