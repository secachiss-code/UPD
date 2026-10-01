//! Апплет на панели COSMIC: одна кнопка с одним значком по приоритету и всплывающее окно
//! «сначала состояние, одно главное действие, предупреждения — только когда есть».

use crate::model::{self, Prefs};
use crate::jobs::{Jobs, Kind, Completion, Phase};
use crate::ui::{self, hrow};
use cosmic::app::{Core, Task};
use cosmic::iced::window::Id;
use cosmic::iced::{Alignment, Length, Rectangle, Subscription, Vector};
use cosmic::surface::action::{app_popup, destroy_popup};
use cosmic::widget::{self, button, divider, text};
use cosmic::{Element, applet};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use upd::summary::{self, Badge, Headline, OpStatus, Summary};
use upd::{helper, t, vpn};

pub struct Applet {
    jobs: Jobs<Message>,
    core: Core,
    popup: Option<Id>,
    summary: Summary,
    op: OpStatus,
    loaded: bool,
    vpn: Option<vpn::Snapshot>,
    servers_open: bool,
    confirm_reboot: bool,
    /// идёт обращение к помощнику (в том числе ожидание пароля)
    pending: bool,
    /// ошибка последнего действия
    error: String,
    prefs: Prefs,
    /// подпись файлов состояния: изменилась — пора перечитать сводку
    stamp: u64,
    summary_at: Option<Instant>,
    /// операция, за которой следим, чтобы сообщить о её завершении
    watching: Option<String>,
    lang: upd::i18n::Lang,
}

#[derive(Clone, Debug)]
pub enum Message {
    Probe(Completion<Message>),
    Metadata(model::Metadata),
    Surface(cosmic::surface::Action),
    OpenPopup(Vector, Rectangle),
    PopupClosed(Id),
    Tick,
    Summary(Box<Summary>),
    Op(OpStatus),
    Vpn(Option<vpn::Snapshot>),
    Check,
    Install,
    Open(Option<&'static str>),
    VpnToggle(bool),
    ToggleServers,
    Select(String, String),
    Reboot,
    MirrorCheck,
    Done(Result<(), String>),
}

fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static, m: impl FnOnce(Result<T, String>) -> Message + Send + 'static) -> Task<Message> {
    cosmic::task::future(async move {
        let result = tokio::task::spawn_blocking(f).await.unwrap_or_else(|e| Err(e.to_string()));
        m(result)
    })
}

