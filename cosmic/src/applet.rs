//! Индикатор состояния cm на панели; нажатие открывает TUI.
use crate::model::{self, Prefs};
use crate::jobs::{Jobs, Kind, Completion, Phase};
use cosmic::app::{Core, Task};
use cosmic::iced::Subscription;
use cosmic::{Element, applet};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use cm::summary::{self, Badge, OpStatus, Summary};
use cm::t;
#[cfg(test)]
use cm::vpn;

pub struct Applet {
    jobs: Jobs<Message>,
    core: Core,
    summary: Summary,
    op: OpStatus,
    loaded: bool,
    tui_running: bool,
    error: String,
    prefs: Prefs,
    stamp: u64,
    summary_at: Option<Instant>,
    watching: Option<String>,
    lang: cm::i18n::Lang,
}

#[derive(Clone, Debug)]
pub enum Message {
    Probe(Completion<Message>),
    Metadata(model::Metadata),
    Surface(cosmic::surface::Action),
    Open,
    Tick,
    Summary(Box<Summary>),
    Op(OpStatus),
}

impl cosmic::Application for Applet {
    type Executor = cosmic::executor::Default;
    type Flags = ();
    type Message = Message;
    const APP_ID: &'static str = model::APPLET_ID;
    fn core(&self) -> &Core { &self.core }
    fn core_mut(&mut self) -> &mut Core { &mut self.core }
    fn init(core: Core, _: ()) -> (Self, Task<Message>) {
        let lang = model::init_lang();
        if let Some(f) = summary::applet_pid_file() {
            let _ = std::fs::write(f, std::process::id().to_string());
        }
        let mut app = Self {
            jobs: Jobs::default(), core, summary: Summary::default(),
            op: OpStatus::default(), loaded: false, tui_running: summary::tui_running(),
            error: String::new(), prefs: model::load_prefs(), stamp: 0,
            summary_at: None, watching: None, lang,
        };
        let tasks = Task::batch([
            app.probe(Kind::Summary, "", || model::load_summary().map(|s| Message::Summary(Box::new(s)))),
            app.probe(Kind::Operation, "", || model::load_op(true).map(Message::Op)),
        ]);
        (app, tasks)
    }
    fn subscription(&self) -> Subscription<Message> {
        cosmic::iced::time::every(self.poll_interval()).map(|_| Message::Tick)
    }
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Surface(a) => return cosmic::task::message(cosmic::Action::Cosmic(cosmic::app::Action::Surface(a))),
            Message::Open => {
                match model::open_tui() {
                    Ok(()) => self.error.clear(),
                    Err(error) => {
                        self.error = error.clone();
                        notify(t!("cm: ошибка").into(), error, vec![]);
                    }
                }
            }
            Message::Probe(completion) => {
                let (accept, repeat) = self.jobs.complete(&completion, Message::Probe);
                if accept {
                    match completion.result {
                        Ok(message) => return Task::batch([self.update(*message), repeat]),
                        Err(_) => {},
                    }
                }
                return repeat;
            }
            Message::Tick => return self.probe(Kind::Metadata, "", || Ok(Message::Metadata(model::metadata()))),
            Message::Metadata(meta) => {
                if !meta.launch_errors.is_empty() { self.error = meta.launch_errors.join("\n"); }
                self.lang = meta.lang;
                self.tui_running = meta.tui_running;
                let mut tasks = vec![];
                let stale = self.summary_at.is_none_or(|t| t.elapsed() > Duration::from_secs(120)) || matches!(self.jobs.phase(Kind::Summary), Phase::Error(_));
                if meta.stamp != self.stamp || stale {
                    self.stamp = meta.stamp;
                    self.summary_at = Some(Instant::now());
                    tasks.push(self.probe(Kind::Summary, "", || model::load_summary().map(|s| Message::Summary(Box::new(s)))));
                }
                let follow = self.op.running || self.watching.is_some();
                let retry = matches!(self.jobs.phase(Kind::Operation), Phase::Error(_));
                if follow || meta.busy || retry {
                    tasks.push(self.probe(Kind::Operation, "", move || model::load_op(follow || retry).map(Message::Op)));
                }
                return Task::batch(tasks);
            }
            Message::Summary(s) => {
                let first = !self.loaded;
                self.loaded = true;
                let old = std::mem::replace(&mut self.summary, *s);
                if !first {
                    self.notify_changes(&old);
                } else {
                    self.notify_changes(&Summary::default());
                }
            }
            Message::Op(op) => {
                if op.running && self.watching.is_none() && !op.command.is_empty() && op.command != "auto" {
                    self.watching = Some(op.command.clone());
                }
                if !op.running {
                    if let Some(cmd) = self.watching.take() {
                        if op.last_command == cmd {
                            self.notify_finished(&cmd, op.last_exit.unwrap_or(1));
                        }
                    }
                }
                let finished = self.op.running && !op.running;
                self.op = op;
                if finished {
                    return self.probe(Kind::Summary, "", || model::load_summary().map(|s| Message::Summary(Box::new(s))));
                }
            }
        }
        Task::none()
    }
    fn view(&self) -> Element<'_, Message> {
        let errors = self.jobs.errors().map(|(kind, error)| format!("{}: {error}", kind.label())).collect::<Vec<_>>().join("\n");
        let badge = crate::panel::activity_badge(summary::badge(&self.summary, &self.op), self.tui_running, !self.error.is_empty() || !errors.is_empty());
        let horizontal = self.core.applet.is_horizontal();
        let (size, _) = self.core.applet.suggested_size(true);
        let padding = self.core.applet.suggested_padding(true);
        let padding = if horizontal { padding } else { (padding.1, padding.0) };
        let btn = crate::panel::button(&badge, size, padding, horizontal).on_press(Message::Open);
        let title = if !self.error.is_empty() { self.error.clone() }
            else if self.tui_running && !self.op.running && matches!(badge, Badge::Busy(_)) { t!("работает").to_string() }
            else if matches!(badge, Badge::Waiting) { t!("Ожидание ответа").to_string() }
            else { summary::headline(&self.summary, &self.op).title() };
        let mut tip = format!("cm — {title}\n{}\n{}", summary::checked_line(&self.summary), t!("Открыть cm…"));
        if !self.summary.vpn.error.is_empty() { tip.push_str(&format!("\nVPN: {}", self.summary.vpn.error)); }
        if !errors.is_empty() { tip.push_str(&format!("\n{errors}")); }
        self.core.applet.applet_tooltip(btn, tip, false, Message::Surface, None).into()
    }
    fn style(&self) -> Option<cosmic::iced::theme::Style> { Some(applet::style()) }
}
impl Applet {
    fn poll_interval(&self) -> Duration {
        Duration::from_secs(if self.op.running || self.watching.is_some() || self.tui_running { 1 } else { 10 })
    }
    fn probe(&mut self, kind: Kind, key: &str, work: impl FnOnce() -> Result<Message, String> + Send + 'static) -> Task<Message> {
        self.jobs.request(kind, key.to_owned(), work, Message::Probe)
    }
    #[cfg(test)]
    pub fn demo(summary: Summary, op: OpStatus, _: Option<vpn::Snapshot>, _: bool) -> Self {
        Self { jobs: Jobs::default(), core: Core::default(), summary, op,
            loaded: true, tui_running: false, error: String::new(), prefs: Prefs::default(),
            stamp: 0, summary_at: None, watching: None, lang: cm::i18n::cur() }
    }
    // ---------- уведомления ----------

