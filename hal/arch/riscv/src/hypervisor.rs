use hal_hypervisor::{ViHypervisor, ViVmExit, ViVmStub, ViVcpuStub, ViStage2TableStub};
use types::{ViResult, ViError};

/// RISC-V hypervisor stub — all guest ops return NotSupported.
///
/// H-extension is absent on current RISC-V boards (SG2042/K230); this impl
/// makes the multi-arch trait contract explicit at the HAL level.  The kernel's
/// `hypervisor::registry` provides the same ENOSYS behaviour at syscall dispatch.
pub struct RiscV64Hypervisor;

impl ViHypervisor for RiscV64Hypervisor {
    type Vm = ViVmStub;
    type Vcpu = ViVcpuStub;
    type Stage2Table = ViStage2TableStub;

    fn create_vm(&self) -> ViResult<Self::Vm> { Err(ViError::NotSupported) }
    fn create_vcpu(&self, _: &mut Self::Vm) -> ViResult<Self::Vcpu> { Err(ViError::NotSupported) }
    fn map_guest(&self, _: &mut Self::Stage2Table, _: u64, _: u64, _: usize, _: bool) -> ViResult<()> { Err(ViError::NotSupported) }
    fn run_vcpu(&self, _: &mut Self::Vcpu) -> ViResult<ViVmExit> { Err(ViError::NotSupported) }
    fn inject_irq(&self, _: &mut Self::Vcpu, _: u32) -> ViResult<()> { Err(ViError::NotSupported) }
}
