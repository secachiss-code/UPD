//! Запуск приложения: проверка описания, план, ярлык, общий туннель.

pub mod assign;
pub mod cli;
pub mod desktop;
pub mod env;
pub mod instance;
pub mod packaging;
pub mod plan;
pub mod reassign;
pub mod shared;
pub mod spec;
pub mod tunnel;

pub use desktop::{desktop_entry, exec_quote};
pub use plan::{AppLaunchPlan, plan};
pub use reassign::{Reassign, ReassignReason, decide};
pub use shared::{RefAction, TunnelRefs, rebuild};
pub use spec::{LaunchSpec, SpecError, check};
