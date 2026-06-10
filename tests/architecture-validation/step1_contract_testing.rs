// SPDX-License-Identifier: MPL-2.0
// Architecture Validation Test: Step 1 - Contract Testing

//! Mock implementations to verify trait contracts are sufficient.

#![no_std]

extern crate alloc;
use alloc::vec::Vec;
use alloc::boxed::Box;
use api::*;

/// Mock network driver with complex state for ViStateTransfer testing.
struct MockNetworkDriver {
    active_connections: usize,
    buffer_size: usize,
    ip_address: [u8; 4],
}

impl MockNetworkDriver {
    fn new() -> Self {
        Self {
            active_connections: 42,
            buffer_size: 4096,
            ip_address: [192, 168, 1, 100],
        }
    }
}

impl ViStateTransfer for MockNetworkDriver {
    fn state_size(&self) -> usize {
        // Test: Can we calculate size for all state?
        core::mem::size_of::<usize>() * 2 + 4
    }

    fn serialize_state(&self, buffer: &mut [u8]) -> Result<usize> {
        // Test: Can we serialize all critical state?
        if buffer.len() < self.state_size() {
            return Err(Error::InvalidArgument);
        }

        let mut offset = 0;
        
        // Serialize active_connections
        buffer[offset..offset + 8].copy_from_slice(&self.active_connections.to_le_bytes());
        offset += 8;
        
        // Serialize buffer_size
        buffer[offset..offset + 8].copy_from_slice(&self.buffer_size.to_le_bytes());
        offset += 8;
        
        // Serialize IP address
        buffer[offset..offset + 4].copy_from_slice(&self.ip_address);
        offset += 4;

        Ok(offset)
    }

    fn deserialize_state(&mut self, buffer: &[u8]) -> Result<()> {
        // Test: Can we restore state completely?
        if buffer.len() < self.state_size() {
            return Err(Error::InvalidArgument);
        }

        let mut offset = 0;
        
        // Deserialize active_connections
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&buffer[offset..offset + 8]);
        self.active_connections = usize::from_le_bytes(bytes);
        offset += 8;
        
        // Deserialize buffer_size
        bytes.copy_from_slice(&buffer[offset..offset + 8]);
        self.buffer_size = usize::from_le_bytes(bytes);
        offset += 8;
        
        // Deserialize IP address
        self.ip_address.copy_from_slice(&buffer[offset..offset + 4]);

        Ok(())
    }
}

/// Mock VMM for ViVmRuntime testing.
struct MockVMM {
    vms: Vec<MockVM>,
}

struct MockVM {
    id: usize,
    state: VmState,
    running: bool,
}

impl MockVMM {
    fn new() -> Self {
        Self { vms: Vec::new() }
    }
}

impl ViVmRuntime for MockVMM {
    fn create_vm(&mut self, state: VmState) -> Result<usize> {
        // Test: Can we create VM with provided state?
        let id = self.vms.len();
        self.vms.push(MockVM {
            id,
            state,
            running: false,
        });
        Ok(id)
    }

    fn run_vcpu(&mut self, vm_id: usize, _vcpu_id: usize) -> Result<VmTrap> {
        // Test: Can we simulate VM execution?
        if vm_id >= self.vms.len() {
            return Err(Error::NotFound);
        }
        
        self.vms[vm_id].running = true;
        
        // Simulate a syscall trap
        Ok(VmTrap::Syscall {
            nr: 1, // write
            args: [1, 0x1000, 13, 0, 0, 0],
        })
    }

    fn handle_trap(&mut self, vm_id: usize, _vcpu_id: usize, trap: VmTrap) -> Result<()> {
        // Test: Can we handle different trap types?
        if vm_id >= self.vms.len() {
            return Err(Error::NotFound);
        }

        match trap {
            VmTrap::Syscall { nr, .. } => {
                // Simulate syscall handling
                if nr == 1 {
                    // write syscall - would translate to ViCell API
                    Ok(())
                } else {
                    Err(Error::InvalidArgument)
                }
            }
            VmTrap::PageFault { .. } => {
                // Would allocate page and map
                Ok(())
            }
            VmTrap::Halt => {
                self.vms[vm_id].running = false;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn map_memory(
        &mut self,
        vm_id: usize,
        _gpa: PhysAddr,
        _hpa: PhysAddr,
        _size: usize,
        _writable: bool,
    ) -> Result<()> {
        // Test: Can we map memory?
        if vm_id >= self.vms.len() {
            return Err(Error::NotFound);
        }
        Ok(())
    }

    fn destroy_vm(&mut self, vm_id: usize) -> Result<()> {
        // Test: Can we clean up VM?
        if vm_id >= self.vms.len() {
            return Err(Error::NotFound);
        }
        
        // In real implementation, would free resources
        self.vms[vm_id].running = false;
        Ok(())
    }
}

// Test scenarios
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ViStateTransfer_roundtrip() {
        let mut driver1 = MockNetworkDriver::new();
        let size = driver1.state_size();
        let mut buffer = vec![0u8; size];
        
        // Serialize
        let written = driver1.serialize_state(&mut buffer).unwrap();
        assert_eq!(written, size);
        
        // Deserialize into new instance
        let mut driver2 = MockNetworkDriver {
            active_connections: 0,
            buffer_size: 0,
            ip_address: [0, 0, 0, 0],
        };
        driver2.deserialize_state(&buffer).unwrap();
        
        // Verify state transferred
        assert_eq!(driver2.active_connections, 42);
        assert_eq!(driver2.buffer_size, 4096);
        assert_eq!(driver2.ip_address, [192, 168, 1, 100]);
    }

    #[test]
    fn test_ViVmRuntime_lifecycle() {
        let mut vmm = MockVMM::new();
        
        // Create VM
        let vm_state = VmState {
            gpa_base: 0x8000_0000,
            gpa_size: 128 * 1024 * 1024, // 128MB
            entry: 0x8000_0000,
            vcpu_count: 2,
        };
        let vm_id = vmm.create_vm(vm_state).unwrap();
        assert_eq!(vm_id, 0);
        
        // Map memory
        vmm.map_memory(vm_id, 0x8000_0000, 0x4000_0000, 4096, true).unwrap();
        
        // Run VCPU
        let trap = vmm.run_vcpu(vm_id, 0).unwrap();
        match trap {
            VmTrap::Syscall { nr, .. } => assert_eq!(nr, 1),
            _ => panic!("Expected syscall trap"),
        }
        
        // Handle trap
        vmm.handle_trap(vm_id, 0, trap).unwrap();
        
        // Destroy VM
        vmm.destroy_vm(vm_id).unwrap();
    }
}