    fn notify_changes(&mut self, old: &Summary) {
        if !self.prefs.notifications {
            return;
        }
        let s = &self.summary;
        let mut seen = notified_load();
        let total = s.total();
        if total > 0 && (old.total() != total || old.packages != s.packages) {
            let key = format!("{:x}", {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                (&s.packages, &s.flatpak, &s.firmware).hash(&mut h);
                h.finish()
            });
            if seen.get("updates") != Some(&key) {
                seen.insert("updates".into(), key);
                let body = model::updates_breakdown(s).iter().map(|(n, c)| format!("{n}: {c}")).collect::<Vec<_>>().join(" · ");
                notify(
                    t!("Доступно обновлений: {0}", total),
                    body,
                    vec![("install", t!("Установить…").to_string()), ("later", t!("Позже").to_string())],
                );
            }
        }
        if s.reboot {
            let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap_or_default();
            if seen.get("reboot") != Some(&boot) {
                seen.insert("reboot".into(), boot);
                notify(t!("Нужна перезагрузка").into(), t!("Обновилось ядро").into(), vec![]);
            }
        }
        if let Some(n) = s.news.first() {
            if seen.get("news") != Some(&n.title) {
                seen.insert("news".into(), n.title.clone());
                notify(t!("Новости Arch перед обновлением").into(), n.title.clone(), vec![("news", t!("Прочитать").to_string())]);
            }
        }
        notified_save(&seen);
    }

