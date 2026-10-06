//! Общее: пути, конфиг, состояние, запуск команд, сведения о системе.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, Read, Write};
use std::net::Ipv6Addr;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingUnit { Count, Seconds, Hours, Days, Gibibytes, Port, Flag }
#[derive(Clone, Copy, Debug)]
pub struct NumberSetting { pub min: i64, pub max: i64, pub step: i64, pub unit: SettingUnit }
/// Shared numeric ranges for config validation and every settings interface.
pub fn number_setting(key: &str) -> Option<NumberSetting> {
    use SettingUnit::*;
    let (min, max, unit) = match key {
        "keep" => (1, 10, Count), "timeout" => (2, MAX_TIMEOUT as i64, Seconds),
        "extra_from_list" => (0, MAX_MIRRORS as i64, Count), "rescan_count" => (3, MAX_MIRRORS as i64, Count),
        "retries" => (1, MAX_RETRIES as i64, Count),
        "mirror_max_age_h" | "max_lag_h" | "vpn_sub_update_h" | "vpn_core_check_h" => (1, MAX_HOURS, Hours),
        "network_memory_days" => (0, 365, Days),
        "parallel" | "parallel_vpn" => (1, MAX_PARALLEL as i64, Count), "min_free_gb" => (0, 1 << 20, Gibibytes),
        "vpn_port" => (1024, 65535, Port), "vpn_mode" => (0, 2, Count),
        "prefetch" | "prefetch_on_battery" | "prefetch_on_metered" | "flatpak" | "aur" | "firmware" | "news" | "snapshot" |
        "vpn_tun" | "vpn_autostart" | "vpn_direct_ru" | "vpn_direct_lan" | "vpn_auto_select" | "vpn_auto_allow_ru" | "vpn_dns" | "vpn_ipv6" | "vpn_allow_lan" => (0, 1, Flag),
        _ => return None,
    };
    Some(NumberSetting { min, max, step: 1, unit })
}

pub type Log<'a> = &'a dyn Fn(&str);

pub fn env_or(k: &str, def: &str) -> String {
    std::env::var(k).ok().filter(|v| !v.is_empty())
        .or_else(|| k.strip_prefix("CM_").and_then(|suffix| std::env::var(format!("UPD_{suffix}")).ok()).filter(|v| !v.is_empty()))
        .unwrap_or_else(|| def.to_string())
}

pub fn conf_path() -> String {
    env_or("CM_CONF", if Path::new("/etc/cm.conf").exists() || !Path::new("/etc/upd.conf").exists() { "/etc/cm.conf" } else { "/etc/upd.conf" })
}
pub fn state_dir() -> String {
    env_or("CM_STATE_DIR", if Path::new("/var/lib/cm").exists() || !Path::new("/var/lib/upd").exists() { "/var/lib/cm" } else { "/var/lib/upd" })
}
pub fn test_mode() -> bool {
    std::env::var("CM_STATE_DIR").is_ok() || std::env::var("UPD_STATE_DIR").is_ok()
}

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Верхняя граница часовых интервалов (10 лет): `часы * 3600` при ней заведомо помещается в i64.
pub const MAX_HOURS: i64 = 24 * 365 * 10;

/// Интервал в часах → секунды без переполнения; значение сжимается в 1..=MAX_HOURS.
pub fn hours_secs(h: i64) -> i64 {
    h.clamp(1, MAX_HOURS).checked_mul(3600).unwrap_or(i64::MAX)
}

/// Прошло ли `secs` секунд с момента `since` (без переполнения при любых значениях).
pub fn elapsed_at_least(since: i64, secs: i64) -> bool {
    now().saturating_sub(since) >= secs
}

/// Верхние границы ресурсных настроек: число потоков, повторов и секунд.
pub const MAX_PARALLEL: usize = 16;
pub const MAX_RETRIES: u32 = 20;
pub const MAX_TIMEOUT: u64 = 120;
pub const MAX_MIRRORS: usize = 64;

fn enabled_by_default() -> bool { true }

// ---------- конфиг ----------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub keep: usize,
    pub timeout: u64,
    pub extra_from_list: usize,
    pub rescan_count: usize,
    pub retries: u32,
    pub mirror_max_age_h: i64,
    pub network_memory_days: i64,
    pub max_lag_h: i64,
    pub parallel: usize,
    pub parallel_vpn: usize,
    pub prefetch: bool,
    pub prefetch_on_battery: bool,
    pub prefetch_on_metered: bool,
    pub min_free_gb: u64,
    pub flatpak: bool,
    pub aur: bool,
    pub firmware: bool,
    pub news: bool,
    pub snapshot: bool,
    pub vpn_tun: bool,
    pub vpn_autostart: bool,
    pub vpn_direct_ru: bool,
    pub vpn_direct_lan: bool,
    pub vpn_auto_select: bool,
    #[serde(default = "enabled_by_default")]
    pub vpn_auto_allow_ru: bool,
    pub vpn_dns: bool,
    pub vpn_ipv6: bool,
    pub vpn_allow_lan: bool,
    pub vpn_port: u16,
    pub vpn_mode: u8,
    pub vpn_sub_update_h: i64,
    pub vpn_core_check_h: i64,
    /// язык интерфейса: ru, en, de, it, zh, ar или auto (по локали)
    pub lang: String,
    pub mirrors: Vec<String>,
    #[serde(skip)]
    baseline: Option<Box<Config>>,
}

/// (ключ, описание) — порядок и тексты для записи файла настроек.
const DOCS: &[(&str, &str)] = &[
    ("keep", "Сколько лучших рабочих зеркал ставить первыми"),
    ("timeout", "Таймаут замера одного зеркала, секунд"),
    ("extra_from_list", "Сколько зеркал из текущего списка дополнительно замерять при проверке"),
    ("rescan_count", "Сколько зеркал брать из автопоиска при смене сети"),
    ("retries", "Попыток скачать обновления, прежде чем сдаться"),
    ("mirror_max_age_h", "Перепроверять зеркала, если последний замер старше N часов"),
    ("network_memory_days", "Сколько дней помнить лучшие зеркала для каждой сети"),
    ("max_lag_h", "Отбрасывать зеркала, отставшие от самого свежего больше чем на N часов"),
    ("parallel", "Сколько зеркал замерять одновременно"),
    ("parallel_vpn", "То же, когда поднят VPN/TUN (прокси делит канал между замерами)"),
    ("prefetch", "Скачивать обновления заранее в фоне (1 — да, 0 — нет)"),
    ("prefetch_on_battery", "Фоновая загрузка при работе от батареи"),
    ("prefetch_on_metered", "Фоновая загрузка и замеры зеркал в лимитной сети (точка доступа телефона)"),
    ("min_free_gb", "Сколько ГБ должно остаться свободными после загрузки"),
    ("flatpak", "Обновлять Flatpak"),
    ("aur", "Обновлять AUR (paru/yay), только при ручном обновлении"),
    ("firmware", "Проверять прошивки через fwupd"),
    ("news", "Показывать новости Arch, требующие ручного вмешательства, перед обновлением"),
    ("snapshot", "Делать снапшот перед обновлением, если этого не делает snap-pac"),
    ("vpn_tun", "VPN: режим TUN — весь трафик системы (0 — только прокси на vpn_port)"),
    ("vpn_autostart", "VPN: запускать при загрузке"),
    ("vpn_direct_ru", "VPN: российские сайты и IP — напрямую (по геофайлам)"),
    ("vpn_direct_lan", "VPN: локальная сеть — напрямую"),
    ("vpn_auto_select", "VPN: группа «⚡ Авто» — сама выбирает самый быстрый живой сервер"),
    ("vpn_auto_allow_ru", "VPN: разрешать российские серверы в автовыборе (по названию сервера)"),
    ("vpn_dns", "VPN: свой DNS (fake-ip, DoH через VPN); 0 — системный DNS"),
    ("vpn_ipv6", "VPN: IPv6"),
    ("vpn_allow_lan", "VPN: пускать другие устройства локальной сети через этот прокси"),
    ("vpn_port", "VPN: порт прокси (HTTP+SOCKS)"),
    ("vpn_mode", "VPN: маршрутизация: 0 — по правилам, 1 — всё через VPN, 2 — всё напрямую"),
    ("vpn_sub_update_h", "VPN: обновлять подписки раз в N часов (если провайдер не указал сам)"),
    ("vpn_core_check_h", "VPN: проверять новые релизы FlClash (сигнал обновить ядро mihomo) раз в N часов"),
];

/// Состояние systemd-юнита (active, inactive, failed…) для показа на языке интерфейса.
pub fn unit_label(v: &str) -> String {
    match v {
        "active" => t!("вкл").into(),
        "inactive" => t!("выкл").into(),
        "failed" => t!("ошибка").into(),
        "activating" | "reloading" => t!("запускается").into(),
        "deactivating" => t!("останавливается").into(),
        "" => t!("не установлено").into(),
        _ => v.into(),
    }
}

/// Настройка lang из файла настроек без полной загрузки (нужна до разбора команды); нет — auto.
pub fn conf_lang() -> String {
    fs::read_to_string(conf_path())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once('='))
        .find(|(k, _)| k.trim() == "lang")
        .map(|(_, v)| v.trim().to_string())
        .unwrap_or_else(|| "auto".into())
}

impl Config {
    pub fn defaults(mirrors: Vec<String>) -> Self {
        Config {
            keep: 3,
            timeout: 10,
            extra_from_list: 8,
            rescan_count: 15,
            retries: 5,
            mirror_max_age_h: 12,
            network_memory_days: 3,
            max_lag_h: 3,
            parallel: 4,
            parallel_vpn: 2,
            prefetch: true,
            prefetch_on_battery: false,
            prefetch_on_metered: false,
            min_free_gb: 2,
            flatpak: true,
            aur: true,
            firmware: true,
            news: true,
            snapshot: true,
            vpn_tun: true,
            vpn_autostart: true,
            vpn_direct_ru: true,
            vpn_direct_lan: true,
            vpn_auto_select: true,
            vpn_auto_allow_ru: true,
            vpn_dns: true,
            vpn_ipv6: false,
            vpn_allow_lan: false,
            vpn_port: 7897,
            vpn_mode: 0,
            vpn_sub_update_h: 12,
            vpn_core_check_h: 24,
            lang: "auto".into(),
            mirrors,
            baseline: None,
        }
    }

