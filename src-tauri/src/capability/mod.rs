//! Hardware capability probing and resource policy.
//!
//! Split so that interpretation is testable without the hardware present:
//! `probe` holds the data model and pure parsers, `collect` performs the I/O.

pub mod collect;
pub mod probe;

pub use collect::{capabilities, capabilities_uncached, set_torch_cuda};
pub use probe::{Capabilities, CpuInfo, CudaStatus, GpuInfo, GpuVendor, RamInfo, VulkanStatus};
