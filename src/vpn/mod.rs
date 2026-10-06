//! VPN на ядре mihomo (Clash.Meta — то же ядро, что внутри FlClash).
//! Подписки, сборка конфига, управление через REST API, обновление ядра по релизам FlClash, геофайлы.

use crate::common::*;
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};
use std::fs;
use std::io::Read;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use url::Url;

include!("subs.rs");
include!("fetch.rs");
include!("profile_files.rs");
include!("state.rs");
include!("config.rs");
include!("core.rs");
include!("conflicts.rs");

#[cfg(test)]
mod contract_tests {
    include!("contract_tests_a.rs");
    include!("contract_tests_b.rs");
}
