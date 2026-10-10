//! Кадры контроллера. В кадре нет uid, пути и байтов конфига: личность берётся из сокета.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const CONTROL_VERSION: u32 = 1;
pub const MAX_FRAME_BYTES: usize = 64 << 10;

/// Что приложению нужно от графической сессии клиента. Приложение работает под uid самого
/// клиента, поэтому эти значения не дают ему ничего сверх его прав. Остальное окружение
/// (`PATH`, `HOME`, `XDG_RUNTIME_DIR`, `TZ`, `LANG`) задаёт контроллер.
pub const SESSION_ENV_KEYS: &[&str] = &[
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XAUTHORITY",
    "DBUS_SESSION_BUS_ADDRESS",
    "PULSE_SERVER",
    "XDG_SESSION_TYPE",
    "XDG_CURRENT_DESKTOP",
];
pub const MAX_ENV_VALUE_BYTES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub v: u32,
    pub id: String,
    pub op: Op,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Op {
    /// Создать каталоги экземпляра, в том числе `config/` владельца для его конфигов.
    InstancePrepare {
        instance: String,
    },
    WorkerStart {
        instance: String,
        generation: u64,
    },
    WorkerReload {
        instance: String,
        generation: u64,
        next_generation: u64,
    },
    WorkerStop {
        instance: String,
        generation: u64,
    },
    WorkerStatus {
        instance: String,
    },
    NetApply {
        instance: String,
        generation: u64,
    },
    NetRevert {
        instance: String,
        generation: u64,
    },
    AppLaunch {
        instance: String,
        generation: u64,
        program: String,
        args: Vec<String>,
        /// Переменные сессии клиента. Принимаются только имена из [`SESSION_ENV_KEYS`].
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        env: BTreeMap<String, String>,
    },
    Reconcile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpClass {
    Status,
    Worker,
    Net,
    App,
}

impl Op {
    pub fn class(&self) -> OpClass {
        match self {
            Self::WorkerStatus { .. } => OpClass::Status,
            Self::InstancePrepare { .. }
            | Self::WorkerStart { .. }
            | Self::WorkerReload { .. }
            | Self::WorkerStop { .. }
            | Self::Reconcile => OpClass::Worker,
            Self::NetApply { .. } | Self::NetRevert { .. } => OpClass::Net,
            Self::AppLaunch { .. } => OpClass::App,
        }
    }

    pub fn instance(&self) -> Option<&str> {
        match self {
            Self::InstancePrepare { instance }
            | Self::WorkerStart { instance, .. }
            | Self::WorkerReload { instance, .. }
            | Self::WorkerStop { instance, .. }
            | Self::WorkerStatus { instance }
            | Self::NetApply { instance, .. }
            | Self::NetRevert { instance, .. }
            | Self::AppLaunch { instance, .. } => Some(instance),
            Self::Reconcile => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reply {
    pub v: u32,
    pub id: String,
    pub ok: bool,
    pub code: String,
    pub data: Option<ReplyData>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReplyData {
    Started {
        generation: u64,
    },
    Status {
        running: bool,
        generation: Option<u64>,
        api: String,
        route: String,
        remote: String,
    },
    Reconciled {
        compensated: u32,
    },
    Net {
        index: u8,
        netns: String,
    },
    Launched {
        pid: u32,
    },
}

impl Reply {
    pub fn ok(id: impl Into<String>, data: Option<ReplyData>) -> Self {
        Self {
            v: CONTROL_VERSION,
            id: id.into(),
            ok: true,
            code: "ok".to_owned(),
            data,
        }
    }

    pub fn error(id: impl Into<String>, error: ControlError) -> Self {
        Self {
            v: CONTROL_VERSION,
            id: id.into(),
            ok: false,
            code: error.code().to_owned(),
            data: None,
        }
    }
}

/// Фиксированные коды ответа. Вариант не несёт текст ОС, путь или байты запроса.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlError {
    BadFrame,
    TooLarge,
    UnsupportedVersion,
    BadId,
    BadInstance,
    BadArgument,
    Denied,
    PeerChanged,
    GenerationMismatch,
    Busy,
    Quota,
    NotRunning,
    Conflict,
    InvalidConfig,
    Timeout,
    Unsupported,
    Crashed,
    Failed,
}

impl ControlError {
    pub fn code(self) -> &'static str {
        match self {
            Self::BadFrame => "bad_frame",
            Self::TooLarge => "too_large",
            Self::UnsupportedVersion => "unsupported_version",
            Self::BadId => "bad_id",
            Self::BadInstance => "bad_instance",
            Self::BadArgument => "bad_argument",
            Self::Denied => "denied",
            Self::PeerChanged => "peer_changed",
            Self::GenerationMismatch => "generation_mismatch",
            Self::Busy => "busy",
            Self::Quota => "quota",
            Self::NotRunning => "not_running",
            Self::Conflict => "conflict",
            Self::InvalidConfig => "invalid_config",
            Self::Timeout => "timeout",
            Self::Unsupported => "unsupported",
            Self::Crashed => "crashed",
            Self::Failed => "failed",
        }
    }
}

impl std::fmt::Display for ControlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for ControlError {}
