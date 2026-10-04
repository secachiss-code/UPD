//! Общая логика cm: пакетные менеджеры, зеркала, VPN, состояние и помощник для графического интерфейса.
//! Бинарник `cm` (CLI и TUI) и апплет COSMIC используют одни и те же операции и правила.

#[macro_use]
pub mod i18n;
pub mod i18n_table;
pub mod backend;
pub mod common;
pub mod extras;
pub mod helper;
pub mod mirrors;
pub mod migration;
pub mod profiles;
pub mod sources;
pub mod status;
pub mod summary;
pub mod vpn;

pub use status::{gather_status, or_dash, sub_info, vpn_line, Status};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