    pub fn load(default_mirrors: Vec<String>) -> Result<Self, String> {
        let mut c = Config::defaults(default_mirrors);
        c.baseline = Some(Box::new(c.clone()));
        let path = conf_path();
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(c),
            Err(e) => return Err(format!("{path}: {e}")),
        };
        c.mirrors.clear();
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            if k == "lang" {
                c.lang = if crate::i18n::Lang::from_code(v).is_some() { v.to_ascii_lowercase() } else { "auto".into() };
                continue;
            }
            if k == "mirror" {
                if !contains(&c.mirrors, v) {
                    c.mirrors.push(v.to_string());
                }
                continue;
            }
            c.set_number(k, v);
        }
        // mirror = … без предела превратил бы список кандидатов в сотни потоков и запросов
        c.mirrors.truncate(MAX_MIRRORS);
        c.baseline = None;
        c.baseline = Some(Box::new(c.clone()));
        Ok(c)
    }

    /// Числовая настройка по имени из файла; незнакомый ключ или не число — false.
    fn set_number(&mut self, k: &str, v: &str) -> bool {
        let c = self;
        let Ok(n) = v.parse::<i64>() else { return false };
        let b = n != 0;
        // ресурсные значения сжимаются в допустимый диапазон: отказ всего конфига остановил бы и обновление пакетов
        match k {
            "keep" => c.keep = n.clamp(1, 10) as usize,
            "timeout" => c.timeout = n.clamp(2, MAX_TIMEOUT as i64) as u64,
            "extra_from_list" => c.extra_from_list = n.clamp(0, MAX_MIRRORS as i64) as usize,
            "rescan_count" => c.rescan_count = n.clamp(3, MAX_MIRRORS as i64) as usize,
            "retries" => c.retries = n.clamp(1, MAX_RETRIES as i64) as u32,
            "mirror_max_age_h" => c.mirror_max_age_h = n.clamp(1, MAX_HOURS),
            "network_memory_days" => c.network_memory_days = n.clamp(0, 365),
            "max_lag_h" => c.max_lag_h = n.clamp(1, MAX_HOURS),
            "parallel" => c.parallel = n.clamp(1, MAX_PARALLEL as i64) as usize,
            "parallel_vpn" => c.parallel_vpn = n.clamp(1, MAX_PARALLEL as i64) as usize,
            "prefetch" => c.prefetch = b,
            "prefetch_on_battery" => c.prefetch_on_battery = b,
            "prefetch_on_metered" => c.prefetch_on_metered = b,
            "min_free_gb" => c.min_free_gb = n.clamp(0, 1 << 20) as u64,
            "flatpak" => c.flatpak = b,
            "aur" => c.aur = b,
            "firmware" => c.firmware = b,
            "news" => c.news = b,
            "snapshot" => c.snapshot = b,
            "vpn_tun" => c.vpn_tun = b,
            "vpn_autostart" => c.vpn_autostart = b,
            "vpn_direct_ru" => c.vpn_direct_ru = b,
            "vpn_direct_lan" => c.vpn_direct_lan = b,
            "vpn_auto_select" => c.vpn_auto_select = b,
            "vpn_auto_allow_ru" => c.vpn_auto_allow_ru = b,
            "vpn_dns" => c.vpn_dns = b,
            "vpn_ipv6" => c.vpn_ipv6 = b,
            "vpn_allow_lan" => c.vpn_allow_lan = b,
            // годность порта для VPN проверяет сборка конфига VPN, а не чтение всех настроек
            "vpn_port" => c.vpn_port = n.clamp(1, 65535) as u16,
            "vpn_mode" => c.vpn_mode = n.clamp(0, 2) as u8,
            "vpn_sub_update_h" => c.vpn_sub_update_h = n.clamp(1, MAX_HOURS),
            "vpn_core_check_h" => c.vpn_core_check_h = n.clamp(1, MAX_HOURS),
            _ => return false,
        }
        true
    }

    /// Ключи настроек, которые можно менять по одному (интерфейс настроек).
    pub fn keys() -> impl Iterator<Item = &'static str> {
        DOCS.iter().map(|(k, _)| *k)
    }

    /// Описание настройки на языке интерфейса.
    pub fn doc(k: &str) -> Option<&'static str> {
        DOCS.iter().find(|(key, _)| *key == k).map(|(_, d)| t!(*d))
    }

    /// Текущее значение числовой настройки.
    pub fn get(&self, k: &str) -> Option<i64> {
        Config::keys().any(|x| x == k).then(|| self.value(k))
    }

    /// Изменить одну настройку: число (0/1 для флагов) или язык. Значения вне schema отвергаются.
    pub fn set(&mut self, k: &str, v: &str) -> Result<(), String> {
        let v = v.trim();
        if k == "lang" {
            if v != "auto" && crate::i18n::Lang::from_code(v).is_none() {
                return Err(t!("неизвестный язык: {0}", v));
            }
            self.lang = v.to_ascii_lowercase();
            return Ok(());
        }
        if !Config::keys().any(|x| x == k) {
            return Err(t!("неизвестная настройка: {0}", k));
        }
        if let (Some(schema), Ok(n)) = (number_setting(k), v.parse::<i64>())
            && (n < schema.min || n > schema.max) { return Err(format!("{k}: value outside its allowed range")); }
        let mut candidate = self.clone();
        if !candidate.set_number(k, v) { return Err(t!("{0}: нужно целое число", k)); }
        if v.parse::<i64>().ok() != Some(candidate.value(k)) { return Err(format!("{k}: value outside its allowed range")); }
        *self = candidate;
        Ok(())
    }

    fn value(&self, k: &str) -> i64 {
        match k {
            "keep" => self.keep as i64,
            "timeout" => self.timeout as i64,
            "extra_from_list" => self.extra_from_list as i64,
            "rescan_count" => self.rescan_count as i64,
            "retries" => self.retries as i64,
            "mirror_max_age_h" => self.mirror_max_age_h,
            "network_memory_days" => self.network_memory_days,
            "max_lag_h" => self.max_lag_h,
            "parallel" => self.parallel as i64,
            "parallel_vpn" => self.parallel_vpn as i64,
            "prefetch" => self.prefetch as i64,
            "prefetch_on_battery" => self.prefetch_on_battery as i64,
            "prefetch_on_metered" => self.prefetch_on_metered as i64,
            "min_free_gb" => self.min_free_gb as i64,
            "flatpak" => self.flatpak as i64,
            "aur" => self.aur as i64,
            "firmware" => self.firmware as i64,
            "news" => self.news as i64,
            "snapshot" => self.snapshot as i64,
            "vpn_tun" => self.vpn_tun as i64,
            "vpn_autostart" => self.vpn_autostart as i64,
            "vpn_direct_ru" => self.vpn_direct_ru as i64,
            "vpn_direct_lan" => self.vpn_direct_lan as i64,
            "vpn_auto_select" => self.vpn_auto_select as i64,
            "vpn_auto_allow_ru" => self.vpn_auto_allow_ru as i64,
            "vpn_dns" => self.vpn_dns as i64,
            "vpn_ipv6" => self.vpn_ipv6 as i64,
            "vpn_allow_lan" => self.vpn_allow_lan as i64,
            "vpn_port" => self.vpn_port as i64,
            "vpn_mode" => self.vpn_mode as i64,
            "vpn_sub_update_h" => self.vpn_sub_update_h,
            "vpn_core_check_h" => self.vpn_core_check_h,
            _ => 0,
        }
    }

    pub fn vpn_mode_name(&self) -> &'static str {
        ["rule", "global", "direct"][self.vpn_mode.min(2) as usize]
    }

    /// Merge changed fields into the latest version under a cross-process lock.
    pub fn save(&mut self) -> std::io::Result<()> {
        let _runtime = vpn_config_lock(true).map_err(std::io::Error::other)?;
        let _config = config_lock(true).map_err(std::io::Error::other)?;
        let mut latest = Config::load(self.mirrors.clone()).map_err(std::io::Error::other)?;
        let previous = latest.clone();
        if let Some(base) = &self.baseline {
            for (key, _) in DOCS {
                if self.value(key) != base.value(key) { latest.set_number(key, &self.value(key).to_string()); }
            }
            if self.lang != base.lang { latest.lang = self.lang.clone(); }
            latest.mirrors.retain(|mirror| !base.mirrors.contains(mirror) || self.mirrors.contains(mirror));
            for mirror in &self.mirrors {
                if !base.mirrors.contains(mirror) && !latest.mirrors.contains(mirror) { latest.mirrors.push(mirror.clone()); }
            }
        } else { latest = self.clone(); }
        latest.validate().map_err(std::io::Error::other)?;
        latest.validate_changed_from(&previous).map_err(std::io::Error::other)?;
        latest.save_locked()?;
        latest.baseline = None;
        latest.baseline = Some(Box::new(latest.clone()));
        *self = latest;
        Ok(())
    }

    pub fn update(default_mirrors: Vec<String>, change: impl FnOnce(&mut Config) -> Result<(), String>) -> Result<Config, String> {
        let _runtime = vpn_config_lock(true)?;
        let _config = config_lock(true)?;
        let mut candidate = Config::load(default_mirrors)?;
        let previous = candidate.clone();
        change(&mut candidate)?;
        candidate.validate()?;
        candidate.validate_changed_from(&previous)?;
        candidate.save_locked().map_err(|error| format!("not saved: {error}"))?;
        candidate.baseline = None;
        candidate.baseline = Some(Box::new(candidate.clone()));
        Ok(candidate)
    }

    pub fn validate(&self) -> Result<(), String> {
        let mut normalized = self.clone();
        for (key, _) in DOCS {
            normalized.set_number(key, &self.value(key).to_string());
            if normalized.value(key) != self.value(key) { return Err(format!("{key}: value outside its allowed range")); }
        }
        crate::vpn::check_port(self)?;
        if self.mirrors.len() > MAX_MIRRORS || self.mirrors.iter().any(|mirror| mirror.contains(['\r', '\n'])) {
            return Err("invalid mirror list".into());
        }
        if self.lang != "auto" && crate::i18n::Lang::from_code(&self.lang).is_none() { return Err("invalid language".into()); }
        Ok(())
    }

    fn validate_changed_from(&self, previous: &Config) -> Result<(), String> {
        if DOCS.iter().any(|(key, _)| key.starts_with("vpn_") && self.value(key) != previous.value(key)) {
            let subscriptions = crate::vpn::load_subs()?;
            if !subscriptions.list.is_empty() { crate::vpn::build_config(self)?; }
        }
        Ok(())
    }

    pub fn revision(&self) -> String {
        sha1_smol::Sha1::from(serde_json::to_vec(self).unwrap_or_default()).digest().to_string()
    }

    fn save_locked(&self) -> std::io::Result<()> {
        let mut s = String::from(t!("# cm — настройки. Правится вручную или через TUI (cm → Зеркала).\n"));
        s += &format!("\n# {}\nlang = {}\n", t!("Язык интерфейса: ru, en, de, it, zh, ar или auto (по локали системы)"), self.lang);
        for (k, doc) in DOCS {
            s += &format!("\n# {}\n{k} = {}\n", t!(*doc), self.value(k));
        }
        s += t!("\n# Предпочитаемые зеркала: замеряются всегда, наравне с найденными автоматически\n");
        for m in &self.mirrors {
            s += &format!("mirror = {m}\n");
        }
        atomic_write(Path::new(&conf_path()), s.as_bytes(), 0o644)
    }
}

