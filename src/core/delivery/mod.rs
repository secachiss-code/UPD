//! Закреплённые ядра: проверка, распаковка и атомарная установка.

mod fetch;
mod install;
mod pin;
mod unpack;

pub use fetch::{Download, TransportDownload, obtain};
pub use install::{Installed, current, install, rollback};
pub use pin::{Archive, CoreKind, DeliveryError, PINS, Pin, pin, verify_sha256};
pub use unpack::{unpack_gzip, unpack_zip_member};
