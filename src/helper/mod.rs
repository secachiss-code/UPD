//! Системный помощник для графического интерфейса.
//!
//! Интерфейс работает от обычного пользователя; всё, что требует root (установка, зеркала, VPN,
//! живой снимок ядра VPN через закрытый сокет), он просит у помощника `cm helper`. Помощник
//! запускается systemd по обращению к сокету `/run/cm/helper.sock` и сам завершается без работы.
//! Каждый запрос проверяется через polkit (`pkcheck`) для процесса, который прислал запрос
//! (pid, время старта и uid берутся из SO_PEERCRED, а не из запроса).
//!
//! Протокол — строки JSON: запрос `Envelope`, ответ `Reply`; после `attach` помощник шлёт поток `Event`.
//! Адрес подписки передаётся только в теле запроса: он не попадает в аргументы команд, журнал и вывод.

use crate::backend::{self, Backend};
use crate::common::*;
use crate::summary::OpStatus;
use crate::{extras, vpn};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};


include!("protocol.rs");
include!("auth.rs");
include!("ops.rs");
include!("ops_run.rs");
include!("settings.rs");
include!("tests.rs");