// ---------- состояние ----------

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Probe {
    pub url: String,
    #[serde(default)]
    pub src: String,
    #[serde(default)]
    pub ok: bool,
    /// скорость этого замера, байт/с
    #[serde(default)]
    pub speed: f64,
    /// сглаженная скорость (по истории замеров в этой сети)
    #[serde(default)]
    pub score: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lag_h: Option<f64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub err: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Stat {
    pub ewma: f64,
    pub ok: u32,
    pub fail: u32,
    pub streak_fail: u32,
    pub last: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct NetMem {
    #[serde(default)]
    pub best: Vec<String>,
    #[serde(default)]
    pub checked: i64,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub stats: BTreeMap<String, Stat>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct MirrorState {
    #[serde(default)]
    pub checked: i64,
    #[serde(default)]
    pub results: Vec<Probe>,
    #[serde(default)]
    pub best: Vec<String>,
    #[serde(default)]
    pub fingerprint: String,
    #[serde(default)]
    pub pending_apply: bool,
    #[serde(default)]
    pub pending_fingerprint: String,
    #[serde(default)]
    pub pending_label: String,
    #[serde(default)]
    pub pending_fallback: Option<Vec<String>>,
    #[serde(default)]
    pub apply_error: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub networks: BTreeMap<String, NetMem>,
    #[serde(default)]
    pub event: String,
    #[serde(default)]
    pub event_time: i64,
    #[serde(default)]
    pub hints: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct News {
    pub title: String,
    pub date: i64,
    pub link: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PackageCheckFailure {
    Refresh,
    UpdateList,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct UpdState {
    #[serde(default)]
    pub checked: i64,
    #[serde(default)]
    pub list: Vec<String>,
    #[serde(default)]
    pub downloaded: bool,
    #[serde(default)]
    pub download_size: Option<u64>,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub package_check_failure: Option<PackageCheckFailure>,
    #[serde(default)]
    pub space_check_error: Option<String>,
    #[serde(default)]
    pub skipped: String,
    #[serde(default)]
    pub flatpak: Vec<String>,
    #[serde(default)]
    pub flatpak_error: String,
    #[serde(default)]
    pub firmware: Vec<String>,
    #[serde(default)]
    pub firmware_error: String,
    #[serde(default)]
    pub news: Vec<News>,
}

pub fn load_json<T: for<'de> Deserialize<'de> + Default>(name: &str) -> T {
    fs::read(Path::new(&state_dir()).join(name))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_json<T: Serialize>(name: &str, v: &T) -> std::io::Result<()> {
    let dir = PathBuf::from(state_dir());
    fs::create_dir_all(&dir)?;
    let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o755));
    let data = serde_json::to_vec_pretty(v).unwrap_or_default();
    atomic_write(&dir.join(name), &data, 0o644)
}

pub fn atomic_write(path: &Path, data: &[u8], mode: u32) -> std::io::Result<()> {
    atomic_write_with_hook(path, data, mode, || {})
}

/// Atomically replace `path` with a file owned by the caller and the exact requested mode.
/// A destination symlink itself is replaced; its target is never followed. APT sources use
/// their specialized writer because those files preserve the distribution's uid/gid and mode.
struct AtomicWriteOps<F, W, R, S, N> {
    after_create: F,
    write_data: W,
    rename_file: R,
    sync_parent: S,
    temp_path: N,
}

fn atomic_write_with_hook(
    path: &Path,
    data: &[u8],
    mode: u32,
    after_create: impl FnOnce(),
) -> std::io::Result<()> {
    atomic_write_with_ops(
        path,
        data,
        mode,
        AtomicWriteOps {
            after_create,
            write_data: |file: &mut fs::File, data: &[u8]| file.write_all(data),
            rename_file: |from: &Path, to: &Path| fs::rename(from, to),
            sync_parent: sync_directory,
            temp_path: unique_temp_path(),
        },
    )
}

fn atomic_write_with_ops<F, W, R, S, N>(
    path: &Path,
    data: &[u8],
    mode: u32,
    ops: AtomicWriteOps<F, W, R, S, N>,
) -> std::io::Result<()>
where
    F: FnOnce(),
    W: FnOnce(&mut fs::File, &[u8]) -> std::io::Result<()>,
    R: FnOnce(&Path, &Path) -> std::io::Result<()>,
    S: FnOnce(&Path) -> std::io::Result<()>,
    N: FnMut(&Path, usize) -> PathBuf,
{
    let AtomicWriteOps { after_create, write_data, rename_file, sync_parent, mut temp_path } = ops;
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let (tmp, mut file) = create_atomic_temp(dir, mode, &mut temp_path)?;
    let mut cleanup = TempPathGuard { path: tmp.clone(), armed: true };
    after_create();
    write_data(&mut file, data).map_err(|e| staged_io("write temporary file", e))?;
    file.sync_all().map_err(|e| staged_io("sync temporary file", e))?;
    drop(file);

    rename_file(&tmp, path).map_err(|e| staged_io("rename temporary file", e))?;
    cleanup.armed = false;
    sync_parent(dir).map_err(|e| staged_io("sync parent directory after rename", e))?;
    Ok(())
}

fn create_atomic_temp(
    dir: &Path,
    mode: u32,
    temp_path: &mut impl FnMut(&Path, usize) -> PathBuf,
) -> std::io::Result<(PathBuf, fs::File)> {
    for attempt in 0..128 {
        let path = temp_path(dir, attempt);
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true).mode(mode);
        match options.open(&path) {
            Ok(file) => {
                if let Err(error) = file.set_permissions(fs::Permissions::from_mode(mode)) {
                    drop(file);
                    let _ = fs::remove_file(&path);
                    return Err(staged_io("set temporary file mode", error));
                }
                return Ok((path, file));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(staged_io("create temporary file", error)),
        }
    }
    Err(staged_io(
        "create temporary file",
        std::io::Error::new(std::io::ErrorKind::AlreadyExists, "all unique names were occupied"),
    ))
}

fn unique_temp_path() -> impl FnMut(&Path, usize) -> PathBuf {
    const ATTEMPTS: u64 = 128;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let pid = std::process::id();
    let first = NEXT.fetch_add(ATTEMPTS, Ordering::Relaxed);
    move |dir, attempt| dir.join(format!(".cm.{pid}.{}.tmp", first.wrapping_add(attempt as u64)))
}

fn sync_directory(dir: &Path) -> std::io::Result<()> {
    fs::File::open(dir)?.sync_all()
}

fn staged_io(stage: &str, error: std::io::Error) -> std::io::Error {
    std::io::Error::new(error.kind(), format!("{stage}: {error}"))
}

struct TempPathGuard {
    path: PathBuf,
    armed: bool,
}

impl Drop for TempPathGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Блокировка: одна тяжёлая задача за раз. Освобождается при закрытии файла.
pub struct Lock(#[allow(dead_code)] fs::File);

fn named_lock(name: &str, block: bool) -> Result<Lock, String> {
    lock_in(&state_dir(), name, block)
}

fn lock_in(dir: &str, name: &str, block: bool) -> Result<Lock, String> {
    if name.is_empty() || Path::new(name).components().count() != 1 {
        return Err(t!("некорректное имя блокировки").into());
    }
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new().recursive(true).mode(0o700).create(dir).map_err(|e| format!("{dir}: {e}"))?;
    let path = Path::new(dir).join(name);
    let f = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    use std::os::fd::AsRawFd;
    let op = if block { libc::LOCK_EX } else { libc::LOCK_EX | libc::LOCK_NB };
    if unsafe { libc::flock(f.as_raw_fd(), op) } != 0 {
        return Err("busy".into());
    }
    Ok(Lock(f))
}

pub fn lock(block: bool) -> Result<Lock, String> {
    named_lock(".lock", block)
}

pub fn subscriptions_lock(block: bool) -> Result<Lock, String> {
    named_lock(".vpn-subs.lock", block)
}

/// Order: heavy operation (if held), VPN files, config, subscriptions. State mutex is never held here.
// ExecStartPre runs with ProtectSystem=strict: only the VPN directory is writable.
// All writers, including Config::save and the helper, must use the same lock here.
pub fn vpn_config_lock(block: bool) -> Result<Lock, String> { lock_in(&crate::vpn::home(), ".vpn-config.lock", block) }

pub fn config_lock(block: bool) -> Result<Lock, String> {
    let path = PathBuf::from(conf_path());
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let name = path.file_name().ok_or("invalid config path")?.to_string_lossy();
    let file = fs::OpenOptions::new().create(true).truncate(false).read(true).write(true)
        .open(parent.join(format!(".{name}.lock"))).map_err(|e| e.to_string())?;
    use std::os::fd::AsRawFd;
    let flags = libc::LOCK_EX | if block { 0 } else { libc::LOCK_NB };
    if unsafe { libc::flock(file.as_raw_fd(), flags) } != 0 { return Err("config busy".into()); }
    Ok(Lock(file))
}

// ---------- команды ----------

pub fn have(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
        .unwrap_or(false)
}

/// Предел stdout внешней команды по умолчанию.
pub const OUT_MAX: u64 = 16 << 20;
/// Сколько последних байт stderr хранится для сообщения об ошибке.
const ERR_TAIL: usize = 64 << 10;

mod probe;
pub use probe::{capture, capture_interactive, capture_with_policy, CaptureError, CapturePolicy, with_probe_scope};

/// Запуск с захватом stdout (не больше OUT_MAX); код выхода (-1, если не запустилось или вывод превысил предел).
pub fn out(cmd: &str, args: &[&str]) -> (String, i32) {
    out_limited(cmd, args, OUT_MAX).unwrap_or_else(|e| { probe::record_probe_error(e); (String::new(), -1) })
}

/// Как out, но превышение предела и отказ запуска — ошибка с причиной.
pub fn out_limited(cmd: &str, args: &[&str], max: u64) -> Result<(String, i32), CaptureError> {
    let o = capture(Command::new(cmd).args(args).env("LC_ALL", "C"), Some(max))?;
    Ok((String::from_utf8_lossy(&o.stdout).into_owned(), o.status.code().unwrap_or(-1)))
}

/// Запуск с выводом в терминал (или молча). Ok — если код 0.
pub fn run(quiet: bool, env: &[(&str, &str)], cmd: &str, args: &[&str]) -> Result<(), String> {
    let mut c = Command::new(cmd);
    c.args(args);
    for (k, v) in env {
        c.env(k, v);
    }
    if quiet {
        c.stdin(Stdio::null());
        let o = capture_interactive(&mut c, None)?;
        if o.status.success() {
            return Ok(());
        }
        let err = String::from_utf8_lossy(&o.stderr);
        return Err(format!("{cmd}: {}", last_line(&err).unwrap_or(t!("ошибка"))));
    }
    let st = c.status().map_err(|e| format!("{cmd}: {e}"))?;
    if st.success() {
        Ok(())
    } else {
        Err(t!("{1}: код {}", st.code().unwrap_or(-1), cmd))
    }
}

/// Тело HTTP-ответа не больше `max` байт: лишний байт сверх предела — ошибка, а не обрезанный ответ.
pub fn read_limited(r: ureq::Response, max: u64) -> Result<Vec<u8>, String> {
    let expected = r.header("content-length").and_then(|v| v.parse::<u64>().ok());
    if expected.map(|size| size > max).unwrap_or(false) {
        return Err(t!("ответ превышает лимит {}", fmt_bytes(max)));
    }
    let mut b = vec![];
    r.into_reader().take(max.saturating_add(1)).read_to_end(&mut b).map_err(|e| e.to_string())?;
    if b.len() as u64 > max {
        return Err(t!("ответ превышает лимит {}", fmt_bytes(max)));
    }
    if expected.map(|size| size != b.len() as u64).unwrap_or(false) {
        return Err(t!("ответ обрезан относительно Content-Length").into());
    }
    Ok(b)
}

/// Как read_limited, но текстом.
pub fn read_text(r: ureq::Response, max: u64) -> Result<String, String> {
    read_limited(r, max).map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// Поток с именем; отказ ОС — ошибка, а не паника, как у thread::spawn.
pub fn spawn_thread<T: Send + 'static>(name: &str, f: impl FnOnce() -> T + Send + 'static) -> Result<std::thread::JoinHandle<T>, String> {
    std::thread::Builder::new().name(name.into()).spawn(f).map_err(|e| t!("не удалось запустить поток: {0}", e))
}

/// Master и slave с `FD_CLOEXEC` с момента создания. `openpty` оставляет оба fd
/// наследуемыми до отдельного `fcntl`, и fork/exec в другом потоке уносит копию PTY.
pub fn open_pty_pair(rows: u16, cols: u16) -> std::io::Result<(std::fs::File, std::fs::File)> {
    use std::os::fd::{AsRawFd, FromRawFd};
    // SAFETY: posix_openpt returns a new fd or -1; O_CLOEXEC is set by the kernel before the fd is visible.
    let master_fd = unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC) };
    if master_fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let master = unsafe { std::fs::File::from_raw_fd(master_fd) };
    // SAFETY: master is an open pty master we own. grantpt/unlockpt only change its slave lock.
    if unsafe { libc::grantpt(master.as_raw_fd()) } != 0 || unsafe { libc::unlockpt(master.as_raw_fd()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // TIOCGPTPEER (Linux 4.13+) opens the slave of *this* master without a path, so another
    // devpts instance (container, private /dev, mount namespace) cannot hand back a foreign pty.
    // SAFETY: the ioctl takes open flags and returns a new fd or -1.
    let mut slave_fd = unsafe {
        libc::ioctl(master.as_raw_fd(), libc::TIOCGPTPEER, libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC)
    };
    if slave_fd < 0 {
        let error = std::io::Error::last_os_error();
        if !matches!(error.raw_os_error(), Some(libc::EINVAL | libc::ENOTTY)) {
            return Err(error);
        }
        // Older kernel: fall back to the pts path of this master.
        let mut pts: libc::c_uint = 0;
        // SAFETY: TIOCGPTN writes one c_uint into pts for this master.
        if unsafe { libc::ioctl(master.as_raw_fd(), libc::TIOCGPTN, &mut pts) } < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let path = std::ffi::CString::new(format!("/dev/pts/{pts}")).map_err(|_| std::io::Error::other("invalid pty path"))?;
        // SAFETY: path is a NUL-terminated pts name; O_CLOEXEC is applied at open.
        slave_fd = unsafe { libc::open(path.as_ptr(), libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC) };
        if slave_fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    let slave = unsafe { std::fs::File::from_raw_fd(slave_fd) };
    let size = libc::winsize { ws_row: rows.max(1), ws_col: cols.max(1), ws_xpixel: 0, ws_ypixel: 0 };
    // SAFETY: slave is an open pty slave we own; winsize is a valid stack object.
    if unsafe { libc::ioctl(slave.as_raw_fd(), libc::TIOCSWINSZ, &size) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    set_cloexec(master.as_raw_fd())?;
    set_cloexec(slave.as_raw_fd())?;
    Ok((master, slave))
}

/// Дубликат fd, у которого `FD_CLOEXEC` включён атомарно (`dup` его сбрасывает).
pub fn dup_cloexec(file: &std::fs::File) -> std::io::Result<std::fs::File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    // SAFETY: file is open; F_DUPFD_CLOEXEC returns a new fd or -1.
    let fd = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { std::fs::File::from_raw_fd(fd) })
}

fn set_cloexec(fd: std::os::fd::RawFd) -> std::io::Result<()> {
    // SAFETY: fd belongs to an open File the caller owns.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

pub fn lines(s: &str) -> Vec<String> {
    s.lines().filter(|l| !l.trim().is_empty()).map(String::from).collect()
}

pub fn last_line(s: &str) -> Option<&str> {
    s.lines().rev().map(str::trim).find(|l| !l.is_empty())
}

pub fn contains(list: &[String], s: &str) -> bool {
    list.iter().any(|x| x.trim_end_matches('/') == s.trim_end_matches('/'))
}

pub fn host_of(u: &str) -> &str {
    let u = u.trim_start_matches("https://").trim_start_matches("http://");
    u.split('/').next().unwrap_or(u)
}

/// Пользователь, от имени которого запущен sudo (для paru/flatpak --user).
pub fn invoking_user() -> Option<String> {
    std::env::var("SUDO_USER").ok().or_else(|| std::env::var("DOAS_USER").ok()).filter(|u| !u.is_empty() && u != "root")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserContext { pub uid: u32, pub name: String, pub runtime_dir: PathBuf }

impl UserContext {
    pub fn from_uid(uid: u32) -> Result<Self, String> {
        let mut passwd: libc::passwd = unsafe { std::mem::zeroed() };
        let mut buffer = vec![0 as libc::c_char; 64 << 10];
        let mut result = std::ptr::null_mut();
        let error = unsafe { libc::getpwuid_r(uid, &mut passwd, buffer.as_mut_ptr(), buffer.len(), &mut result) };
        if error != 0 || result.is_null() { return Err(format!("cannot resolve user UID {uid} through NSS")); }
        let name = unsafe { std::ffi::CStr::from_ptr(passwd.pw_name) }.to_string_lossy().into_owned();
        Ok(Self { uid, name, runtime_dir: PathBuf::from(format!("/run/user/{uid}")) })
    }

    pub fn from_name(name: &str) -> Result<Self, String> {
        let name = std::ffi::CString::new(name).map_err(|_| "invalid invoking user name".to_string())?;
        let mut passwd: libc::passwd = unsafe { std::mem::zeroed() };
        let mut buffer = vec![0 as libc::c_char; 64 << 10];
        let mut result = std::ptr::null_mut();
        let error = unsafe { libc::getpwnam_r(name.as_ptr(), &mut passwd, buffer.as_mut_ptr(), buffer.len(), &mut result) };
        if error != 0 || result.is_null() { return Err("cannot resolve invoking user through NSS".into()); }
        Self::from_uid(passwd.pw_uid)
    }

    pub fn command_env(&self) -> Vec<(String, String)> {
        vec![("SUDO_USER".into(), self.name.clone()), ("SUDO_UID".into(), self.uid.to_string()),
            ("XDG_RUNTIME_DIR".into(), self.runtime_dir.to_string_lossy().into_owned()),
            ("DBUS_SESSION_BUS_ADDRESS".into(), format!("unix:path={}/bus", self.runtime_dir.display()))]
    }
}

/// Interactive CLI identity only; background callers explicitly pass None to VPN apply.
pub fn cli_user_context() -> Result<Option<UserContext>, String> {
    let uid = unsafe { libc::geteuid() };
    if uid != 0 { return UserContext::from_uid(uid).map(Some); }
    if let Some(name) = invoking_user() {
        let context = UserContext::from_name(&name)?;
        if let Some(sudo_uid) = std::env::var("SUDO_UID").ok().and_then(|value| value.parse::<u32>().ok())
            && sudo_uid != context.uid { return Err("invoking user name and UID disagree".into()); }
        return Ok((context.uid != 0).then_some(context));
    }
    if let Some(uid) = std::env::var("PKEXEC_UID").ok().and_then(|value| value.parse::<u32>().ok()).filter(|uid| *uid != 0) {
        return UserContext::from_uid(uid).map(Some);
    }
    Ok(None)
}

pub fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

pub fn confirm(q: &str, default_yes: bool) -> bool {
    confirm_from(&mut std::io::stdin().lock(), q, default_yes)
}

/// Вопрос да/нет. Пустой Enter — ответ по умолчанию; EOF и ошибка чтения — отказ при любом умолчании:
/// закрытый stdin не должен соглашаться на установку, перезапуск служб или отказ от снапшота.
pub fn confirm_from(input: &mut dyn BufRead, q: &str, default_yes: bool) -> bool {
    print!("{q} {} ", if default_yes { "[Y/n]" } else { "[y/N]" });
    loop {
        let _ = std::io::stdout().flush();
        let mut s = String::new();
        if input.read_line(&mut s).unwrap_or(0) == 0 {
            println!();
            return false;
        }
        match crate::i18n::yes_no(&s) {
            Some(a) => return a.unwrap_or(default_yes),
            // «н» — это и Y в русской раскладке, и «нет»: переспрашиваем, а не угадываем
            None => print!("{}", t!("не понял ответ «{}»: y или д — да, n или т — нет, Enter — {} ", s.trim(), if default_yes { t!("да") } else { t!("нет") })),
        }
    }
}

// ---------- форматирование ----------

pub fn fmt_speed(p: &Probe) -> String {
    if !p.ok {
        return if p.err.is_empty() { "—".into() } else { crate::i18n::tr_data(&p.err) };
    }
    let s = if p.score > 0.0 { p.score } else { p.speed };
    if s >= 1048576.0 {
        format!("{:.1} MB/s", s / 1048576.0)
    } else {
        format!("{:.0} KB/s", s / 1024.0)
    }
}

pub fn fmt_bytes(n: u64) -> String {
    let n = n as f64;
    if n >= 1073741824.0 {
        t!("{:.1} ГБ", n / 1073741824.0)
    } else if n >= 1048576.0 {
        t!("{:.0} МБ", n / 1048576.0)
    } else {
        t!("{:.0} КБ", n / 1024.0)
    }
}

pub fn fmt_ago(ts: i64) -> String {
    if ts == 0 {
        return t!("не было").into();
    }
    let d = now() - ts;
    match d {
        _ if d < 60 => t!("только что").into(),
        _ if d < 3600 => t!("{} мин назад", d / 60),
        _ if d < 48 * 3600 => t!("{} ч назад", d / 3600),
        _ => t!("{} дн назад", d / 86400),
    }
}

/// Unix-время → «2026-09-24 22:19» (UTC-смещение берём из /etc/localtime через libc).
pub fn fmt_time(ts: i64) -> String {
    if ts == 0 {
        return t!("не было").into();
    }
    let t = ts as _;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    format!("{:04}-{:02}-{:02} {:02}:{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min)
}

/// Unix-время → «22:19:05» по местному времени: когда получен снимок данных.
pub fn fmt_clock(ts: i64) -> String {
    if ts == 0 {
        return "—".into();
    }
    let t = ts as _;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
}

// ---------- даты ----------

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// RFC 2822 / RSS / apt: «Thu, 24 Sep 2026 20:10:05 +0000» (или UTC/GMT) → unix.
pub fn parse_rfc2822(s: &str) -> Option<i64> {
    let s = s.split_once(',').map(|x| x.1).unwrap_or(s).trim();
    let f: Vec<&str> = s.split_whitespace().collect();
    if f.len() < 4 {
        return None;
    }
    let d: i64 = f[0].parse().ok()?;
    const M: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    let m = M.iter().position(|x| f[1].to_lowercase().starts_with(x))? as i64 + 1;
    let y: i64 = f[2].parse().ok()?;
    let hms: Vec<i64> = f[3].split(':').filter_map(|x| x.parse().ok()).collect();
    let (h, mi, se) = (*hms.first()?, *hms.get(1).unwrap_or(&0), *hms.get(2).unwrap_or(&0));
    let mut off = 0;
    if let Some(z) = f.get(4)
        && (z.starts_with('+') || z.starts_with('-')) && z.len() == 5 {
        let v: i64 = z[1..].parse().ok()?;
        off = (v / 100 * 3600 + v % 100 * 60) * if z.starts_with('-') { -1 } else { 1 };
    }
    Some(days_from_civil(y, m, d) * 86400 + h * 3600 + mi * 60 + se - off)
}

/// «[2026-09-24T22:14:30+0300]» из pacman.log → unix.
pub fn parse_iso(s: &str) -> Option<i64> {
    let s = s.trim_matches(|c| c == '[' || c == ']');
    let (date, rest) = s.split_once('T')?;
    let dp: Vec<i64> = date.split('-').filter_map(|x| x.parse().ok()).collect();
    if dp.len() != 3 || rest.len() < 8 {
        return None;
    }
    let hms: Vec<i64> = rest[..8].split(':').filter_map(|x| x.parse().ok()).collect();
    if hms.len() != 3 {
        return None;
    }
    let tz = &rest[8..];
    let mut off = 0;
    if tz.len() >= 5 && (tz.starts_with('+') || tz.starts_with('-')) {
        let v: i64 = tz[1..5].parse().unwrap_or(0);
        off = (v / 100 * 3600 + v % 100 * 60) * if tz.starts_with('-') { -1 } else { 1 };
    }
    Some(days_from_civil(dp[0], dp[1], dp[2]) * 86400 + hms[0] * 3600 + hms[1] * 60 + hms[2] - off)
}

// ---------- сеть ----------

pub struct NetInfo {
    pub id: String,
    pub label: String,
    pub online: bool,
    pub vpn: bool,
    pub dev: String,
}

fn net_info_from_routes(routes: Vec<DefaultRoute>) -> NetInfo {
    let mac = routes.iter().find(|r| r.family == "IPv4").map(|r| arp_mac(&r.gateway)).unwrap_or_default();
    let vpn = vpn_ifaces();
    let route_key = routes.iter().map(|r| format!("{}|{}|{}|{}", r.family, r.gateway, r.dev, r.metric)).collect::<Vec<_>>().join(";");
    let raw = format!("{route_key}|{mac}|{}", vpn.join(","));
    let id = sha1_smol::Sha1::from(raw.as_bytes()).digest().to_string()[..10].to_string();
    let dev = routes.iter().find(|r| r.family == "IPv4").or_else(|| routes.first()).map(|r| r.dev.clone()).unwrap_or_default();
    let mut label = if routes.is_empty() {
        t!("нет сети").to_string()
    } else {
        routes.iter().map(|r| t!("{} {} через {}", r.family, r.dev, r.gateway)).collect::<Vec<_>>().join(", ")
    };
    if !vpn.is_empty() {
        label += &format!(" + VPN {}", vpn.join(","));
    }
    NetInfo { id, label, online: !routes.is_empty(), vpn: !vpn.is_empty(), dev }
}

/// Отпечаток сети: шлюз, его MAC и поднятые VPN/TUN. Только /proc и /sys, без запросов в интернет.
pub fn fingerprint() -> NetInfo {
    net_info_from_routes(default_routes())
}

struct DefaultRoute {
    family: &'static str,
    gateway: String,
    dev: String,
    metric: u64,
}

fn default_routes() -> Vec<DefaultRoute> {
    [default_ipv4_route(), default_ipv6_route()].into_iter().flatten().collect()
}

fn default_ipv4_route() -> Option<DefaultRoute> {
    let data = fs::read_to_string("/proc/net/route").ok()?;
    let mut best: Option<DefaultRoute> = None;
    for l in data.lines().skip(1) {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() < 8 || f[1] != "00000000" || f[7] != "00000000" {
            continue;
        }
        let metric: u64 = f[6].parse().unwrap_or(0);
        let Ok(g) = u32::from_str_radix(f[2], 16) else { continue };
        let b = g.to_le_bytes();
        let route = DefaultRoute {
            family: "IPv4",
            gateway: format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3]),
            dev: f[0].to_string(),
            metric,
        };
        if best.as_ref().map(|x| metric < x.metric).unwrap_or(true) {
            best = Some(route);
        }
    }
    best
}

fn select_default_ipv6_route_with(data: &str, iface_up: fn(&str) -> bool) -> Option<DefaultRoute> {
    let mut best: Option<DefaultRoute> = None;
    for line in data.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 10 || f[0] != "00000000000000000000000000000000" || f[1] != "00" || f[2] != "00000000000000000000000000000000" || f[3] != "00" {
            continue;
        }
        let Ok(metric) = u64::from_str_radix(f[5], 16) else { continue };
        let Ok(flags) = u32::from_str_radix(f[8], 16) else { continue };
        if flags & 0x1 == 0 || flags & 0x200 != 0 || !iface_up(f[9]) {
            continue;
        }
        let Some(gateway) = ipv6_route_address(f[4]) else { continue };
        let route = DefaultRoute { family: "IPv6", gateway, dev: f[9].to_string(), metric };
        if best.as_ref().map(|x| metric < x.metric).unwrap_or(true) {
            best = Some(route);
        }
    }
    best
}

fn select_default_ipv6_route(data: &str) -> Option<DefaultRoute> {
    select_default_ipv6_route_with(data, route_interface_up)
}

fn default_ipv6_route() -> Option<DefaultRoute> {
    let data = fs::read_to_string("/proc/net/ipv6_route").ok()?;
    select_default_ipv6_route(&data)
}

fn ipv6_route_address(hex: &str) -> Option<String> {
    if hex.len() != 32 {
        return None;
    }
    let mut octets = [0u8; 16];
    for (i, octet) in octets.iter_mut().enumerate() {
        *octet = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(Ipv6Addr::from(octets).to_string())
}

fn route_interface_up(dev: &str) -> bool {
    let flags = fs::read_to_string(format!("/sys/class/net/{dev}/flags")).ok().and_then(|s| u32::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok());
    let operstate = fs::read_to_string(format!("/sys/class/net/{dev}/operstate")).unwrap_or_default();
    flags.map(|f| f & 0x1 != 0).unwrap_or(false) && matches!(operstate.trim(), "up" | "unknown")
}

fn arp_mac(ip: &str) -> String {
    if ip.is_empty() {
        return String::new();
    }
    fs::read_to_string("/proc/net/arp")
        .unwrap_or_default()
        .lines()
        .find_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            (f.len() >= 4 && f[0] == ip).then(|| f[3].to_string())
        })
        .unwrap_or_default()
}

fn vpn_ifaces() -> Vec<String> {
    let mut r = vec![];
    if let Ok(rd) = fs::read_dir("/sys/class/net") {
        for e in rd.flatten() {
            let p = e.path();
            let t = fs::read_to_string(p.join("type")).unwrap_or_default();
            let op = fs::read_to_string(p.join("operstate")).unwrap_or_default();
            let tun = p.join("tun_flags").exists();
            if (t.trim() == "65534" || tun) && op.trim() != "down" {
                r.push(e.file_name().to_string_lossy().into_owned());
            }
        }
    }
    r.sort();
    r
}

/// Лимитная сеть (NetworkManager): точка доступа телефона и т.п.
pub fn metered(dev: &str) -> bool {
    if dev.is_empty() || !have("nmcli") {
        return false;
    }
    let (s, _) = out("nmcli", &["-t", "-g", "GENERAL.METERED", "dev", "show", dev]);
    s.trim().starts_with("yes")
}

/// Работа от батареи: есть батарея, и ни один сетевой адаптер питания не подключён.
pub fn on_battery() -> bool {
    let Ok(rd) = fs::read_dir("/sys/class/power_supply") else { return false };
    let (mut battery, mut mains) = (false, false);
    for e in rd.flatten() {
        let p = e.path();
        let t = fs::read_to_string(p.join("type")).unwrap_or_default();
        match t.trim() {
            "Battery" => battery = true,
            "Mains" | "USB" => mains |= fs::read_to_string(p.join("online")).unwrap_or_default().trim() == "1",
            _ => {}
        }
    }
    battery && !mains
}

// ---------- система ----------

pub fn free_space(path: &str) -> std::io::Result<u64> {
    let c = std::ffi::CString::new(path).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(s.f_bavail as u64 * s.f_frsize as u64)
}

fn boot_time() -> i64 {
    fs::read_to_string("/proc/stat")
        .unwrap_or_default()
        .lines()
        .find_map(|l| l.strip_prefix("btime ").and_then(|v| v.trim().parse().ok()))
        .unwrap_or(0)
}

fn kernel_release() -> String {
    fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default().trim().to_string()
}

fn module_dirs_from(candidates: &[PathBuf]) -> Vec<PathBuf> {
    let mut dirs = vec![];
    for path in candidates {
        if !path.is_dir() {
            continue;
        }
        let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.clone());
        if !dirs.contains(&canonical) {
            dirs.push(canonical);
        }
    }
    dirs
}

fn module_dirs() -> Vec<PathBuf> {
    module_dirs_from(&[PathBuf::from("/usr/lib/modules"), PathBuf::from("/lib/modules")])
}

fn running_modules_missing(release: &str, dirs: &[PathBuf]) -> bool {
    !release.is_empty() && !dirs.iter().any(|dir| dir.join(release).is_dir())
}

/// Нужна перезагрузка: отметка дистрибутива, пропали модули запущенного ядра, или новое ядро после загрузки.
pub fn reboot_needed() -> bool {
    if Path::new("/run/reboot-required").exists() {
        return true;
    }
    let dirs = module_dirs();
    let rel = kernel_release();
    if running_modules_missing(&rel, &dirs) {
        return true;
    }
    let boot = boot_time();
    dirs.iter().any(|dir| {
        fs::read_dir(dir)
            .map(|rd| {
                rd.flatten().any(|e| {
                    fs::metadata(e.path().join("modules.dep"))
                        .and_then(|m| m.modified())
                        .map(|t| boot > 0 && t.duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0) > boot + 60)
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false)
    })
}

pub fn dir_size(dirs: &[&str]) -> u64 {
    fn walk(p: &Path) -> u64 {
        let Ok(rd) = fs::read_dir(p) else { return 0 };
        rd.flatten()
            .map(|e| match e.file_type() {
                Ok(t) if t.is_dir() => walk(&e.path()),
                Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
                _ => 0,
            })
            .sum()
    }
    dirs.iter().map(|d| walk(Path::new(d))).sum()
}

pub fn find_etc(suffixes: &[&str]) -> Vec<String> {
    fn walk(p: &Path, suf: &[&str], r: &mut Vec<String>) {
        let Ok(rd) = fs::read_dir(p) else { return };
        for e in rd.flatten() {
            let path = e.path();
            match e.file_type() {
                Ok(t) if t.is_dir() => walk(&path, suf, r),
                Ok(_) => {
                    let s = path.to_string_lossy();
                    if suf.iter().any(|x| s.ends_with(x)) {
                        r.push(s.into_owned());
                    }
                }
                _ => {}
            }
        }
    }
    let mut r = vec![];
    walk(Path::new("/etc"), suffixes, &mut r);
    r.sort();
    r
}

pub fn failed_units() -> usize {
    if !have("systemctl") {
        return 0;
    }
    lines(&out("systemctl", &["--failed", "--no-legend", "--plain"]).0).len()
}

pub fn unit_state(unit: &str) -> String {
    if !have("systemctl") {
        return String::new();
    }
    out("systemctl", &["is-active", unit]).0.trim().to_string()
}

pub fn systemd() -> bool {
    Path::new("/run/systemd/system").exists()
}

pub fn tail_file(path: &str, max: u64) -> Vec<String> {
    let Ok(mut f) = fs::File::open(path) else { return vec![] };
    use std::io::{Read, Seek, SeekFrom};
    if let Ok(m) = f.metadata()
        && m.len() > max {
        let _ = f.seek(SeekFrom::End(-(max as i64)));
    }
    let mut b = vec![];
    let _ = f.read_to_end(&mut b);
    String::from_utf8_lossy(&b).lines().map(String::from).collect()
}

// ---------- кому нужен перезапуск ----------

/// Процессы, которые держат удалённые (обновлённые) библиотеки или бинарники.
#[derive(Default, Clone, Debug)]
pub struct Restart {
    /// системные службы, которые можно перезапустить
    pub services: Vec<String>,
    /// системные службы, перезапуск которых оборвёт сеанс — нужна перезагрузка
    pub critical: Vec<String>,
    /// программы пользователя — перезапустить вручную или перелогиниться
    pub apps: Vec<String>,
    /// процессы с удалёнными библиотеками, cgroup которых не удалось отнести к службе или приложению
    pub unknown: Vec<String>,
}

const CRITICAL: &[&str] = &[
    "dbus", "dbus-broker", "systemd-logind", "gdm", "sddm", "lightdm", "display-manager", "systemd-journald", "polkit",
];

fn systemd_cgroup_path(cgroups: &str) -> Option<&str> {
    if let Some(path) = cgroups.lines().find_map(|line| line.strip_prefix("0::")).filter(|path| path.starts_with('/')) {
        return Some(path);
    }
    cgroups.lines().find_map(|line| {
        let mut fields = line.splitn(3, ':');
        let hierarchy = fields.next()?;
        let controllers = fields.next()?;
        let path = fields.next()?;
        (!hierarchy.is_empty()
            && hierarchy.bytes().all(|b| b.is_ascii_digit())
            && controllers.split(',').any(|controller| controller == "name=systemd")
            && path.starts_with('/'))
        .then_some(path)
    })
}

fn systemd_service(path: &str) -> Option<&str> {
    let unit = path.strip_prefix("/system.slice/")?.split('/').next()?;
    unit.strip_suffix(".service").filter(|stem| !stem.is_empty()).map(|_| unit)
}

fn process_label(comm: &str, pid: &str) -> String {
    if comm.is_empty() { format!("PID {pid}") } else { format!("{comm} (PID {pid})") }
}

pub fn needs_restart() -> Restart {
    let mut r = Restart::default();
    let Ok(rd) = fs::read_dir("/proc") else { return r };
    for e in rd.flatten() {
        let name = e.file_name();
        let Some(pid) = name.to_str().filter(|s| s.chars().all(|c| c.is_ascii_digit())) else { continue };
        let base = Path::new("/proc").join(pid);
        let exe_deleted = fs::read_link(base.join("exe")).map(|p| p.to_string_lossy().ends_with(" (deleted)")).unwrap_or(false);
        let libs_deleted = exe_deleted
            || fs::read_to_string(base.join("maps"))
                .map(|m| m.lines().any(|l| l.ends_with(" (deleted)") && l.contains(".so") && (l.contains(" /usr/") || l.contains(" /lib"))))
                .unwrap_or(false);
        if !libs_deleted {
            continue;
        }
        let cg = fs::read_to_string(base.join("cgroup")).unwrap_or_default();
        let path = systemd_cgroup_path(&cg);
        let comm = fs::read_to_string(base.join("comm")).unwrap_or_default().trim().to_string();
        if let Some(unit) = path.and_then(systemd_service) {
            let unit = unit.to_string();
            let stem = unit.trim_end_matches(".service");
            let target = if CRITICAL.iter().any(|c| stem == *c || stem.starts_with(&format!("{c}@"))) { &mut r.critical } else { &mut r.services };
            if !target.contains(&unit) {
                target.push(unit);
            }
        } else if path.map(|p| p == "/user.slice" || p.starts_with("/user.slice/")).unwrap_or(false) {
            if !comm.is_empty() && !r.apps.contains(&comm) {
                r.apps.push(comm.clone());
            } else if comm.is_empty() {
                let process = process_label(&comm, pid);
                if !r.unknown.contains(&process) {
                    r.unknown.push(process);
                }
            }
        } else if path == Some("/init.scope") || pid == "1" {
            if !r.critical.contains(&"systemd (PID 1)".to_string()) {
                r.critical.push("systemd (PID 1)".into());
            }
        } else {
            let process = process_label(&comm, pid);
            if !r.unknown.contains(&process) {
                r.unknown.push(process);
            }
        }
    }
    r.services.sort();
    r.critical.sort();
    r.apps.sort();
    r.unknown.sort();
    r
}

// ---------- os-release ----------

pub fn os_release() -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    for p in ["/etc/os-release", "/usr/lib/os-release"] {
        if let Ok(t) = fs::read_to_string(p) {
            for l in t.lines() {
                if let Some((k, v)) = l.split_once('=') {
                    m.insert(k.to_string(), v.trim_matches(|c| c == '"' || c == '\'').to_string());
                }
            }
            break;
        }
    }
    m.entry("PRETTY_NAME".into()).or_insert_with(|| "Linux".into());
    m
}

/// Изоляция тестов, меняющих окружение процесса; нужна и тестам бинарника, поэтому не под cfg(test).
#[doc(hidden)]
pub mod contract_fixtures {
    use std::ffi::{OsStr, OsString};
    use std::io;
    use std::path::Path;
    use std::path::PathBuf;
    use std::process::{Child, Command, ExitStatus};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Mutex, OnceLock};

    pub fn isolation_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    pub struct PathGuard {
        old: String,
    }

    impl Drop for PathGuard {
        fn drop(&mut self) {
            unsafe {
                std::env::set_var("PATH", &self.old);
            }
        }
    }

    /// Меняет PATH; вызывать только под `isolation_lock` или из `with_prepend_path`.
    pub fn prepend_path(bin_dir: &Path) -> PathGuard {
        let old = std::env::var("PATH").unwrap_or_default();
        unsafe {
            std::env::set_var("PATH", format!("{}:{}", bin_dir.display(), old));
        }
        PathGuard { old }
    }

    pub fn with_prepend_path<R>(bin_dir: &Path, f: impl FnOnce() -> R) -> R {
        let _g = isolation_lock();
        let _path = prepend_path(bin_dir);
        f()
    }

    /// Сохраняет только изменяемые переменные и восстанавливает их даже при panic.
    #[derive(Default)]
    pub struct EnvGuard {
        old: Vec<(OsString, Option<OsString>)>,
    }

    impl EnvGuard {
        pub fn new() -> Self {
            Self::default()
        }

        fn remember(&mut self, key: &OsStr) {
            if !self.old.iter().any(|(saved, _)| saved == key) {
                self.old.push((key.to_os_string(), std::env::var_os(key)));
            }
        }

        pub fn set(&mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) {
            let key = key.as_ref();
            self.remember(key);
            // Environment mutation is serialized by the shared isolation_lock in fixture tests.
            unsafe { std::env::set_var(key, value.as_ref()) };
        }

        pub fn remove(&mut self, key: impl AsRef<OsStr>) {
            let key = key.as_ref();
            self.remember(key);
            // Environment mutation is serialized by the shared isolation_lock in fixture tests.
            unsafe { std::env::remove_var(key) };
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in self.old.drain(..) {
                // Environment mutation is serialized by the shared isolation_lock in fixture tests.
                unsafe {
                    if let Some(value) = value {
                        std::env::set_var(key, value);
                    } else {
                        std::env::remove_var(key);
                    }
                }
            }
        }
    }

    /// Уникальный временный каталог, который удаляется и при раннем выходе или panic.
    pub struct TempDirGuard(PathBuf);

    impl TempDirGuard {
        pub fn new(prefix: &str) -> io::Result<Self> {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir();
            for _ in 0..100 {
                let n = NEXT.fetch_add(1, Ordering::Relaxed);
                let path = root.join(format!("{prefix}-{}-{n}", std::process::id()));
                match std::fs::create_dir(&path) {
                    Ok(()) => return Ok(Self(path)),
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(e) => return Err(e),
                }
            }
            Err(io::Error::new(io::ErrorKind::AlreadyExists, "could not allocate unique fixture directory"))
        }

        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Child test processes are killed if still running and always waited for on drop.
    pub struct ChildGuard(Option<Child>);

    impl ChildGuard {
        pub fn spawn(command: &mut Command) -> io::Result<Self> {
            command.spawn().map(|child| Self(Some(child)))
        }

        pub fn id(&self) -> u32 {
            self.0.as_ref().expect("child already reaped").id()
        }

        pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
            self.0.as_mut().expect("child already reaped").try_wait()
        }

        pub fn terminate(&mut self) -> io::Result<ExitStatus> {
            let mut child = self.0.take().expect("child already reaped");
            if let Some(status) = child.try_wait()? {
                return Ok(status);
            }
            match child.kill() {
                Ok(()) => child.wait(),
                Err(e) if e.kind() == io::ErrorKind::InvalidInput => child.wait(),
                Err(e) => Err(e),
            }
        }
    }

    impl Drop for ChildGuard {
        fn drop(&mut self) {
            if let Some(mut child) = self.0.take() {
                if !matches!(child.try_wait(), Ok(Some(_))) {
                    let _ = child.kill();
                }
                let _ = child.wait();
            }
        }
    }
}

#[cfg(test)]
mod contract_tests {
    use super::*;

    // --- NET-01 ---
    #[test]
    fn net01_ipv6_gateway_decodes() {
        assert_eq!(ipv6_route_address("20010db8000000000000000000000001"), Some("2001:db8::1".to_string()));
    }

    #[test]
    fn net01_ipv6_only_route_is_online() {
        let routes = vec![DefaultRoute {
            family: "IPv6",
            gateway: "2001:db8::1".into(),
            dev: "eth0".into(),
            metric: 100,
        }];
        let info = net_info_from_routes(routes);
        assert!(info.online);
        assert!(info.label.contains("IPv6"));
        assert!(info.label.contains("2001:db8::1"));
    }

    #[test]
    fn net01_gateway_change_changes_fingerprint_id() {
        let a = net_info_from_routes(vec![DefaultRoute {
            family: "IPv6",
            gateway: "2001:db8::1".into(),
            dev: "eth0".into(),
            metric: 0,
        }]);
        let b = net_info_from_routes(vec![DefaultRoute {
            family: "IPv6",
            gateway: "2001:db8::2".into(),
            dev: "eth0".into(),
            metric: 0,
        }]);
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn net01_no_default_routes_offline() {
        let info = net_info_from_routes(vec![]);
        assert!(!info.online);
        assert!(info.label.contains("нет сети"));
    }

    #[test]
    fn net01_parses_ipv6_default_from_fixture() {
        // flags 0x201 — UP, не expired; gateway 2001:db8::1; dev lo обычно поднят
        let fixture = "00000000000000000000000000000000 00 00000000000000000000000000000000 00 \
            20010db8000000000000000000000001 0000000000000000 0000000000000000 0000000000000064 00000001 lo";
        let route = select_default_ipv6_route_with(fixture, |_| true);
        assert_eq!(route.as_ref().map(|r| r.gateway.as_str()), Some("2001:db8::1"));
        assert_eq!(route.as_ref().map(|r| r.dev.as_str()), Some("lo"));
    }

    // --- SYS-01 ---
    #[test]
    fn sys01_systemd_cgroup_v2_path() {
        let cg = "0::/system.slice/ssh.service\n";
        assert_eq!(systemd_cgroup_path(cg), Some("/system.slice/ssh.service"));
    }

    #[test]
    fn sys01_systemd_cgroup_v1_path() {
        let cg = "9:devices:/user.slice\n2:name=systemd:/system.slice/cups.service\n";
        assert_eq!(systemd_cgroup_path(cg), Some("/system.slice/cups.service"));
    }

    #[test]
    fn sys01_systemd_service_from_cgroup() {
        assert_eq!(systemd_service("/system.slice/ssh.service"), Some("ssh.service"));
        assert_eq!(systemd_service("/user.slice/user-1000.slice"), None);
    }

    // --- SYS-02 ---
    #[test]
    fn sys02_module_dirs_has_no_duplicates() {
        let dirs = module_dirs();
        let mut seen = std::collections::HashSet::new();
        for d in dirs {
            assert!(seen.insert(d.clone()), "дубликат каталога модулей: {d:?}");
        }
    }

    #[test]
    fn sys02_lib_modules_layout_keeps_running_kernel() {
        let base = std::env::temp_dir().join(format!("cm-modules-{}", std::process::id()));
        let lib = base.join("lib").join("modules");
        let usr = base.join("usr").join("lib").join("modules");
        let release = "6.8.0-cm";
        fs::create_dir_all(lib.join(release)).unwrap();
        let only_lib = module_dirs_from(&[usr.clone(), lib.clone()]);
        assert_eq!(only_lib, vec![fs::canonicalize(&lib).unwrap()]);
        assert!(!running_modules_missing(release, &only_lib));
        assert!(running_modules_missing(release, std::slice::from_ref(&usr)));
        fs::create_dir_all(&usr).unwrap();
        std::os::unix::fs::symlink(&lib, usr.join("same")).ok();
        let linked = usr.join("link");
        let _ = fs::remove_dir_all(&linked);
        std::os::unix::fs::symlink(&lib, &linked).unwrap();
        let folded = module_dirs_from(&[lib.clone(), linked]);
        assert_eq!(folded.len(), 1);
        let _ = fs::remove_dir_all(&base);
    }

    // --- DATA-04 ---
    #[test]
    fn data04_unreadable_config_is_error_without_clobber() {
        let base = std::env::temp_dir().join(format!("cm-conf-{}", std::process::id()));
        fs::create_dir_all(&base).unwrap();
        let conf = base.join("cm.conf");
        fs::write(&conf, "keep=7\n").unwrap();
        let before = fs::read_to_string(&conf).unwrap();
        unsafe {
            std::env::set_var("CM_CONF", conf.to_str().unwrap());
        }
        fs::set_permissions(&conf, fs::Permissions::from_mode(0o000)).unwrap();
        assert!(Config::load(vec![]).is_err());
        fs::set_permissions(&conf, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(fs::read_to_string(&conf).unwrap(), before);
        let c = Config::load(vec![]).unwrap();
        assert_eq!(c.keep, 7);
        unsafe {
            std::env::remove_var("CM_CONF");
        }
        let _ = fs::remove_dir_all(&base);
    }

    // --- B07 ---
    #[test]
    fn b07_eof_is_refusal_even_with_default_yes() {
        let mut eof = std::io::Cursor::new(Vec::<u8>::new());
        assert!(!confirm_from(&mut eof, "q", true));
        assert!(!confirm_from(&mut std::io::Cursor::new(Vec::<u8>::new()), "q", false));
        assert!(confirm_from(&mut std::io::Cursor::new(b"\n".to_vec()), "q", true), "пустой Enter — ответ по умолчанию");
        assert!(!confirm_from(&mut std::io::Cursor::new(b"\n".to_vec()), "q", false));
        assert!(!confirm_from(&mut std::io::Cursor::new(b"n\n".to_vec()), "q", true));
        assert!(confirm_from(&mut std::io::Cursor::new(b"y\n".to_vec()), "q", false));
        // непонятный ответ, затем EOF — отказ
        assert!(!confirm_from(&mut std::io::Cursor::new(b"maybe\n".to_vec()), "q", true));
    }

    // --- B11 ---
    #[test]
    fn b11_command_output_is_limited() {
        let t0 = std::time::Instant::now();
        // бесконечный вывод: процесс останавливается на пределе, память не растёт
        let err = out_limited("sh", &["-c", "yes"], 1 << 20).unwrap_err();
        assert!(err.to_string().contains("sh"), "{err}");
        assert!(t0.elapsed() < std::time::Duration::from_secs(10));
        assert_eq!(out("sh", &["-c", "yes | head -c 100"]).0.len(), 100);
        let (s, code) = out_limited("sh", &["-c", "printf abc; exit 3"], 3).unwrap();
        assert_eq!((s.as_str(), code), ("abc", 3), "ровно предел — не ошибка");
        // stderr хранится хвостом: большой поток ошибок не держится в памяти целиком и не блокирует процесс
        let o = capture(Command::new("sh").args(["-c", "head -c 5000000 /dev/zero >&2; echo tail >&2; exit 1"]), None).unwrap();
        assert!(o.stderr.len() <= 64 << 10 && String::from_utf8_lossy(&o.stderr).ends_with("tail\n"));
        assert!(run(true, &[], "sh", &["-c", "echo boom >&2; exit 1"]).unwrap_err().contains("boom"));
    }

    // --- B08 / B25 ---
    #[test]
    fn b08_config_upper_bounds() {
        let _iso = contract_fixtures::isolation_lock();
        let base = contract_fixtures::TempDirGuard::new("cm-conf-bounds").unwrap();
        let conf = base.path().join("cm.conf");
        let mut text = String::from("parallel = 100000\nparallel_vpn = 99999999\nretries = 1000000000\ntimeout = 999999\nkeep = 500\nrescan_count = 100000\nextra_from_list = 100000\nmirror_max_age_h = 9223372036854775807\nvpn_sub_update_h = 9223372036854775807\nvpn_core_check_h = -5\nnetwork_memory_days = -3\nvpn_port = 1053\n");
        for i in 0..500 {
            text += &format!("mirror = https://m{i}.example/\n");
        }
        fs::write(&conf, text).unwrap();
        let mut env = contract_fixtures::EnvGuard::new();
        env.set("CM_CONF", &conf);
        let c = Config::load(vec![]);
        let c = c.expect("экстремальные значения не ломают загрузку конфига");
        assert_eq!((c.parallel, c.parallel_vpn, c.retries, c.timeout, c.keep), (MAX_PARALLEL, MAX_PARALLEL, MAX_RETRIES, MAX_TIMEOUT, 10));
        assert_eq!((c.rescan_count, c.extra_from_list, c.mirrors.len()), (MAX_MIRRORS, MAX_MIRRORS, MAX_MIRRORS));
        assert_eq!((c.mirror_max_age_h, c.vpn_sub_update_h, c.vpn_core_check_h, c.network_memory_days), (MAX_HOURS, MAX_HOURS, 1, 0));
        assert_eq!(c.vpn_port, 1053, "порт проверяет сборка VPN, не загрузка конфига");
        assert!(hours_secs(c.mirror_max_age_h) > 0);
    }
}

#[cfg(test)]
mod atomic_write_tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt, symlink};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Barrier};

    fn audit_temp_path(dir: &Path, n: usize) -> PathBuf {
        dir.join(format!(".cm-audit-{}-{n}.tmp", std::process::id()))
    }

    fn audit_temp_names() -> impl FnMut(&Path, usize) -> PathBuf {
        |dir, n| audit_temp_path(dir, n)
    }

    fn assert_no_audit_temps(dir: &Path) {
        let process_prefix = format!(".cm.{}.", std::process::id());
        let fixture_prefix = format!(".cm-audit-{}-", std::process::id());
        let names: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| {
                name.ends_with(".tmp") && (name.starts_with(&process_prefix) || name.starts_with(&fixture_prefix))
            })
            .collect();
        assert!(names.is_empty(), "temporary files remain: {names:?}");
    }

    #[test]
    fn atomic_write_16_concurrent_paths_keep_complete_content() {
        const WORKERS: usize = 16;
        let dir = contract_fixtures::TempDirGuard::new("cm-atomic-many-files").unwrap();
        let barrier = Arc::new(Barrier::new(WORKERS));
        let mut workers = Vec::with_capacity(WORKERS);
        for n in 0..WORKERS {
            let path = dir.path().join(format!("state-{n}"));
            let data = vec![b'A' + n as u8; 64 * 1024];
            let barrier = barrier.clone();
            workers.push(std::thread::spawn(move || {
                let result = atomic_write_with_hook(&path, &data, 0o600, || {
                    barrier.wait();
                });
                (result.is_err(), std::fs::read(path).is_ok_and(|actual| actual == data))
            }));
        }
        let results: Vec<_> = workers.into_iter().map(|worker| worker.join().unwrap()).collect();
        assert!(results.iter().all(|(error, matches)| !error && *matches), "{results:?}");
        assert_no_audit_temps(dir.path());
    }

    #[test]
    fn atomic_write_concurrent_same_path_never_exposes_mixed_content() {
        const WORKERS: usize = 16;
        let dir = contract_fixtures::TempDirGuard::new("cm-atomic-one-file").unwrap();
        let path = dir.path().join("state");
        fs::write(&path, b"initial").unwrap();
        let payloads: Vec<Vec<u8>> = (0..WORKERS).map(|n| vec![b'A' + n as u8; 64 * 1024]).collect();
        let mut allowed = payloads.clone();
        allowed.push(b"initial".to_vec());
        let allowed = Arc::new(allowed);
        let barrier = Arc::new(Barrier::new(WORKERS + 1));
        let done = Arc::new(AtomicBool::new(false));

        let reader_path = path.clone();
        let reader_allowed = allowed.clone();
        let reader_barrier = barrier.clone();
        let reader_done = done.clone();
        let reader = std::thread::spawn(move || {
            reader_barrier.wait();
            loop {
                let bytes = fs::read(&reader_path).unwrap();
                if !reader_allowed.contains(&bytes) {
                    return false;
                }
                if reader_done.load(Ordering::Acquire) {
                    return true;
                }
                std::thread::yield_now();
            }
        });

        let mut writers = Vec::with_capacity(WORKERS);
        for data in payloads {
            let path = path.clone();
            let barrier = barrier.clone();
            writers.push(std::thread::spawn(move || {
                atomic_write_with_hook(&path, &data, 0o600, || {
                    barrier.wait();
                })
            }));
        }
        for writer in writers {
            writer.join().unwrap().unwrap();
        }
        done.store(true, Ordering::Release);
        assert!(reader.join().unwrap(), "reader observed a partial or mixed version");
        assert!(allowed.contains(&fs::read(&path).unwrap()));
        assert_no_audit_temps(dir.path());
    }

    #[test]
    fn atomic_write_uses_exact_mode_and_replaces_symlink_entry() {
        let dir = contract_fixtures::TempDirGuard::new("cm-atomic-mode").unwrap();
        let target = dir.path().join("target");
        let link = dir.path().join("state-link");
        fs::write(&target, b"target content").unwrap();
        symlink(&target, &link).unwrap();

        atomic_write(&link, b"replacement", 0o600).unwrap();

        assert_eq!(fs::read(&target).unwrap(), b"target content");
        assert!(!fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(fs::read(&link).unwrap(), b"replacement");
        let metadata = fs::metadata(&link).unwrap();
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
        assert_eq!(metadata.gid(), unsafe { libc::getegid() });
    }

    #[test]
    fn atomic_write_temp_symlink_collision_does_not_follow_target() {
        let dir = contract_fixtures::TempDirGuard::new("cm-atomic-temp-link").unwrap();
        let target = dir.path().join("sensitive");
        let temp_link = dir.path().join(".temp-link");
        let output = dir.path().join("output");
        fs::write(&target, b"untouched").unwrap();
        symlink(&target, &temp_link).unwrap();

        atomic_write_with_ops(
            &output,
            b"new output",
            0o600,
            AtomicWriteOps {
                after_create: || {},
                write_data: |file: &mut fs::File, data: &[u8]| file.write_all(data),
                rename_file: |from: &Path, to: &Path| fs::rename(from, to),
                sync_parent: sync_directory,
                temp_path: |parent: &Path, attempt: usize| -> PathBuf {
                    if attempt == 0 { temp_link.clone() } else { parent.join(format!(".safe-{attempt}.tmp")) }
                },
            },
        )
        .unwrap();

        assert_eq!(fs::read(&target).unwrap(), b"untouched");
        assert!(fs::symlink_metadata(&temp_link).unwrap().file_type().is_symlink());
        assert_eq!(fs::read(&output).unwrap(), b"new output");
        assert!(!dir.path().join(".safe-1.tmp").exists());
    }

    #[test]
    fn atomic_write_failure_before_rename_preserves_target_and_cleans_temp() {
        let dir = contract_fixtures::TempDirGuard::new("cm-atomic-write-fail").unwrap();
        let path = dir.path().join("state");
        fs::write(&path, b"old content").unwrap();
        let error = atomic_write_with_ops(
            &path,
            b"new content",
            0o600,
            AtomicWriteOps {
                after_create: || {},
                write_data: |_: &mut fs::File, _: &[u8]| Err(std::io::Error::other("injected write failure")),
                rename_file: |from: &Path, to: &Path| fs::rename(from, to),
                sync_parent: sync_directory,
                temp_path: audit_temp_names(),
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("write temporary file"), "{error}");
        assert_eq!(fs::read(&path).unwrap(), b"old content");
        assert_no_audit_temps(dir.path());
    }

    #[test]
    fn atomic_write_rename_failure_preserves_target_and_cleans_temp() {
        let dir = contract_fixtures::TempDirGuard::new("cm-atomic-rename-fail").unwrap();
        let path = dir.path().join("state-dir");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("marker"), b"old content").unwrap();
        let error = atomic_write_with_hook(&path, b"new content", 0o600, || {}).unwrap_err();
        assert!(error.to_string().contains("rename temporary file"), "{error}");
        assert_eq!(fs::read(path.join("marker")).unwrap(), b"old content");
        assert_no_audit_temps(dir.path());
    }

    #[test]
    fn atomic_write_directory_sync_error_reports_commit_stage() {
        let dir = contract_fixtures::TempDirGuard::new("cm-atomic-sync-fail").unwrap();
        let path = dir.path().join("state");
        fs::write(&path, b"old content").unwrap();
        let error = atomic_write_with_ops(
            &path,
            b"new content",
            0o600,
            AtomicWriteOps {
                after_create: || {},
                write_data: |file: &mut fs::File, data: &[u8]| file.write_all(data),
                rename_file: |from: &Path, to: &Path| fs::rename(from, to),
                sync_parent: |_: &Path| Err(std::io::Error::other("injected directory sync failure")),
                temp_path: audit_temp_names(),
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("sync parent directory after rename"), "{error}");
        assert_eq!(fs::read(&path).unwrap(), b"new content", "rename has committed despite the later sync failure");
        assert_no_audit_temps(dir.path());
    }
}

#[cfg(test)]
mod setting_schema_tests {
    use super::*;
    #[test]
    fn every_numeric_setting_accepts_schema_bounds_and_rejects_outside() {
        for key in Config::keys().filter(|key| *key != "lang") {
            let schema = number_setting(key).unwrap_or_else(|| panic!("missing schema for {key}"));
            let mut config = Config::defaults(vec![]);
            for value in [schema.min, schema.max] {
                config.set(key, &value.to_string()).unwrap();
                assert_eq!(config.get(key), Some(value), "{key}");
            }
            assert!(config.set(key, &(schema.min - 1).to_string()).is_err(), "{key}");
            assert!(config.set(key, &(schema.max + 1).to_string()).is_err(), "{key}");
            assert_eq!(schema.step, 1);
        }
        assert_eq!(number_setting("vpn_sub_update_h").unwrap().unit, SettingUnit::Hours);
    }
}

#[cfg(test)]
mod pty_cloexec_tests {
    use super::*;
    use std::os::fd::AsRawFd;
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn fd_is_cloexec(fd: std::os::fd::RawFd) -> bool {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        flags >= 0 && flags & libc::FD_CLOEXEC != 0
    }

    fn inherited_pty_fds(pid: u32) -> Vec<String> {
        let mut found = Vec::new();
        let dir = match std::fs::read_dir(format!("/proc/{pid}/fd")) {
            Ok(dir) => dir,
            Err(_) => return found,
        };
        for entry in dir.flatten() {
            if let Ok(target) = std::fs::read_link(entry.path()) {
                let text = target.to_string_lossy();
                if text.contains("/dev/pts/") || text.contains("/dev/ptmx") {
                    found.push(text.into_owned());
                }
            }
        }
        found
    }

    fn wait_until_exec(pid: u32, name: &str) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
            if comm.trim() == name {
                return;
            }
            assert!(Instant::now() < deadline, "pid {pid} did not exec {name}, comm={comm:?}");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn parallel_spawn_during_pty_open_does_not_inherit_pty() {
        let stop = Arc::new(AtomicBool::new(false));
        let opener_stop = stop.clone();
        let opener = std::thread::spawn(move || {
            let mut held = open_pty_pair(24, 80).expect("pty");
            while !opener_stop.load(Ordering::Acquire) {
                held = open_pty_pair(24, 80).expect("pty");
            }
            held
        });
        std::thread::sleep(Duration::from_millis(30));
        let mut child = Command::new("sleep")
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("sleep");
        let pid = child.id();
        let check = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            wait_until_exec(pid, "sleep");
            let leaked = inherited_pty_fds(pid);
            assert!(leaked.is_empty(), "parallel child inherited pty fds: {leaked:?}");
        }));
        stop.store(true, Ordering::Release);
        let (master, slave) = opener.join().expect("opener");
        assert!(fd_is_cloexec(master.as_raw_fd()));
        assert!(fd_is_cloexec(slave.as_raw_fd()));
        let copied = dup_cloexec(&slave).expect("dup");
        assert!(fd_is_cloexec(copied.as_raw_fd()));
        let _ = child.kill();
        let _ = child.wait();
        if let Err(payload) = check {
            std::panic::resume_unwind(payload);
        }
    }
}
