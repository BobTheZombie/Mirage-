//! Device access service seam.

use crate::kernel::device::{DeviceDescriptor, DeviceId};
use crate::kernel::process::ProcessId;
use crate::kernel::{Kernel, KernelResult};

/// Kernel-internal adapter for enumerating and accessing devices.
pub trait DeviceService {
    fn enumerate_devices(&self, out: &mut [DeviceDescriptor]) -> usize;

    fn device_info(&self, id: DeviceId) -> Option<DeviceDescriptor>;

    fn device_read(
        &self,
        caller: ProcessId,
        id: DeviceId,
        buffer: &mut [u8],
    ) -> KernelResult<usize>;

    fn device_write(&self, caller: ProcessId, id: DeviceId, data: &[u8]) -> KernelResult<usize>;
}

impl<const MAX_PROC: usize, const MSG_DEPTH: usize> DeviceService for Kernel<MAX_PROC, MSG_DEPTH> {
    fn enumerate_devices(&self, out: &mut [DeviceDescriptor]) -> usize {
        Kernel::enumerate_devices(self, out)
    }

    fn device_info(&self, id: DeviceId) -> Option<DeviceDescriptor> {
        Kernel::device_info(self, id)
    }

    fn device_read(
        &self,
        caller: ProcessId,
        id: DeviceId,
        buffer: &mut [u8],
    ) -> KernelResult<usize> {
        Kernel::device_read(self, caller, id, buffer)
    }

    fn device_write(&self, caller: ProcessId, id: DeviceId, data: &[u8]) -> KernelResult<usize> {
        Kernel::device_write(self, caller, id, data)
    }
}
