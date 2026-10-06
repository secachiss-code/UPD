//! Core instance contract. Real process control arrives with later I04 tasks.

pub mod adapter;
pub mod fake;
pub mod instance;
pub mod leases;
pub mod mihomo;
pub mod unit;
pub mod worker;

pub use adapter::{
    ApiState, CoreAdapter, CoreCapabilities, CoreConfig, CoreError, CoreReadiness, CoreStatistics,
    RemoteState, RouteState,
};
pub use fake::{FakeAdapter, FakeOp};
pub use instance::{InstanceDirs, InstanceError, InstanceId, InstanceRoot};
pub use leases::{Lease, LeaseError, LeaseRegistry, ResourceKind};
pub use unit::{NetPrivileges, legacy_hardening_directives, render_core_unit};
pub use worker::{WorkerError, generate as generate_worker};