    fn notify_finished(&self, command: &str, code: i32) {
        if !self.prefs.notifications || command == "check" && code == 0 {
            return;
        }
        let what = summary::op_title(command);
        let (title, body) = if code == 0 { (t!("{0}: готово", what), String::new()) } else { (t!("{0}: ошибка", what), t!("Код завершения {0}", code)) };
        notify(title, body, vec![("log", t!("Открыть журнал").to_string())]);
    }
}

fn notified_path() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".cache/cm-notified.json")
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    use cosmic::Application;
    use cosmic::iced::futures::StreamExt;
    #[test]
    fn idle_applet_recovers_from_initial_helper_error() {
        use cm::common::contract_fixtures::{isolation_lock, TempDirGuard, EnvGuard};
        let _isolation = isolation_lock();
        let dir = TempDirGuard::new("cosmic-idle-applet-retry").unwrap();
        let mut env = EnvGuard::new(); env.set("CM_HELPER_SOCK", dir.path().join("absent.sock"));
        env.set("CM_STATE_DIR", dir.path());
        let mut app = Applet::demo(Summary::default(), OpStatus::default(), None, false);
        app.summary_at = Some(Instant::now());
        app.jobs.demo_phase(Kind::Operation, Phase::Error("old helper protocol".into()));
        let stamp = app.stamp;
        let task = <Applet as Application>::update(&mut app, Message::Metadata(model::Metadata {
            busy: false, stamp, lang: cm::i18n::Lang::Ru, launch_errors: vec![], tui_running: false,
        }));
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(async {
            let mut stream = cosmic::iced::runtime::task::into_stream(task).expect("idle helper retry must be scheduled");
            while let Some(action) = stream.next().await {
                if let cosmic::iced::runtime::Action::Output(cosmic::Action::App(message)) = action {
                    drop(<Applet as Application>::update(&mut app, message));
                }
            }
        });
        assert!(matches!(app.jobs.phase(Kind::Operation), Phase::Ready));
        assert!(!app.op.running);
    }
}

/// Общий с `cm notify` учёт показанного: одно и то же не показывается дважды.
fn notified_load() -> BTreeMap<String, String> {
    std::fs::read(notified_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn notified_save(m: &BTreeMap<String, String>) {
    let p = notified_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(p, serde_json::to_vec(m).unwrap_or_default());
}

/// Уведомление с кнопками: notify-send ждёт выбора в отдельном потоке.
fn notify(title: String, body: String, actions: Vec<(&'static str, String)>) {
    if !cm::common::have("notify-send") {
        return;
    }
    let mut c = std::process::Command::new("notify-send");
    c.args(["-a", "cm", "-i", model::APP_ID]);
    for (k, label) in &actions { c.arg(format!("--action={k}={label}")); }
    c.arg(&title).arg(&body);
    let result = crate::notifications::send(c, |choice| {
        let result = match choice {
            crate::notifications::Action::Install => model::open_tui(),
            crate::notifications::Action::News => model::open_tui(),
            crate::notifications::Action::Log => model::open_tui(),
        };
        if let Err(error) = result { crate::launch::record_error(error); }
    });
    if let Err(error) = result { crate::launch::record_error(error.to_string()); }
}