impl cosmic::Application for Applet {
    type Executor = cosmic::executor::Default;
    type Flags = ();
    type Message = Message;
    const APP_ID: &'static str = model::APPLET_ID;

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, _: ()) -> (Self, Task<Message>) {
        let lang = model::init_lang();
        // пока апплет работает, `upd notify` не дублирует уведомления
        if let Some(f) = summary::applet_pid_file() {
            let _ = std::fs::write(f, std::process::id().to_string());
        }
        let mut app = Applet {
            jobs: Jobs::default(),
            core,
            popup: None,
            summary: Summary::default(),
            op: OpStatus::default(),
            loaded: false,
            vpn: None,
            servers_open: false,
            confirm_reboot: false,
            pending: false,
            error: String::new(),
            prefs: model::load_prefs(),
            stamp: 0,
            summary_at: None,
            watching: None,
            lang,
        };
        let tasks = Task::batch([app.probe(Kind::Summary, "", || model::load_summary().map(|s| Message::Summary(Box::new(s)))), app.probe(Kind::Operation, "", || model::load_op(true).map(Message::Op))]);
        (app, tasks)
    }

    fn on_close_requested(&self, id: Id) -> Option<Message> {
        Some(Message::PopupClosed(id))
    }

    fn subscription(&self) -> Subscription<Message> {
        cosmic::iced::time::every(Duration::from_secs(3)).map(|_| Message::Tick)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Surface(a) => return cosmic::task::message(cosmic::Action::Cosmic(cosmic::app::Action::Surface(a))),
            Message::OpenPopup(offset, bounds) => {
                if let Some(id) = self.popup.take() {
                    self.jobs.invalidate(&[Kind::Vpn]);
                    return cosmic::task::message(cosmic::Action::Cosmic(cosmic::app::Action::Surface(destroy_popup(id))));
                }
                self.confirm_reboot = false;
                self.servers_open = false;
                self.error.clear();
                let open = app_popup::<Applet>(
                    |_| Default::default(),
                    move |state: &mut Applet| {
                        let id = Id::unique();
                        state.popup = Some(id);
                        let mut s = state.core.applet.get_popup_settings(state.core.main_window_id().unwrap_or(Id::RESERVED), id, None, None, None);
                        s.positioner.anchor_rect = Rectangle {
                            x: (bounds.x - offset.x) as i32,
                            y: (bounds.y - offset.y) as i32,
                            width: bounds.width as i32,
                            height: bounds.height as i32,
                        };
                        s
                    },
                    Some(Box::new(|state: &Applet| Element::from(state.core.applet.popup_container(state.popup_view())).map(cosmic::Action::App))),
                );
                let mut tasks = vec![
                    cosmic::task::message(cosmic::Action::Cosmic(cosmic::app::Action::Surface(open))),
                    self.probe(Kind::Summary, "", || model::load_summary().map(|s| Message::Summary(Box::new(s)))),
                ];
                if self.summary.vpn.active {
                    tasks.push(self.probe(Kind::Vpn, "popup", || model::vpn_snapshot().map(|s| Message::Vpn(Some(s)))));
                }
                return Task::batch(tasks);
            }
            Message::PopupClosed(id) => {
                if self.popup == Some(id) {
                    self.popup = None;
                    self.jobs.invalidate(&[Kind::Vpn]);
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
            Message::Vpn(v) => self.vpn = v,
            Message::Check => return self.run(&["check"]),
            Message::Install => {
                if let Err(error) = model::open_window(Some("updates"), Some("update")) { self.error = error; return Task::none(); }
                return self.close_popup();
            }
            Message::Open(page) => {
                if let Err(error) = model::open_window(page, None) { self.error = error; return Task::none(); }
                return self.close_popup();
            }
            Message::VpnToggle(on) => return self.run(&["vpn", if on { "start" } else { "stop" }]),
            Message::ToggleServers => {
                self.servers_open = !self.servers_open;
                if self.servers_open {
                    return self.probe(Kind::Vpn, "popup", || model::vpn_snapshot().map(|s| Message::Vpn(Some(s))));
                }
            }
            Message::Select(group, name) => {
                self.pending = true;
                return blocking(
                    move || helper::call(&helper::Request::VpnSelect { group, name }).map(|_| ()),
                    Message::Done,
                );
            }
            Message::Reboot => {
                if !self.confirm_reboot {
                    self.confirm_reboot = true;
                } else {
                    return blocking(model::reboot, Message::Done);
                }
            }
            Message::MirrorCheck => return self.run(&["mirrors", "check"]),
            Message::Done(r) => {
                self.pending = false;
                match r {
                    Ok(()) => {
                        self.error.clear();
                        let mut tasks = vec![self.probe(Kind::Operation, "", || model::load_op(true).map(Message::Op))];
                        if self.summary.vpn.active || self.servers_open {
                            tasks.push(self.probe(Kind::Vpn, "popup", || model::vpn_snapshot().map(|s| Message::Vpn(Some(s)))));
                        }
                        return Task::batch(tasks);
                    }
                    Err(e) => self.error = e,
                }
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let badge = summary::badge(&self.summary, &self.op);
        let horizontal = self.core.applet.is_horizontal();
        let (icon, label) = match &badge {
            Badge::Busy(Some((n, m))) => ("emblem-synchronizing-symbolic", Some(format!("{n}/{m}"))),
            Badge::Busy(None) => ("emblem-synchronizing-symbolic", None),
            Badge::Error => ("dialog-warning-symbolic", None),
            Badge::Reboot => ("system-reboot-symbolic", None),
            Badge::Updates(n) | Badge::Stale(n) => ("software-update-available-symbolic", Some(n.to_string())),
            Badge::Idle => (model::ICON, None),
        };
        let dim = matches!(badge, Badge::Stale(_));
        let open = |offset, bounds| Message::OpenPopup(offset, bounds);
        // на вертикальной панели число не помещается: остаётся значок, число — в подсказке
        let btn = match label.filter(|_| horizontal) {
            Some(label) => {
                let (w, h) = self.core.applet.suggested_size(true);
                let (_, vpad) = self.core.applet.suggested_padding(true);
                let mut t = self.core.applet.text(label);
                if dim {
                    t = t.class(cosmic::theme::Text::Custom(ui::dim_text));
                }
                let content = widget::row::with_capacity(2)
                    .push(widget::icon::from_name(icon).size(w).symbolic(true))
                    .push(t)
                    .spacing(4)
                    .align_y(Alignment::Center);
                button::custom(content)
                    .class(cosmic::theme::Button::AppletIcon)
                    .padding([0, vpad.max(4)])
                    .height(Length::Fixed(f32::from(h + 2 * vpad)))
                    .on_press_with_rectangle(open)
            }
            None => self.core.applet.icon_button(icon).on_press_with_rectangle(open),
        };
        let tip = format!("upd — {}\n{}", summary::headline(&self.summary, &self.op).title(), summary::checked_line(&self.summary));
        self.core.applet.applet_tooltip(btn, tip, self.popup.is_some(), Message::Surface, None).into()
    }

    fn view_window(&self, _id: Id) -> Element<'_, Message> {
        widget::text("").into()
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(applet::style())
    }
}

impl Applet {
    fn probe(&mut self, kind: Kind, key: &str, work: impl FnOnce() -> Result<Message, String> + Send + 'static) -> Task<Message> {
        self.jobs.request(kind, key.to_owned(), work, Message::Probe)
    }

    /// Апплет с заданным состоянием — для снимков интерфейса в тестах.
    #[cfg(test)]
    pub fn demo(summary: Summary, op: OpStatus, vpn: Option<vpn::Snapshot>, servers_open: bool) -> Self {
        Applet {
            jobs: Jobs::default(),
            core: Core::default(),
            popup: None,
            summary,
            op,
            loaded: true,
            vpn,
            servers_open,
            confirm_reboot: false,
            pending: false,
            error: String::new(),
            prefs: Prefs::default(),
            stamp: 0,
            summary_at: None,
            watching: None,
            lang: upd::i18n::cur(),
        }
    }

    #[cfg(test)]
    pub fn demo_case(&mut self, case: &str) {
        match case {
            "loading" => self.jobs.demo_phase(Kind::Summary, Phase::Loading),
            "denied" => self.error = "polkit: authorization denied".into(),
            "long-error" => self.error = "Сеть недоступна / connection refused: https://example.invalid/long/".repeat(20),
            "stale" => self.summary.checked = upd::common::now().saturating_sub(90000),
            _ => {}
        }
    }

    #[cfg(test)]
    pub fn demo_popup(&self) -> Element<'_, Message> {
        self.popup_view()
    }

    fn close_popup(&mut self) -> Task<Message> {
        self.jobs.invalidate(&[Kind::Vpn]);
        match self.popup.take() {
            Some(id) => cosmic::task::message(cosmic::Action::Cosmic(cosmic::app::Action::Surface(destroy_popup(id)))),
            None => Task::none(),
        }
    }

    /// Команда через помощника; окно пароля (если нужно) покажет агент polkit.
    fn run(&mut self, args: &[&'static str]) -> Task<Message> {
        self.pending = true;
        self.error.clear();
        self.watching = Some(args.join(" "));
        let args: Vec<&'static str> = args.to_vec();
        blocking(move || model::start(&args).map(|_| ()), Message::Done)
    }

    fn can_act(&self) -> bool {
        !self.pending && !self.op.running && helper::available()
    }

    fn popup_view(&self) -> Element<'_, Message> {
        let sp = cosmic::theme::spacing();
        let s = &self.summary;
        let head = summary::headline(s, &self.op);
        let mut col = widget::column::with_capacity(12).padding([sp.space_xs, 0]);

        if matches!(self.jobs.phase(Kind::Summary), Phase::Loading) { col = col.push(text(t!("Загрузка…"))); }

        // заголовок: итог и проверка
        let refresh = button::icon(widget::icon::from_name("view-refresh-symbolic"))
            .tooltip(t!("Проверить сейчас"))
            .on_press_maybe(self.can_act().then_some(Message::Check));
        let title = widget::column::with_capacity(2)
            .push(ui::txt(text::title4(head.title())))
            .push(ui::txt(text::caption(summary::checked_line(s))))
            .width(Length::Fill);
        col = col.push(applet::padded_control(hrow(vec![title.into(), refresh.into()]).align_y(Alignment::Center)));

        for (kind, error) in self.jobs.errors() {
            col = col.push(ui::banner("dialog-warning-symbolic", format!("{}: {error} · {}", kind.label(), t!("Показаны предыдущие данные")), None));
        }
        if !helper::available() {
            col = col.push(ui::banner("dialog-warning-symbolic", t!("Помощник upd не установлен — действия недоступны (sudo upd install)").into(), None));
        }
        if !self.error.is_empty() {
            col = col.push(ui::banner("dialog-warning-symbolic", model::ellipsize(&self.error, 160), None));
        }

        match &head {
            Headline::Installing { .. } | Headline::Checking => {
                let label = if self.op.waiting { t!("Ждёт ответа — показать") } else { t!("Показать ход") };
                let bar = widget::progress_bar::linear::Linear::new().width(Length::Fill);
                // этап известен — полоса показывает долю, иначе — бегущая
                let bar = match &self.op.stage {
                    Some((n, m, _)) => bar.progress((*n as f32 - 0.5) / *m as f32),
                    None => bar,
                };
                col = col.push(applet::padded_control(bar));
                if self.op.command != "auto" {
                    col = col.push(applet::padded_control(hrow(vec![
                        widget::space::horizontal().into(),
                        button::standard(label).on_press(Message::Open(Some("operation"))).into(),
                    ])));
                }
            }
            Headline::CheckFailed(e) => {
                col = col.push(applet::padded_control(ui::txt(text::caption(model::ellipsize(e, 200)))));
                col = col.push(applet::padded_control(hrow(vec![
                    widget::space::horizontal().into(),
                    button::standard(t!("Подробнее")).on_press(Message::Open(Some("updates"))).into(),
                ])));
            }
            _ => {}
        }

        // новости Arch перед обновлением — сразу под заголовком
        if let Some(n) = s.news.first() {
            let more = if s.news.len() > 1 { t!(" (и ещё {0})", s.news.len() - 1) } else { String::new() };
            col = col.push(ui::banner(
                "dialog-information-symbolic",
                t!("Новость Arch: {0}{1}", n.title, more),
                Some((t!("Прочитать"), Message::Open(Some("news")))),
            ));
        }

        // обновления: разбивка и единственная акцентная кнопка
        let total = s.total();
        if total > 0 && !matches!(head, Headline::Installing { .. }) {
            for (name, n) in model::updates_breakdown(s) {
                col = col.push(applet::padded_control(hrow(vec![ui::txt(text::body(name)).width(Length::Fill).into(), text::body(n.to_string()).into()])));
            }
            if let Some(note) = model::download_note(s) {
                col = col.push(applet::padded_control(ui::txt(text::caption(note))));
            }
            let install = button::suggested(t!("Установить {0}…", total)).on_press_maybe(self.can_act().then_some(Message::Install));
            col = col.push(applet::padded_control(hrow(vec![widget::space::horizontal().into(), install.into()])));
        }

        // VPN: переключатель и выбор сервера, только если VPN настроен
        if s.features.vpn && s.vpn.installed && s.vpn.has_subs {
            col = col.push(applet::padded_control(divider::horizontal::default()));
            let detail = if s.vpn.failed {
                t!("Ошибка запуска").to_string()
            } else if s.vpn.active {
                match self.vpn.as_ref().and_then(model::vpn_current) {
                    Some((name, d)) => format!("{} · {}", vpn::label(&name), model::delay_text(d)),
                    None => t!("Подключён").into(),
                }
            } else {
                t!("Выключен").into()
            };
            let toggle = widget::toggler(s.vpn.active).on_toggle_maybe(self.can_act().then_some(Message::VpnToggle));
            let info = widget::column::with_capacity(2).push(ui::txt(text::body("VPN"))).push(ui::txt(text::caption(detail))).width(Length::Fill);
            col = col.push(applet::padded_control(
                hrow(vec![widget::icon::from_name(if s.vpn.active { "network-vpn-symbolic" } else { "network-vpn-disabled-symbolic" }).size(20).into(), info.into(), toggle.into()])
                    .spacing(sp.space_xs)
                    .align_y(Alignment::Center),
            ));
            if s.vpn.active {
                let arrow = if self.servers_open { "pan-down-symbolic" } else { "pan-end-symbolic" };
                col = col.push(
                    applet::menu_button(hrow(vec![ui::txt(text::body(t!("Сервер"))).width(Length::Fill).into(), widget::icon::from_name(arrow).size(16).into()]))
                        .on_press(Message::ToggleServers),
                );
                if self.servers_open {
                    match self.vpn.as_ref().and_then(|v| model::vpn_servers(v, 5)) {
                        Some((group, list)) => {
                            let current = self.vpn.as_ref().and_then(|v| v.groups.iter().find(|g| g.name == group)).map(|g| g.now.clone()).unwrap_or_default();
                            for (name, d) in list {
                                // текущий сервер — галочкой, остальные выровнены по ней
                                let mark: Element<'_, Message> = if name == current {
                                    widget::icon::from_name("object-select-symbolic").size(16).into()
                                } else {
                                    widget::space::horizontal().width(16).into()
                                };
                                let row = hrow(vec![
                                    mark,
                                    ui::txt(text::body(model::ellipsize(&vpn::label(&name), 34))).width(Length::Fill).into(),
                                    text::caption(model::delay_text(d)).into(),
                                ]);
                                col = col.push(applet::menu_button(row).on_press_maybe((!self.pending).then(|| Message::Select(group.clone(), name.clone()))));
                            }
                        }
                        None => col = col.push(applet::padded_control(ui::txt(text::caption(t!("Список серверов загружается…"))))),
                    }
                    col = col.push(applet::menu_button(ui::txt(text::body(t!("Все серверы…")))).on_press(Message::Open(Some("vpn"))));
                }
            }
        }

        // предупреждения — только когда они есть
        let mut warn = vec![];
        if s.reboot {
            let label = if self.confirm_reboot { t!("Точно перезагрузить?") } else { t!("Перезагрузить") };
            let b = if self.confirm_reboot { button::destructive(label) } else { button::standard(label) };
            warn.push(ui::warn_row("system-reboot-symbolic", t!("Требуется перезагрузка").into(), b.on_press(Message::Reboot).into()));
        }
        if s.mirror_problem() {
            let what = if s.mirrors.apply_error.is_empty() { t!("Зеркал не отвечает: {0}", s.mirrors.failing) } else { t!("Зеркала не применены") .into() };
            warn.push(ui::warn_row(
                "dialog-warning-symbolic",
                what,
                button::standard(t!("Проверить")).on_press_maybe(self.can_act().then_some(Message::MirrorCheck)).into(),
            ));
        }
        if !warn.is_empty() {
            col = col.push(applet::padded_control(divider::horizontal::default()));
            for w in warn {
                col = col.push(applet::padded_control(w));
            }
        }

        col = col.push(applet::padded_control(divider::horizontal::default()));
        col = col.push(applet::menu_button(ui::txt(text::body(t!("Открыть upd…")))).on_press(Message::Open(None)));
        col = col.push(applet::menu_button(ui::txt(text::body(t!("Настройки…")))).on_press(Message::Open(Some("settings"))));
        widget::container(widget::scrollable(col).height(Length::Shrink)).max_height(480.0).width(Length::Fill).into()
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
    std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".cache/upd-notified.json")
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    use cosmic::Application;
    use cosmic::iced::futures::StreamExt;
    #[test]
    fn idle_applet_recovers_from_initial_helper_error() {
        use upd::common::contract_fixtures::{isolation_lock, TempDirGuard, EnvGuard};
        let _isolation = isolation_lock();
        let dir = TempDirGuard::new("cosmic-idle-applet-retry").unwrap();
        let mut env = EnvGuard::new(); env.set("UPD_HELPER_SOCK", dir.path().join("absent.sock"));
        let mut app = Applet::demo(Summary::default(), OpStatus::default(), None, false);
        app.summary_at = Some(Instant::now());
        app.jobs.demo_phase(Kind::Operation, Phase::Error("old helper protocol".into()));
        let stamp = app.stamp;
        let task = <Applet as Application>::update(&mut app, Message::Metadata(model::Metadata {
            busy: false, stamp, lang: upd::i18n::Lang::Ru, launch_errors: vec![],
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

/// Общий с `upd notify` учёт показанного: одно и то же не показывается дважды.
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
    if !upd::common::have("notify-send") {
        return;
    }
    let mut c = std::process::Command::new("notify-send");
    c.args(["-a", "upd", "-i", model::APP_ID]);
    for (k, label) in &actions { c.arg(format!("--action={k}={label}")); }
    c.arg(&title).arg(&body);
    let result = crate::notifications::send(c, |choice| {
        let result = match choice {
            crate::notifications::Action::Install => model::open_window(Some("updates"), Some("update")),
            crate::notifications::Action::News => model::open_window(Some("news"), None),
            crate::notifications::Action::Log => model::open_window(Some("operation"), None),
        };
        if let Err(error) = result { crate::launch::record_error(error); }
    });
    if let Err(error) = result { crate::launch::record_error(error.to_string()); }
}
