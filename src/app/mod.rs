//! Запуск приложения: проверка описания, план, ярлык, общий туннель.

pub mod cli;
pub mod desktop;
pub mod plan;
pub mod reassign;
pub mod shared;
pub mod spec;

pub use desktop::{desktop_entry, exec_quote};
pub use plan::{AppLaunchPlan, plan};
pub use reassign::{Reassign, ReassignReason, decide};
pub use shared::{RefAction, TunnelRefs, rebuild};
pub use spec::{LaunchSpec, SpecError, check};
