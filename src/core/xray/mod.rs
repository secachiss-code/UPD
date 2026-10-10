//! Второе ядро: конфиг и жизненный цикл через тот же CoreAdapter.

pub mod config;
pub mod lifecycle;

pub use config::{XrayConfigError, generate_config, outbound};
pub use lifecycle::XrayWorker;
