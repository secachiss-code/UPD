//! Host information and one-second resource sampling without external commands.
use cm::{common::fmt_bytes, t};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use std::{
    collections::BTreeMap,
    fs,
    time::{Duration, Instant},
};

fn read(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_default()
}
fn clean(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(160)
        .collect::<String>()
        .trim()
        .to_string()
}
fn field(text: &str, key: &str) -> String {
    text.lines()
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.trim() == key)
        .map(|(_, v)| clean(v))
        .unwrap_or_default()
}
fn cpu(text: &str) -> Option<(u64, u64)> {
    let mut fields = text
        .lines()
        .find(|l| l.starts_with("cpu "))?
        .split_whitespace()
        .skip(1);
    // guest/guest_nice are already included in user/nice.
    let values: Vec<u64> = fields
        .by_ref()
        .take(8)
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    if values.len() < 4 {
        return None;
    }
    Some((
        values.iter().sum(),
        values[3] + values.get(4).copied().unwrap_or(0),
    ))
}
fn cpu_percent(old: (u64, u64), new: (u64, u64)) -> Option<f64> {
    let total = new.0.checked_sub(old.0)?;
    let idle = new.1.checked_sub(old.1)?;
    (total > 0 && idle <= total).then(|| 100.0 * (total - idle) as f64 / total as f64)
}
fn memory(text: &str) -> Option<(u64, u64, u64, u64)> {
    let values: BTreeMap<_, _> = text
        .lines()
        .filter_map(|line| {
            let (key, rest) = line.split_once(':')?;
            Some((
                key,
                rest.split_whitespace()
                    .next()?
                    .parse::<u64>()
                    .ok()?
                    .saturating_mul(1024),
            ))
        })
        .collect();
    let total = *values.get("MemTotal")?;
    let available = *values.get("MemAvailable")?;
    let swap = *values.get("SwapTotal")?;
    Some((
        total.saturating_sub(available),
        total,
        swap.saturating_sub(*values.get("SwapFree")?),
        swap,
    ))
}
fn net(text: &str) -> BTreeMap<String, (u64, u64)> {
    text.lines()
        .filter_map(|line| {
            let (name, rest) = line.split_once(':')?;
            let name = name.trim();
            if name == "lo" {
                return None;
            }
            let columns: Vec<_> = rest.split_whitespace().collect();
            Some((
                clean(name),
                (
                    columns.first()?.parse().ok()?,
                    columns.get(8)?.parse().ok()?,
                ),
            ))
        })
        .collect()
}
fn filesystem() -> Option<(u64, u64)> {
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(c"/".as_ptr(), stat.as_mut_ptr()) } != 0 {
        return None;
    }
    let stat = unsafe { stat.assume_init() };
    let size = stat.f_frsize;
    Some((
        stat.f_blocks
            .saturating_sub(stat.f_bfree)
            .saturating_mul(size),
        stat.f_blocks.saturating_mul(size),
    ))
}
fn gpu() -> String {
    let mut cards = Vec::new();
    for entry in fs::read_dir("/sys/class/drm")
        .into_iter()
        .flatten()
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name
            .strip_prefix("card")
            .is_some_and(|suffix| !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()))
        {
            continue;
        }
        let device = entry.path().join("device");
        let vendor = fs::read_to_string(device.join("vendor")).unwrap_or_default();
        let vendor = match vendor.trim() {
            "0x1002" => "AMD",
            "0x10de" => "NVIDIA",
            "0x8086" => "Intel",
            other => other,
        };
        let id = fs::read_to_string(device.join("device")).unwrap_or_default();
        let driver = fs::read_link(device.join("driver"))
            .ok()
            .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_default();
        cards.push(format!("{vendor} {} {driver}", id.trim()));
    }
    if cards.is_empty() {
        "—".into()
    } else {
        cards.join(" · ")
    }
}

pub struct Monitor {
    specs: Vec<(String, String)>,
    lang: cm::i18n::Lang,
    sampled: Instant,
    previous_cpu: Option<(u64, u64)>,
    previous_net: BTreeMap<String, (u64, u64)>,
    usage: Option<f64>,
    metrics: Vec<(String, String)>,
}
impl Monitor {
    pub fn new() -> Self {
        let cpuinfo = read("/proc/cpuinfo");
        let cpu_name = ["model name", "Hardware", "Processor"]
            .iter()
            .map(|key| field(&cpuinfo, key))
            .find(|s| !s.is_empty())
            .unwrap_or_else(|| std::env::consts::ARCH.into());
        let threads = cpuinfo
            .lines()
            .filter(|line| line.starts_with("processor\t") || line.starts_with("processor "))
            .count();
        let os = read("/etc/os-release")
            .lines()
            .find_map(|line| {
                line.strip_prefix("PRETTY_NAME=")
                    .map(|s| clean(s.trim_matches('"')))
            })
            .unwrap_or_default();
        let mut monitor = Self {
            lang: cm::i18n::cur(),
            specs: vec![
                (t!("Хост").into(), clean(&read("/proc/sys/kernel/hostname"))),
                (
                    t!("Модель").into(),
                    clean(&format!(
                        "{} {}",
                        read("/sys/class/dmi/id/sys_vendor").trim(),
                        read("/sys/class/dmi/id/product_name").trim()
                    )),
                ),
                (t!("Система").into(), os),
                (
                    t!("Ядро").into(),
                    format!(
                        "{} · {}",
                        clean(&read("/proc/sys/kernel/osrelease")),
                        std::env::consts::ARCH
                    ),
                ),
                (
                    "CPU".into(),
                    format!("{cpu_name} · {threads} {}", t!("потоков")),
                ),
                ("GPU".into(), gpu()),
            ],
            sampled: Instant::now(),
            previous_cpu: cpu(&read("/proc/stat")),
            previous_net: net(&read("/proc/net/dev")),
            usage: None,
            metrics: vec![],
        };
        monitor.sample(Duration::ZERO);
        monitor
    }
    pub fn refresh(&mut self) {
        if self.lang != cm::i18n::cur() {
            *self = Self::new();
        }
        let elapsed = self.sampled.elapsed();
        if elapsed < Duration::from_secs(1) {
            return;
        }
        self.sample(elapsed);
        self.sampled = Instant::now();
    }
    fn sample(&mut self, elapsed: Duration) {
        if !elapsed.is_zero() {
            let current = cpu(&read("/proc/stat"));
            self.usage = self
                .previous_cpu
                .zip(current)
                .and_then(|(old, new)| cpu_percent(old, new));
            self.previous_cpu = current;
        }
        self.metrics.clear();
        self.metrics.push((
            t!("Загрузка CPU").into(),
            self.usage
                .map(|v| format!("{v:.1}%"))
                .unwrap_or_else(|| "—".into()),
        ));
        self.metrics.push((
            t!("Нагрузка 1/5/15").into(),
            read("/proc/loadavg")
                .split_whitespace()
                .take(3)
                .collect::<Vec<_>>()
                .join(" · "),
        ));
        if let Some((used, total, swap_used, swap_total)) = memory(&read("/proc/meminfo")) {
            self.metrics.push((
                t!("Память").into(),
                format!(
                    "{} / {} · {:.0}%",
                    fmt_bytes(used),
                    fmt_bytes(total),
                    if total > 0 {
                        100.0 * used as f64 / total as f64
                    } else {
                        0.0
                    }
                ),
            ));
            self.metrics.push((
                "Swap".into(),
                format!("{} / {}", fmt_bytes(swap_used), fmt_bytes(swap_total)),
            ));
        }
        if let Some((used, total)) = filesystem() {
            self.metrics.push((
                t!("Диск /").into(),
                format!("{} / {}", fmt_bytes(used), fmt_bytes(total)),
            ));
        }
        let current_net = net(&read("/proc/net/dev"));
        for (name, (rx, tx)) in &current_net {
            let rates = if elapsed.is_zero() {
                None
            } else {
                self.previous_net.get(name).and_then(|(old_rx, old_tx)| {
                    Some((
                        rx.checked_sub(*old_rx)? as f64 / elapsed.as_secs_f64(),
                        tx.checked_sub(*old_tx)? as f64 / elapsed.as_secs_f64(),
                    ))
                })
            };
            let text = rates
                .map(|(rx, tx)| {
                    format!(
                        "↓ {}/{}  ↑ {}/{}",
                        fmt_bytes(rx as u64),
                        t!("с"),
                        fmt_bytes(tx as u64),
                        t!("с")
                    )
                })
                .unwrap_or_else(|| "—".into());
            self.metrics.push((name.clone(), text));
        }
        self.previous_net = current_net;
        let uptime = read("/proc/uptime")
            .split_whitespace()
            .next()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0) as u64;
        self.metrics.push((
            t!("Время работы").into(),
            format!(
                "{} {} {} {}",
                uptime / 3600,
                t!("ч"),
                uptime % 3600 / 60,
                t!("мин")
            ),
        ));
    }
    pub fn compact_lines(&self) -> Vec<Line<'static>> {
        let value = |key: &str| {
            self.metrics
                .iter()
                .find(|(label, _)| label == key)
                .map(|(_, value)| value.as_str())
                .unwrap_or("—")
        };
        vec![
            Line::raw(format!("{} · {}", self.specs[0].1, self.specs[2].1)),
            Line::raw(format!(
                "CPU {} · {} {}",
                value(t!("Загрузка CPU")),
                t!("Память"),
                value(t!("Память"))
            )),
            Line::raw(format!("{} {}", t!("Диск /"), value(t!("Диск /")))),
        ]
    }
    pub fn lines(&self) -> Vec<Line<'static>> {
        self.specs
            .iter()
            .chain(&self.metrics)
            .map(|(label, value)| {
                Line::from(vec![
                    Span::styled(format!("{label:<16} "), Style::default().fg(Color::Cyan)),
                    Span::raw(if value.is_empty() {
                        "—".into()
                    } else {
                        value.clone()
                    }),
                ])
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cpu_uses_deltas_and_excludes_guest_double_counting() {
        assert_eq!(cpu("cpu 10 0 10 70 10 0 0 0 5 0\n"), Some((100, 80)));
        assert_eq!(cpu_percent((100, 80), (200, 100)), Some(80.0));
        assert_eq!(cpu_percent((100, 80), (100, 80)), None);
        assert_eq!(cpu_percent((100, 80), (90, 70)), None);
    }
    #[test]
    fn memory_counts_available_cache_as_available_and_handles_no_swap() {
        assert_eq!(
            memory("MemTotal: 1000 kB\nMemAvailable: 600 kB\nSwapTotal: 0 kB\nSwapFree: 0 kB\n"),
            Some((400 * 1024, 1000 * 1024, 0, 0))
        );
        assert_eq!(memory("MemTotal: 1000 kB\n"), None);
    }
    #[test]
    fn network_tracks_interfaces_separately_and_excludes_loopback() {
        let counters = net("lo: 9 0 0 0 0 0 0 0 9\nwlp1s0: 1024 0 0 0 0 0 0 0 2048\n");
        assert_eq!(counters.len(), 1);
        assert_eq!(counters["wlp1s0"], (1024, 2048));
    }
}
