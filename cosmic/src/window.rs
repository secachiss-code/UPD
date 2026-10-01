//! Окно upd: одно окно с боковой навигацией (Обновления, Зеркала, VPN, Обслуживание, Настройки)
//! и экраном долгой операции поверх текущего раздела.

use crate::model::{self, Prefs};
use crate::jobs::{Jobs, Kind, Completion, Phase};
use crate::op::{self, OpView};
use crate::ui::{self, hrow, txt};
use cosmic::app::{Core, Task};
use cosmic::iced::{Alignment, Length, Subscription};
use cosmic::widget::{self, button, nav_bar, settings, text};
use cosmic::widget::list::ListButton;
use cosmic::{Application, ApplicationExt, Element};
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use upd::common::{fmt_ago, fmt_bytes, fmt_speed, fmt_time, host_of, unit_label, Config, MirrorState, News, UpdState};
use upd::extras::AurPkg;
use upd::helper::{self, PromptKind, Request};
use upd::summary::{self, OpStatus, Summary};
use upd::{i18n, t, vpn, Status};

static LOG_ID: LazyLock<widget::Id> = LazyLock::new(|| widget::Id::new("upd-log"));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Updates,
    Mirrors,
    Vpn,
    Maintenance,
    Settings,
}

impl Page {
    fn from_arg(s: &str) -> Option<Page> {
        Some(match s {
            "updates" | "news" => Page::Updates,
            "mirrors" => Page::Mirrors,
            "vpn" => Page::Vpn,
            "maintenance" => Page::Maintenance,
            "settings" => Page::Settings,
            _ => return None,
        })
    }
    fn title(self) -> &'static str {
        match self {
            Page::Updates => t!("Обновления"),
            Page::Mirrors => t!("Зеркала"),
            Page::Vpn => "VPN",
            Page::Maintenance => t!("Обслуживание"),
            Page::Settings => t!("Настройки"),
        }
    }
    fn icon(self) -> &'static str {
        match self {
            Page::Updates => "software-update-available-symbolic",
            Page::Mirrors => "network-server-symbolic",
            Page::Vpn => "network-vpn-symbolic",
            Page::Maintenance => "applications-system-symbolic",
            Page::Settings => "preferences-system-symbolic",
        }
    }
}

/// Аргументы окна; при повторном запуске передаются уже открытому окну (single instance).
#[derive(Clone, Debug, Default)]
pub struct Flags {
    pub page: Option<String>,
    pub run: Option<String>,
    action: String,
}

impl Flags {
    pub fn new(page: Option<String>, run: Option<String>) -> Self {
        Flags { page, run, action: "open".into() }
    }
    fn from_args(args: &[String]) -> Self {
        let get = |k: &str| args.iter().find_map(|a| a.strip_prefix(&format!("{k}=")).map(String::from)).filter(|v| !v.is_empty());
        Flags::new(get("page"), get("run"))
    }
}

impl cosmic::app::CosmicFlags for Flags {
    type SubCommand = String;
    type Args = Vec<String>;
    fn action(&self) -> Option<&String> {
        Some(&self.action)
    }
    fn args(&self) -> Vec<&str> {
        let mut v = vec![];
        if let Some(p) = &self.page {
            v.push(p.as_str());
        }
        v
    }
}

/// Данные без Debug/Clone в сообщениях.
#[derive(Clone)]
pub struct Shared<T>(pub Arc<T>);

impl<T> std::fmt::Debug for Shared<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("…")
    }
}

#[derive(Clone, Debug, Default)]
pub struct RestartInfo {
    services: Vec<String>,
    critical: Vec<String>,
    apps: Vec<String>,
}

type Loaded<T> = Option<Result<T, String>>;

struct SettingMutation { id: u64, draft: Config }

pub struct Window {
    jobs: Jobs<Message>,
    next_request: u64,
    setting_mutation: Option<SettingMutation>,
    pending_start: Option<u64>,
    queued_start: Option<Vec<String>>,
    submitted_sub: Option<(String, String)>,
    answer_request: Option<u64>,
    cancel_request: Option<u64>,
    core: Core,
    nav: nav_bar::Model,
    page: Page,
    summary: Summary,
    summary_loaded: bool,
    upd_state: UpdState,
    op_status: OpStatus,
    op: OpView,
    op_gen: u64,
    operation_id: Option<helper::OperationId>,
    op_view_id: Option<helper::OperationId>,
    prompt_id: Option<helper::PromptId>,
    start_pending: bool,
    pending_event_id: Option<helper::OperationId>,
    replaying_current_operation: bool,
    attached: bool,
    show_op: bool,
    confirm_cancel: bool,
    confirm_reboot: bool,
    pending: bool,
    /// upd занят (блокировка) — обновляется по таймеру, а не при каждой отрисовке
    busy: bool,
    stamp: u64,
    summary_at: Option<std::time::Instant>,
    error: String,
    notice: String,
    // обновления
    show_all_packages: bool,
    news_all: Loaded<Vec<News>>,
    aur_updates: Loaded<Vec<String>>,
    aur_query: String,
    aur_results: Loaded<Vec<AurPkg>>,
    // зеркала
    mirrors: MirrorState,
    config: Option<Config>,
    mirror_input: String,
    // VPN
    vpn_state: vpn::VpnState,
    snap: Option<vpn::Snapshot>,
    snap_error: String,
    sub_url: String,
    sub_name: String,
    sub_hidden: bool,
    confirm_delete: Option<String>,
    // обслуживание
    status: Option<Shared<Status>>,
    snapshots: Loaded<Vec<String>>,
    history: Loaded<Vec<String>>,
    restart: Loaded<RestartInfo>,
    // настройки
    prefs: Prefs,
}

#[derive(Clone)]
pub enum Message {
    Probe(Completion<Message>),
    Metadata(model::Metadata),
    Maintenance(Shared<Status>, Vec<String>, Vec<String>, RestartInfo),
    Tick,
    Summary(Box<Summary>, Box<UpdState>),
    OpStatus(OpStatus),
    OpEvent(u64, Result<Option<helper::OperationEvent>, String>),
    Reattach,
    Reboot,
    Start(Vec<String>),
    Started(u64, Result<helper::OperationId, String>),
    ShowOp(bool),
    AnswerResult(u64, helper::OperationId, helper::PromptId, Result<(), String>),
    CancelResult(u64, helper::OperationId, Result<(), String>),
    LogScroll(bool),
    FollowLog,
    Answer(String),
    AnswerInput(String),
    ToggleReveal,
    Cancel,
    CopyLog,
    Done(Result<(), String>),
    Settings(u64, Result<helper::SettingsReply, String>),
    DismissError,
    // обновления
    ToggleAllPackages,
    LoadNews,
    News(Result<Vec<News>, String>),
    OpenUrl(String),
    CheckAur,
    AurUpdates(Result<Vec<String>, String>),
    AurQuery(String),
    AurSearch,
    AurResults(Result<Vec<AurPkg>, String>),
    // зеркала
    Mirrors(Box<MirrorState>, Option<Box<Config>>),
    MirrorInput(String),
    // VPN
    VpnData(Box<vpn::VpnState>, Option<Box<vpn::Snapshot>>, String),
    RefreshVpn,
    Select(String, String),
    Delay(String),
    SubUrl(String),
    SubName(String),
    ToggleSubHidden,
    AddSub,
    DeleteSub(String),
    Terminal(Vec<String>),
    // обслуживание
    // настройки
    Set(&'static str, String),
    Lang(usize),
    Notifications(bool),
}

impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SubUrl(_) => f.write_str("SubUrl(<redacted>)"),
            Self::Answer(_) | Self::AnswerInput(_) => f.write_str("Answer(<redacted>)"),
            Self::Probe(c) => f.debug_struct("Probe").field("kind", &c.kind).finish_non_exhaustive(),
            _ => f.debug_tuple("WindowMessage").field(&std::mem::discriminant(self)).finish(),
        }
    }
}

fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static, m: impl FnOnce(Result<T, String>) -> Message + Send + 'static) -> Task<Message> {
    cosmic::task::future(async move {
        let result = tokio::task::spawn_blocking(f).await.unwrap_or_else(|e| Err(e.to_string()));
        m(result)
    })
}

fn load_summary() -> Result<Message, String> {
    Ok(Message::Summary(Box::new(model::load_summary()?), Box::new(model::update_state())))
}

fn load_mirrors() -> Result<Message, String> {
    let backend = upd::backend::detect()?;
    let cfg = Config::load(backend.default_mirrors())?;
    Ok(Message::Mirrors(Box::new(upd::mirrors::load_mirror_state()), Some(Box::new(cfg))))
}

fn load_vpn(with_snapshot: bool) -> Result<Message, String> {
    let state = vpn::load_state();
    let snap = if with_snapshot { Some(Box::new(model::vpn_snapshot()?)) } else { None };
    Ok(Message::VpnData(Box::new(state), snap, String::new()))
}

fn load_maintenance() -> Result<Message, String> {
    let backend = upd::backend::detect()?;
    let status = Shared(Arc::new(upd::gather_status(backend.as_ref())));
    let snapshots = helper::call_as(&Request::Query { what: "snapshots".into() })?;
    let history = helper::call_as(&Request::Query { what: "history".into() })?;
    let v = helper::call(&Request::Query { what: "restart".into() })?;
    let restart = RestartInfo {
        services: serde_json::from_value(v["services"].clone()).map_err(|e| e.to_string())?,
        critical: serde_json::from_value(v["critical"].clone()).map_err(|e| e.to_string())?,
        apps: serde_json::from_value(v["apps"].clone()).map_err(|e| e.to_string())?,
    };
    Ok(Message::Maintenance(status, snapshots, history, restart))
}

/// Язык: 0 — как в системе, дальше — по списку upd.
fn lang_options() -> Vec<String> {
    let mut v = vec![t!("Как в системе").to_string()];
    v.extend(i18n::ALL.iter().map(|l| l.name().to_string()));
    v
}

impl cosmic::Application for Window {
    type Executor = cosmic::executor::Default;
    type Flags = Flags;
    type Message = Message;
    const APP_ID: &'static str = model::APP_ID;

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, flags: Flags) -> (Self, Task<Message>) {
        model::init_lang();
        let mut nav = nav_bar::Model::default();
        for p in [Page::Updates, Page::Mirrors, Page::Vpn, Page::Maintenance, Page::Settings] {
            nav.insert().text(p.title()).icon(widget::icon::from_name(p.icon()).icon()).data(p);
        }
        nav.activate_position(0);
        let mut w = Window {
            jobs: Jobs::default(),
            next_request: 0,
            setting_mutation: None,
            pending_start: None,
            queued_start: None,
            submitted_sub: None,
            answer_request: None,
            cancel_request: None,
            core,
            nav,
            page: Page::Updates,
            summary: Summary::default(),
            summary_loaded: false,
            upd_state: UpdState::default(),
            op_status: OpStatus::default(),
            op: OpView::default(),
            op_gen: 0,
            operation_id: None,
            op_view_id: None,
            prompt_id: None,
            start_pending: false,
            pending_event_id: None,
            replaying_current_operation: false,
            attached: true,
            show_op: false,
            confirm_cancel: false,
            confirm_reboot: false,
            pending: false,
            busy: false,
            stamp: 0,
            summary_at: Some(std::time::Instant::now()),
            error: String::new(),
            notice: String::new(),
            show_all_packages: false,
            news_all: None,
            aur_updates: None,
            aur_query: String::new(),
            aur_results: None,
            mirrors: MirrorState::default(),
            config: None,
            mirror_input: String::new(),
            vpn_state: vpn::VpnState::default(),
            snap: None,
            snap_error: String::new(),
            sub_url: String::new(),
            sub_name: String::new(),
            sub_hidden: true,
            confirm_delete: None,
            status: None,
            snapshots: None,
            history: None,
            restart: None,
            prefs: model::load_prefs(),
        };
        let mut tasks = vec![w.probe(Kind::Summary, "", load_summary), w.probe(Kind::Mirrors, "", load_mirrors), w.probe(Kind::Operation, "", || model::load_op(true).map(Message::OpStatus)), w.probe(Kind::Metadata, "", || Ok(Message::Metadata(model::metadata())))];
        tasks.push(w.activate(flags));
        tasks.push(w.update_title());
        (w, Task::batch(tasks))
    }

    fn nav_model(&self) -> Option<&nav_bar::Model> {
        Some(&self.nav)
    }

    fn on_nav_select(&mut self, id: nav_bar::Id) -> Task<Message> {
        self.nav.activate(id);
        let page = self.nav.data::<Page>(id).copied().unwrap_or(Page::Updates);
        self.show_op = false;
        self.open_page(page)
    }

    fn dbus_activation(&mut self, msg: cosmic::dbus_activation::Message) -> Task<Message> {
        match msg.msg {
            cosmic::dbus_activation::Details::ActivateAction { args, .. } => self.activate(Flags::from_args(&args)),
            _ => Task::none(),
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subs = vec![cosmic::iced::time::every(Duration::from_secs(4)).map(|_| Message::Tick)];
        if self.attached && helper::available() {
            let generation = self.op_gen;
            subs.push(Subscription::run_with(generation, op::events).with(generation).map(|(generation, event)| Message::OpEvent(generation, event)));
        }
        Subscription::batch(subs)
    }

    fn header_end(&self) -> Vec<Element<'_, Message>> {
        let mut v: Vec<Element<'_, Message>> = vec![];
        if self.op.running() && !self.show_op {
            v.push(button::text(t!("Ход операции")).leading_icon(widget::icon::from_name("emblem-synchronizing-symbolic")).on_press(Message::ShowOp(true)).into());
        }
        v
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Probe(completion) => {
                let (accept, repeat) = self.jobs.complete(&completion, Message::Probe);
                if accept {
                    match completion.result {
                        Ok(message) => return Task::batch([self.update(*message), repeat]),
                        Err(error) => match completion.kind {
                            Kind::News => self.news_all = Some(Err(error)),
                            Kind::AurUpdates => self.aur_updates = Some(Err(error)),
                            Kind::AurSearch => self.aur_results = Some(Err(error)),
                            Kind::Vpn => self.snap_error = error,
                            _ => {}
                        }
                    }
                }
                return repeat;
            }
            Message::Tick => return self.probe(Kind::Metadata, "", || Ok(Message::Metadata(model::metadata()))),
            Message::Metadata(meta) => {
                if !meta.launch_errors.is_empty() { self.error = meta.launch_errors.join("\n"); }
                self.busy = meta.busy;
                let mut tasks = vec![];
                if meta.stamp != self.stamp || self.summary_at.is_none_or(|t| t.elapsed() > Duration::from_secs(60)) || matches!(self.jobs.phase(Kind::Summary), Phase::Error(_)) {
                    self.stamp = meta.stamp;
                    self.summary_at = Some(std::time::Instant::now());
                    tasks.push(self.probe(Kind::Summary, "", load_summary));
                }
                if self.busy || self.op.running() {
                    tasks.push(self.probe(Kind::Operation, "", || model::load_op(true).map(Message::OpStatus)));
                }
                if self.page == Page::Vpn && !self.show_op {
                    let active = self.summary.vpn.active;
                    tasks.push(self.probe(Kind::Vpn, "vpn", move || load_vpn(active)));
                }
                return Task::batch(tasks);
            }
            Message::Maintenance(status, snapshots, history, restart) => {
                self.status = Some(status); self.snapshots = Some(Ok(snapshots));
                self.history = Some(Ok(history)); self.restart = Some(Ok(restart));
            }
            Message::Summary(s, u) => {
                self.summary = *s;
                self.summary_loaded = true;
                self.upd_state = *u;
                if self.config.is_some() { if let Some(args) = self.queued_start.take() { return self.update(Message::Start(args)); } }
            }
            Message::OpStatus(st) => {
                let status_matches_current = self.operation_id.as_ref().is_none_or(|current| Some(current) == st.operation_id.as_ref());
                if !self.start_pending && status_matches_current {
                    self.operation_id = st.operation_id.clone();
                    self.prompt_id = st.prompt_id;
                    self.op_status = st;
                } else if self.start_pending {
                    self.op_status = st;
                }
            }
            Message::OpEvent(generation, Ok(Some(frame))) => {
                if generation != self.op_gen {
                    return Task::none();
                }
                if self.replaying_current_operation {
                    if self.operation_id.as_ref() == Some(&frame.operation_id) {
                        if matches!(&frame.event, helper::Event::ReplayComplete) {
                            self.replaying_current_operation = false;
                            self.op.connected();
                        }
                        return Task::none();
                    }
                    self.replaying_current_operation = false;
                }
                let started = matches!(&frame.event, helper::Event::Reset { .. });
                let same_operation = self.operation_id.as_ref() == Some(&frame.operation_id);
                // Reset is authoritative for the helper's current operation. Other event types
                // must match the current id so queued output from an older operation is ignored.
                let accept = same_operation || started && (self.operation_id.is_none() || self.start_pending);
                if !accept {
                    return Task::none();
                }
                if matches!(&frame.event, helper::Event::ReplayComplete) {
                    self.op.connected();
                    return Task::none();
                }
                if started && same_operation && !self.start_pending && self.op_view_id.as_ref() == Some(&frame.operation_id) {
                    // The helper replays the whole journal on reattach. Keep the current view and
                    // scroll offset intact until its explicit replay boundary arrives.
                    self.replaying_current_operation = true;
                    return Task::none();
                }
                if !same_operation {
                    self.answer_request = None; self.cancel_request = None;
                    self.operation_id = Some(frame.operation_id.clone());
                    self.prompt_id = None;
                    self.op = OpView::default();
                }
                if started {
                    self.op_view_id = Some(frame.operation_id.clone());
                }
                if started && self.start_pending {
                    self.pending_event_id = Some(frame.operation_id.clone());
                }
                let preserve_input = (same_operation && started).then(|| (self.op.answer.clone(), self.op.reveal));
                match &frame.event {
                    helper::Event::Prompt { .. } => {
                        let Some(prompt_id) = frame.prompt_id else {
                            self.error = "helper event is missing its prompt id".into();
                            return Task::none();
                        };
                        if self.prompt_id != Some(prompt_id) { self.answer_request = None; }
                        self.prompt_id = Some(prompt_id);
                    }
                    helper::Event::Answered => {
                        if self.prompt_id != frame.prompt_id {
                            return Task::none();
                        }
                        self.prompt_id = None; self.answer_request = None;
                    }
                    helper::Event::Exit { .. } => self.prompt_id = None,
                    _ => {}
                }
                let exited = matches!(&frame.event, helper::Event::Exit { .. });
                self.op.apply(frame.event);
                if let Some((answer, reveal)) = preserve_input {
                    self.op.answer = answer;
                    self.op.reveal = reveal;
                }
                if started {
                    self.confirm_cancel = false;
                }
                if exited { self.answer_request = None; self.cancel_request = None; }
                let mut tasks = vec![];
                if self.op.tail.follow { tasks.push(cosmic::iced::widget::operation::snap_to_end(LOG_ID.clone())); }
                if exited {
                    tasks.push(self.probe(Kind::Summary, "", load_summary));
                    tasks.push(self.probe(Kind::Operation, "", || model::load_op(true).map(Message::OpStatus)));
                    tasks.push(self.reload_page());
                }
                return Task::batch(tasks);
            }
            Message::OpEvent(generation, Ok(None)) => {
                if generation != self.op_gen {
                    return Task::none();
                }
                // помощник закрыл соединение (завершился по простою или перезапущен) — переподключиться позже
                self.attached = false;
                if self.op.exit.is_none() { self.op.phase = op::Phase::Disconnected; }
                return cosmic::task::future(async {
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    Message::Reattach
                });
            }
            Message::OpEvent(generation, Err(error)) => {
                if generation != self.op_gen {
                    return Task::none();
                }
                self.error = error;
                self.attached = false;
                if self.op.exit.is_none() { self.op.phase = op::Phase::Disconnected; }
                return cosmic::task::future(async {
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    Message::Reattach
                });
            }
            Message::Reattach => {
                self.attached = true;
                self.op_gen += 1;
            }
            Message::Reboot => {
                if !self.confirm_reboot {
                    self.confirm_reboot = true;
                } else {
                    return blocking(model::reboot, Message::Done);
                }
            }
            Message::Start(args) => {
                if args.first().is_some_and(|arg| arg == "update") && (!self.summary_loaded || self.config.is_none()) {
                    self.queued_start = Some(args);
                    return Task::batch([self.probe(Kind::Summary, "", load_summary), self.probe(Kind::Mirrors, "", load_mirrors)]);
                }
                if args.first().is_some_and(|arg| arg == "aur" || arg == "update" && self.update_in_terminal()) {
                    if !self.pending && !self.op.running() && !self.busy { return self.update(Message::Terminal(args)); }
                    return Task::none();
                }
                if self.pending || self.op.running() || self.busy { self.error = t!("upd занят").into(); return Task::none(); }
                let request = self.request_id();
                self.pending_start = Some(request);
                self.op.phase = op::Phase::Starting;
                self.pending = true;
                self.error.clear();
                self.show_op = true;
                self.confirm_cancel = false;
                self.jobs.invalidate(&[Kind::Operation, Kind::Vpn, Kind::News, Kind::AurUpdates, Kind::AurSearch, Kind::Maintenance]);
                self.start_pending = true;
                self.pending_event_id = None;
                return blocking(
                    move || {
                        let a: Vec<&str> = args.iter().map(String::as_str).collect();
                        model::start(&a)
                    },
                    move |result| Message::Started(request, result),
                );
            }
            Message::Started(request, r) => {
                if self.pending_start != Some(request) { return Task::none(); }
                self.pending_start = None;
                self.pending = false;
                self.start_pending = false;
                match r {
                    Ok(operation_id) => {
                        if let Some((url, name)) = self.submitted_sub.take() {
                            if self.sub_url == url { self.sub_url.clear(); }
                            if self.sub_name == name { self.sub_name.clear(); }
                        }
                        if self.pending_event_id.take().is_some_and(|observed| observed != operation_id) {
                            self.error = "helper start reply did not match the operation event stream".into();
                            self.prompt_id = None;
                            self.op = OpView::default();
                            self.op_view_id = None;
                            self.op_gen += 1;
                        }
                        self.operation_id = Some(operation_id);
                    }
                    Err(error) => {
                        self.submitted_sub = None;
                        self.pending_event_id = None;
                        self.error = error;
                        self.op.connected();
                        if !self.op.running() {
                            self.show_op = false;
                        }
                    }
                }
                // новая операция придёт в уже открытый поток событий; если он оборвался — подключиться заново
                if !self.attached {
                    self.attached = true;
                    self.op_gen += 1;
                }
            }
            Message::ShowOp(v) => {
                if v { self.jobs.invalidate(&[Kind::Vpn, Kind::News, Kind::AurUpdates, Kind::AurSearch, Kind::Maintenance]); }
                self.show_op = v;
                return if v { self.update_title() } else { Task::batch([self.update_title(), self.reload_page()]) };
            }
            Message::LogScroll(at_end) => self.op.scroll(at_end),
            Message::FollowLog => { self.op.scroll(true); return cosmic::iced::widget::operation::snap_to_end(LOG_ID.clone()); }
            Message::Answer(a) => {
                if self.answer_request.is_some() || !self.attached { return Task::none(); }
                if let Err(error) = helper::validate_answer(&a) { self.error = error; return Task::none(); }
                let (Some(operation_id), Some(prompt_id)) = (self.operation_id.clone(), self.prompt_id) else { return Task::none(); };
                let request = self.request_id(); self.answer_request = Some(request); self.op.phase = op::Phase::Answering;
                let reply_id = operation_id.clone();
                return blocking(move || helper::call(&Request::Input { operation_id, prompt_id, data: a }).map(|_| ()),
                    move |result| Message::AnswerResult(request, reply_id, prompt_id, result));
            }
            Message::AnswerResult(request, operation_id, prompt_id, result) => {
                if self.answer_request != Some(request) { return Task::none(); }
                self.answer_request = None;
                if self.operation_id.as_ref() != Some(&operation_id) || self.prompt_id != Some(prompt_id) { return Task::none(); }
                match result {
                    Ok(()) => { self.prompt_id = None; self.op.prompt = None; self.op.answer.clear(); self.op.phase = op::Phase::Running; }
                    Err(error) => { self.error = error; self.op.phase = if self.attached { op::Phase::Prompt } else { op::Phase::Disconnected }; }
                }
            }
            Message::AnswerInput(s) => { if self.answer_request.is_none() { self.op.answer = s; } }
            Message::ToggleReveal => self.op.reveal = !self.op.reveal,
            Message::Cancel => {
                if self.cancel_request.is_some() || !self.attached { return Task::none(); }
                if !self.confirm_cancel { self.confirm_cancel = true; return Task::none(); }
                let Some(operation_id) = self.operation_id.clone() else { return Task::none(); };
                let request = self.request_id(); self.cancel_request = Some(request); self.op.phase = op::Phase::Cancelling;
                let reply_id = operation_id.clone();
                return blocking(move || helper::call(&Request::Cancel { operation_id }).map(|_| ()),
                    move |result| Message::CancelResult(request, reply_id, result));
            }
            Message::CancelResult(request, operation_id, result) => {
                if self.cancel_request != Some(request) { return Task::none(); }
                self.cancel_request = None; self.confirm_cancel = false;
                if self.operation_id.as_ref() != Some(&operation_id) { return Task::none(); }
                match result {
                    Ok(()) => { self.op.cancel_sent = true; self.op.phase = op::Phase::CancelAccepted; }
                    Err(error) => { self.error = error.clone(); self.op.phase = op::Phase::CancelRejected(error); }
                }
            }
            Message::CopyLog => {
                self.notice = t!("Журнал скопирован").into();
                return cosmic::iced::clipboard::write(self.op.available_log());
            }
            Message::Done(r) => {
                if self.setting_mutation.is_none() && self.pending_start.is_none() { self.pending = false; }
                match r {
                    Ok(()) => return self.reload_page(),
                    Err(e) => self.error = e,
                }
            }
            Message::DismissError => {
                self.error.clear();
                self.notice.clear();
            }
            Message::ToggleAllPackages => self.show_all_packages = !self.show_all_packages,
            Message::LoadNews => {
                self.news_all = None;
                return self.probe(Kind::News, "updates", || upd::extras::arch_news(0).map(|v| Message::News(Ok(v))));
            }
            Message::News(r) => self.news_all = Some(r),
            Message::OpenUrl(u) => { if let Err(error) = model::open_url(&u) { self.error = error; } },
            Message::CheckAur => {
                self.aur_updates = None;
                return self.probe(Kind::AurUpdates, "updates", || {
                    let user = upd::common::UserContext::from_uid(unsafe { libc::geteuid() })?;
                    upd::extras::aur_updates(&user.name).map(|v| Message::AurUpdates(Ok(v)))
                });
            }
            Message::AurUpdates(r) => self.aur_updates = Some(r),
            Message::AurQuery(q) => {
                if self.aur_query != q { self.jobs.invalidate(&[Kind::AurSearch]); self.aur_results = None; }
                self.aur_query = q;
            },
            Message::AurSearch => {
                let q = self.aur_query.trim().to_string();
                if q.len() < 2 {
                    return Task::none();
                }
                self.aur_results = None;
                let key = q.clone();
                return self.probe(Kind::AurSearch, &key, move || upd::extras::aur_search(&q).map(|v| Message::AurResults(Ok(v))));
            }
            Message::AurResults(r) => self.aur_results = Some(r),
            Message::Mirrors(m, c) => {
                self.mirrors = *m;
                if self.setting_mutation.is_none() { if let Some(c) = c { self.config = Some(*c); } }
                if self.summary_loaded && self.config.is_some() { if let Some(args) = self.queued_start.take() { return self.update(Message::Start(args)); } }
            }
            Message::MirrorInput(s) => self.mirror_input = s,
            Message::VpnData(state, snap, err) => {
                self.vpn_state = *state;
                self.snap = snap.map(|s| *s);
                self.snap_error = err;
            }
            Message::RefreshVpn => return self.probe(Kind::Vpn, "vpn", || load_vpn(true)),
            Message::Select(group, name) => {
                return blocking(move || helper::call(&Request::VpnSelect { group, name }).map(|_| ()), Message::Done);
            }
            Message::Delay(group) => {
                self.notice = t!("Замер задержек…").into();
                return blocking(move || helper::call(&Request::VpnDelay { group }).map(|_| ()), Message::Done);
            }
            Message::SubUrl(s) => { if self.submitted_sub.is_none() { self.sub_url = s; } },
            Message::SubName(s) => { if self.submitted_sub.is_none() { self.sub_name = s; } },
            Message::ToggleSubHidden => self.sub_hidden = !self.sub_hidden,
            Message::AddSub => {
                if self.pending || self.op.running() || self.busy { self.error = t!("upd занят").into(); return Task::none(); }
                let (url, name) = (self.sub_url.trim().to_string(), self.sub_name.trim().to_string());
                if !url.starts_with("https://") {
                    self.error = t!("Адрес подписки должен начинаться с https://").into();
                    return Task::none();
                }
                let request = self.request_id();
                self.pending_start = Some(request);
                self.op.phase = op::Phase::Starting;
                self.submitted_sub = Some((self.sub_url.clone(), self.sub_name.clone()));
                self.show_op = true;
                self.pending = true;
                self.confirm_cancel = false;
                self.jobs.invalidate(&[Kind::Operation, Kind::Vpn, Kind::News, Kind::AurUpdates, Kind::AurSearch, Kind::Maintenance]);
                self.start_pending = true;
                self.pending_event_id = None;
                // адрес уходит помощнику в теле запроса, а не в аргументах команды
                return blocking(
                    move || helper::call_as::<helper::StartReply>(&Request::VpnAdd { url, name }).map(|reply| reply.operation_id),
                    move |result| Message::Started(request, result),
                );
            }
            Message::DeleteSub(id) => {
                if self.confirm_delete.as_deref() != Some(&id) {
                    self.confirm_delete = Some(id);
                    return Task::none();
                }
                self.confirm_delete = None;
                return self.update(Message::Start(vec!["vpn".into(), "del".into(), format!("id:{id}")]));
            }
            Message::Terminal(args) => {
                let args: Vec<&str> = args.iter().map(String::as_str).collect();
                if let Err(e) = model::open_terminal(&args) {
                    self.error = e;
                }
            }
            Message::Set(key, value) => {
                if self.pending { return Task::none(); }
                let Some(mut draft) = self.config.clone() else { self.error = t!("Настройки ещё не загружены").into(); return Task::none(); };
                if let Err(error) = draft.set(key, &value) { self.error = error; return Task::none(); }
                let request = self.request_id();
                self.setting_mutation = Some(SettingMutation { id: request, draft });
                self.jobs.invalidate(&[Kind::Mirrors]);
                self.pending = true;
                return blocking(move || helper::call_as::<helper::SettingsReply>(&Request::ConfigSet { key: key.into(), value }), move |result| Message::Settings(request, result));
            }
            Message::Settings(request, result) => {
                if self.setting_mutation.as_ref().map(|m| m.id) != Some(request) { return Task::none(); }
                self.setting_mutation = None;
                self.pending = false;
                match result {
                    Ok(reply) => {
                        self.error = reply.runtime_error().unwrap_or_default();
                        self.notice = match &reply.runtime { helper::SettingsRuntime::Pending { reason } => format!("{}: {reason}", t!("Настройка сохранена")), _ => String::new() };
                        self.config = Some(reply.config);
                        return self.reload_page();
                    }
                    Err(error) => self.error = error,
                }
            }
            Message::Lang(i) => {
                let code = if i == 0 { "auto".to_string() } else { i18n::ALL.get(i - 1).map(|l| l.code().to_string()).unwrap_or_else(|| "auto".into()) };
                let t = self.update(Message::Set("lang", code));
                return t.chain(cosmic::task::future(async {
                    tokio::time::sleep(Duration::from_millis(600)).await;
                    Message::Tick
                }));
            }
            Message::Notifications(on) => {
                self.prefs.notifications = on;
                if let Err(e) = model::save_prefs(&self.prefs) {
                    self.error = e;
                }
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let sp = cosmic::theme::spacing();
        let body: Element<'_, Message> = if self.show_op { self.op_view() } else { self.page_view() };
        let mut alerts = widget::column::with_capacity(4).spacing(sp.space_xxs);
        if !self.error.is_empty() {
            alerts = alerts.push(ui::banner("dialog-warning-symbolic", self.error.clone(), Some((t!("Закрыть"), Message::DismissError))));
        } else if !self.notice.is_empty() {
            alerts = alerts.push(ui::banner("dialog-information-symbolic", self.notice.clone(), Some((t!("Закрыть"), Message::DismissError))));
        }
        for (kind, error) in self.jobs.errors() {
            alerts = alerts.push(ui::banner("dialog-warning-symbolic", format!("{}: {error} · {}", kind.label(), t!("Показаны предыдущие данные")), None));
        }
        if !helper::available() {
            alerts = alerts.push(ui::banner("dialog-warning-symbolic", t!("Помощник upd не установлен — действия недоступны (sudo upd install)").into(), None));
        }
        let kind = match self.page { Page::Updates => Kind::Summary, Page::Mirrors | Page::Settings => Kind::Mirrors,
            Page::Vpn => Kind::Vpn, Page::Maintenance => Kind::Maintenance };
        match self.jobs.phase(kind) {
            Phase::Loading => alerts = alerts.push(text(t!("Загрузка…"))),
            Phase::Error(error) => alerts = alerts.push(text(format!("{error} · {}", t!("Показаны предыдущие данные")))),
            _ => {}
        }
        let col = widget::column::with_capacity(2).spacing(sp.space_s)
            .push(widget::container(widget::scrollable(alerts).height(Length::Shrink)).max_height(128.0).width(Length::Fill))
            .push(body);
        widget::container(col).padding([0, sp.space_m, sp.space_m, sp.space_m]).width(Length::Fill).height(Length::Fill).into()
    }
}

impl Window {
    /// Окно с заданными данными — для снимков интерфейса в тестах.
    #[cfg(test)]
    pub fn demo(summary: Summary, upd_state: UpdState, op: OpView, page: Page) -> Self {
        let (w, _) = <Window as cosmic::Application>::init(Core::default(), Flags::default());
        let mut w = w;
        w.jobs = Jobs::default();
        w.summary = summary;
        w.summary_loaded = true;
        w.upd_state = upd_state;
        w.op = op;
        w.page = page;
        w.config = Some(Config::defaults(vec![]));
        w
    }

    #[cfg(test)]
    pub fn demo_view(&self, operation: bool) -> Element<'_, Message> {
        if operation { self.op_view() } else { <Self as Application>::view(self) }
    }
    #[cfg(test)]
    pub fn demo_case(&mut self, case: &str) {
        match case {
            "loading" => self.jobs.demo_phase(Kind::Summary, Phase::Loading),
            "denied" => self.error = "polkit: authorization denied; no settings were saved".into(),
            "long-error" => self.error = "Сеть недоступна / connection refused: https://example.invalid/very-long-path/".repeat(20),
            "stale" => self.summary.checked = upd::common::now().saturating_sub(90000),
            _ => {}
        }
    }

    #[cfg(test)]
    pub fn set_demo_vpn(&mut self, state: vpn::VpnState, snap: vpn::Snapshot) {
        self.vpn_state = state;
        self.snap = Some(snap);
    }

    fn update_title(&mut self) -> Task<Message> {
        let title = if self.show_op { summary::op_title(&self.op.command).to_string() } else { self.page.title().to_string() };
        self.set_header_title(title.clone());
        match self.core.main_window_id() {
            Some(id) => self.set_window_title(format!("{title} — upd"), id),
            None => Task::none(),
        }
    }

    fn activate(&mut self, f: Flags) -> Task<Message> {
        let mut tasks = vec![];
        if let Some(p) = f.page.as_deref() {
            if p == "operation" {
                self.show_op = true;
            } else if let Some(page) = Page::from_arg(p) {
                let found = self.nav.iter().find(|id| self.nav.data::<Page>(*id) == Some(&page));
                if let Some(id) = found {
                    self.nav.activate(id);
                }
                self.show_op = false;
                tasks.push(self.open_page(page));
            }
        }
        if let Some(run) = f.run {
            let args: Vec<String> = run.split_whitespace().map(String::from).collect();
            if helper::allowed(&args) {
                tasks.push(self.update(Message::Start(args)));
            }
        }
        tasks.push(self.update_title());
        Task::batch(tasks)
    }

    fn open_page(&mut self, page: Page) -> Task<Message> {
        if self.page != page { self.jobs.invalidate(&[Kind::Vpn, Kind::Mirrors, Kind::News, Kind::AurUpdates, Kind::AurSearch, Kind::Maintenance]); }
        self.page = page;
        self.confirm_delete = None;
        Task::batch([self.reload_page(), self.update_title()])
    }

    fn request_id(&mut self) -> u64 { self.next_request = self.next_request.checked_add(1).expect("request counter exhausted"); self.next_request }
    fn display_config(&self) -> Option<&Config> { self.setting_mutation.as_ref().map(|m| &m.draft).or(self.config.as_ref()) }
    fn probe(&mut self, kind: Kind, key: &str, work: impl FnOnce() -> Result<Message, String> + Send + 'static) -> Task<Message> {
        self.jobs.request(kind, key.to_owned(), work, Message::Probe)
    }
    fn reload_page(&mut self) -> Task<Message> {
        match self.page {
            Page::Updates => self.probe(Kind::Summary, "", load_summary),
            Page::Mirrors | Page::Settings => self.probe(Kind::Mirrors, "", load_mirrors),
            Page::Vpn => {
                let active = self.summary.vpn.active;
                Task::batch([self.probe(Kind::Vpn, "vpn", move || load_vpn(active)), self.probe(Kind::Mirrors, "", load_mirrors)])
            }
            Page::Maintenance => self.probe(Kind::Maintenance, "maintenance", load_maintenance),
        }
    }

    fn can_act(&self) -> bool {
        !self.pending && !self.op.running() && !self.busy && helper::available()
    }

    fn update_in_terminal(&self) -> bool { self.summary.features.aur && self.config.as_ref().is_none_or(|c| c.aur) }
    fn start_msg(&self, args: &[&str]) -> Option<Message> {
        self.can_act().then(|| {
            let arguments = args.iter().map(|s| s.to_string()).collect();
            if args.first() == Some(&"aur") || args.first() == Some(&"update") && self.update_in_terminal() { Message::Terminal(arguments) }
            else { Message::Start(arguments) }
        })
    }

    fn page_view(&self) -> Element<'_, Message> {
        let content = match self.page {
            Page::Updates => self.updates_view(),
            Page::Mirrors => self.mirrors_view(),
            Page::Vpn => self.vpn_view(),
            Page::Maintenance => self.maintenance_view(),
            Page::Settings => self.settings_view(),
        };
        widget::scrollable(widget::container(content).max_width(900).width(Length::Fill)).height(Length::Fill).into()
    }

    // ---------- операция ----------

    fn op_view(&self) -> Element<'_, Message> {
        let sp = cosmic::theme::spacing();
        let o = &self.op;
        let mut col = widget::column::with_capacity(8).spacing(sp.space_s);
        let phase = if self.start_pending { &op::Phase::Starting } else { &o.phase };
        let state = match phase {
            op::Phase::Starting => Some(t!("Ожидание авторизации и запуска…").into()),
            op::Phase::Answering => Some(t!("Ответ отправляется…").into()),
            op::Phase::Cancelling => Some(t!("Отмена отправляется…").into()),
            op::Phase::CancelAccepted => Some(t!("Отмена принята; ожидается завершение").into()),
            op::Phase::CancelRejected(error) => Some(t!("Отмена отклонена: {0}", error)),
            op::Phase::Disconnected => Some(t!("Связь с помощником потеряна; переподключение…").into()),
            _ => None,
        };
        if let Some(state) = state { col = col.push(txt(text::body(state))); }
        if self.start_pending || matches!(o.phase, op::Phase::Starting) { return widget::container(col).height(Length::Fill).into(); }
        if o.command.is_empty() {
            col = col.push(txt(text::body(t!("Операций пока не было."))));
            return col.push(hrow(vec![widget::space::horizontal().into(), button::standard(t!("Закрыть")).on_press(Message::ShowOp(false)).into()])).into();
        }
        let stage = match &o.stage {
            Some((n, m, title)) => t!("Этап {0} из {1}: {2}", n, m, title),
            None => String::new(),
        };
        col = col.push(txt(text::title3(summary::op_title(&o.command))));
        if o.started > 0 {
            col = col.push(txt(text::caption(t!("Начата {0}", upd::common::fmt_clock(o.started)))));
        }
        if !stage.is_empty() {
            col = col.push(txt(text::body(stage)));
        }
        let bar = widget::progress_bar::linear::Linear::new().width(Length::Fill);
        col = col.push(match o.progress() {
            Some(p) => bar.progress(p),
            None if o.running() => bar,
            None => bar.progress(0.0),
        });

        // вопрос операции
        let answer_enabled = self.answer_request.is_none() && self.cancel_request.is_none() && self.attached;
        if let Some((q, kind)) = &o.prompt {
            // варианты ответа — кнопками, поэтому «[Y/n]» из вопроса не показываем
            let question = match kind {
                PromptKind::YesNo { .. } => q.trim_end().trim_end_matches(':').trim_end().trim_end_matches("[Y/n]").trim_end_matches("[y/N]").trim_end().to_string(),
                _ => q.clone(),
            };
            let question = widget::container(widget::scrollable(txt(text::heading(question)).wrapping(cosmic::iced::widget::text::Wrapping::WordOrGlyph)).height(Length::Shrink)).max_height(100.0).width(Length::Fill);
            let mut card = widget::column::with_capacity(3).spacing(sp.space_xs).push(question);
            let controls: Vec<Element<'_, Message>> = match kind {
                PromptKind::YesNo { default_yes } => {
                    let yes = if *default_yes { button::suggested(t!("Да")) } else { button::standard(t!("Да")) };
                    let no = if *default_yes { button::standard(t!("Нет")) } else { button::suggested(t!("Нет")) };
                    vec![widget::space::horizontal().into(), no.on_press_maybe(answer_enabled.then(|| Message::Answer("n".into()))).into(), yes.on_press_maybe(answer_enabled.then(|| Message::Answer("y".into()))).into()]
                }
                PromptKind::Secret | PromptKind::Text => {
                    let mut input = if matches!(kind, PromptKind::Secret) {
                        widget::secure_input(t!("Пароль"), &o.answer, Some(Message::ToggleReveal), !o.reveal)
                    } else { widget::text_input(t!("Ответ (пусто — по умолчанию)"), &o.answer) };
                    if answer_enabled { input = input.on_input(Message::AnswerInput).on_submit(Message::Answer); }
                    vec![input.width(Length::Fill).into(), button::suggested(t!("Отправить")).on_press_maybe(answer_enabled.then(|| Message::Answer(o.answer.clone()))).into()]
                }
            };
            card = card.push(hrow(controls));
            col = col.push(widget::container(card).padding(sp.space_s).class(cosmic::theme::Container::Card).width(Length::Fill));
        }

        // живой вывод: последние строки, прокрутка к концу
        if o.gap { col = col.push(txt(text::caption(t!("Ранняя часть журнала удалена по лимиту")))); }
        if !o.tail.follow {
            col = col.push(button::standard(t!("К новым строкам ({0})", o.tail.unread)).on_press(Message::FollowLog));
        }
        let mut log = widget::column::with_capacity(o.lines.len() + 1).spacing(0);
        for l in &o.lines {
            log = log.push(text::monotext(l.as_str()).wrapping(cosmic::iced::widget::text::Wrapping::WordOrGlyph));
        }
        if !o.partial.is_empty() {
            log = log.push(text::monotext(o.partial.as_str()));
        }
        col = col.push(
            widget::container(widget::scrollable(widget::container(log).padding(sp.space_xs).width(Length::Fill)).id(LOG_ID.clone()).on_scroll(|v| Message::LogScroll(v.content_bounds().height - v.bounds().height - v.absolute_offset().y <= 2.0)).height(Length::Fill))
                .class(cosmic::theme::Container::Card)
                .height(Length::Fill)
                .width(Length::Fill),
        );

        let mut footer: Vec<Element<'_, Message>> = vec![];
        match o.exit {
            None => {
                footer.push(txt(text::caption(t!("Можно закрыть окно — операция продолжится; ход виден на панели."))).width(Length::Fill).into());
                let label = if o.cancel_sent { t!("Завершить принудительно") }
                    else if self.confirm_cancel { t!("Точно отменить?") } else { t!("Отменить") };
                footer.push(button::destructive(label).on_press_maybe((self.cancel_request.is_none() && self.attached).then_some(Message::Cancel)).into());
            }
            Some(code) => {
                let (icon, msg) = if code == 0 { ("emblem-ok-symbolic", t!("Готово").to_string()) } else { ("dialog-warning-symbolic", t!("Завершилось с ошибкой (код {0})", code)) };
                footer.push(widget::icon::from_name(icon).size(20).into());
                footer.push(txt(text::heading(msg)).width(Length::Fill).into());
                footer.push(button::standard(t!("Скопировать доступный журнал")).on_press(Message::CopyLog).into());
                footer.push(button::suggested(t!("Закрыть")).on_press(Message::ShowOp(false)).into());
            }
        }
        col = col.push(hrow(footer));
        // «Закрыть» у готовой операции и навигация возвращают к разделу
        widget::container(col).height(Length::Fill).into()
    }

    // ---------- обновления ----------

    fn updates_view(&self) -> Element<'_, Message> {
        let sp = cosmic::theme::spacing();
        let s = &self.summary;
        let u = &self.upd_state;
        let head = summary::headline(s, &self.op_status);
        let mut col = widget::column::with_capacity(10).spacing(sp.space_m);

        let total = s.total();
        let install = button::suggested(if self.update_in_terminal() { t!("Обновить в терминале…").into() } else if total > 0 { t!("Установить {0}…", total) } else { t!("Обновить всё…").into() }).on_press_maybe(self.start_msg(&["update"]));
        let check = button::standard(t!("Проверить")).on_press_maybe(self.start_msg(&["check"]));
        let title = widget::column::with_capacity(2).push(txt(text::title3(head.title()))).push(txt(text::caption(summary::checked_line(s)))).width(Length::Fill);
        col = col.push(hrow(vec![title.into(), check.into(), install.into()]));
        if let summary::Headline::CheckFailed(e) = &head {
            col = col.push(ui::banner("dialog-warning-symbolic", e.clone(), None));
        }
        if !s.warning.is_empty() {
            col = col.push(ui::banner("dialog-information-symbolic", s.warning.clone(), None));
        }
        if let Some(note) = model::download_note(s) {
            col = col.push(txt(text::body(note)));
        }

        if self.update_in_terminal() { col = col.push(txt(text::caption(t!("Сборка AUR и редакторы требуют терминала; обновление откроется там")))); }

        // новости Arch — перед списком пакетов: в них бывают ручные шаги
        if s.features.news {
            let mut sec = settings::section().title(t!("Новости Arch"));
            if s.news.is_empty() {
                sec = sec.add(settings::item::builder(t!("Новых новостей с прошлого обновления нет")).control(button::text(t!("Последние")).on_press(Message::LoadNews)));
            }
            for n in &s.news {
                sec = sec.add(settings::item::builder(n.title.clone()).description(fmt_time(n.date)).control(button::text(t!("Открыть")).on_press(Message::OpenUrl(n.link.clone()))));
            }
            match &self.news_all {
                Some(Ok(list)) => {
                    for n in list.iter().take(10) {
                        sec = sec.add(settings::item::builder(n.title.clone()).description(fmt_time(n.date)).control(button::text(t!("Открыть")).on_press(Message::OpenUrl(n.link.clone()))));
                    }
                }
                Some(Err(e)) => sec = sec.add(settings::item::builder(t!("Не удалось загрузить новости")).description(e.clone()).control(widget::space::horizontal())),
                None => {}
            }
            col = col.push(sec);
        }

        // «имя старая -> новая»: имя — заголовком, версии — справа
        let list_section = |title: String, items: &[String], limit: usize| {
            let mut sec = settings::section().title(title);
            for l in items.iter().take(limit) {
                let (name, ver) = l.split_once(' ').unwrap_or((l.as_str(), ""));
                sec = sec.add(settings::item::builder(name.to_string()).control(text::caption(ver.replace(" -> ", " → "))));
            }
            sec
        };
        if !u.list.is_empty() {
            let limit = if self.show_all_packages { usize::MAX } else { 30 };
            let mut sec = list_section(t!("Пакеты: {0}", u.list.len()), &u.list, limit);
            if u.list.len() > 30 {
                let label = if self.show_all_packages { t!("Свернуть") } else { t!("Показать все") };
                sec = sec.add(settings::item_row(vec![widget::space::horizontal().into(), button::text(label).on_press(Message::ToggleAllPackages).into()]));
            }
            col = col.push(sec);
        }
        if !u.flatpak.is_empty() {
            col = col.push(list_section(format!("Flatpak: {}", u.flatpak.len()), &u.flatpak, usize::MAX));
        }
        if !u.firmware.is_empty() {
            col = col.push(list_section(t!("Прошивки: {0}", u.firmware.len()), &u.firmware, usize::MAX));
        }

        if s.features.aur {
            let mut sec = settings::section().title("AUR");
            let aur_line = match &self.aur_updates {
                None => t!("Обновления AUR проверяются вручную").to_string(),
                Some(Ok(v)) if v.is_empty() => t!("Обновлений AUR нет").into(),
                Some(Ok(v)) => t!("Обновлений AUR: {0} — установятся вместе с системой", v.len()),
                Some(Err(e)) => e.clone(),
            };
            sec = sec.add(settings::item::builder(aur_line).control(button::text(t!("Проверить AUR")).on_press(Message::CheckAur)));
            if let Some(Ok(v)) = &self.aur_updates {
                for l in v.iter().take(50) {
                    sec = sec.add(settings::item_row(vec![text::body(l.clone()).into()]));
                }
            }
            sec = sec.add(settings::item_row(vec![
                widget::search_input(t!("Найти пакет в AUR"), &self.aur_query).on_input(Message::AurQuery).on_submit(|_| Message::AurSearch).width(Length::Fill).into(),
                button::standard(t!("Найти")).on_press(Message::AurSearch).into(),
            ]));
            match &self.aur_results {
                Some(Ok(list)) if list.is_empty() => sec = sec.add(settings::item_row(vec![text::body(t!("ничего не найдено")).into()])),
                Some(Ok(list)) => {
                    for p in list.iter().take(30) {
                        let mut desc = format!("{} · ★{}", p.version, p.votes);
                        if let Some(v) = &p.installed {
                            desc += &format!(" · {}", t!("установлен {0}", v));
                        }
                        if p.out_of_date {
                            desc += &format!(" · {}", t!("устарел"));
                        }
                        desc += &format!("\n{}", p.desc);
                        let install = button::text(t!("Установить в терминале…")).on_press_maybe(self.start_msg(&["aur", "install", &p.name]));
                        sec = sec.add(settings::item::builder(p.name.clone()).description(desc).control(install));
                    }
                }
                Some(Err(e)) => sec = sec.add(settings::item_row(vec![text::body(e.clone()).into()])),
                None => {}
            }
            col = col.push(sec);
        }

        if !self.op.command.is_empty() && !self.op.running() {
            let res = match self.op.exit {
                Some(0) => t!("{0}: готово", summary::op_title(&self.op.command)),
                Some(c) => t!("{0}: ошибка (код {1})", summary::op_title(&self.op.command), c),
                None => String::new(),
            };
            col = col.push(settings::section().title(t!("Последняя операция")).add(settings::item::builder(res).control(button::text(t!("Журнал")).on_press(Message::ShowOp(true)))));
        }
        col.into()
    }

    // ---------- зеркала ----------

    fn mirrors_view(&self) -> Element<'_, Message> {
        let sp = cosmic::theme::spacing();
        let s = &self.summary;
        let m = &self.mirrors;
        let mut col = widget::column::with_capacity(8).spacing(sp.space_m);
        if !s.mirrors.managed {
            col = col.push(ui::banner("dialog-information-symbolic", if s.mirrors.note.is_empty() { t!("Зеркалами управляет пакетный менеджер").into() } else { s.mirrors.note.clone() }, None));
            return col.into();
        }
        let info = widget::column::with_capacity(2)
            .push(txt(text::title3(t!("Закреплено зеркал: {0}", s.mirrors.pinned))))
            .push(txt(text::caption(t!("Сеть: {0} · замер {1}", upd::or_dash(&m.label), fmt_ago(m.checked)))))
            .width(Length::Fill);
        col = col.push(hrow(vec![
            info.into(),
            button::standard(t!("Искать заново")).on_press_maybe(self.start_msg(&["mirrors", "rescan"])).into(),
            button::suggested(t!("Проверить")).on_press_maybe(self.start_msg(&["mirrors", "check"])).into(),
        ]));
        if !m.apply_error.is_empty() {
            col = col.push(ui::banner("dialog-warning-symbolic", t!("Зеркала не применены: {0}", m.apply_error), Some((t!("Применить"), Message::Start(vec!["mirrors".into(), "apply".into()])))));
        }
        for h in &m.hints {
            col = col.push(ui::banner("dialog-information-symbolic", h.clone(), None));
        }
        let mut sec = settings::section().title(t!("Последний замер"));
        if m.results.is_empty() {
            sec = sec.add(settings::item_row(vec![text::body(t!("Замеров ещё не было")).into()]));
        }
        for p in m.results.iter().take(25) {
            let mut desc = i18n::tr_data(&p.src);
            if let Some(l) = p.lag_h {
                desc += &format!(" · {}", t!("отстаёт {0:.1} ч", l));
            }
            let mark = if m.best.iter().any(|b| b == &p.url) { "● " } else { "" };
            sec = sec.add(settings::item::builder(format!("{mark}{}", host_of(&p.url))).description(desc).control(text::body(fmt_speed(p))));
        }
        col = col.push(sec);

        let mut pref = settings::section().title(t!("Предпочитаемые зеркала"));
        if let Some(c) = &self.config {
            for u in &c.mirrors {
                pref = pref.add(settings::item::builder(u.clone()).control(
                    button::icon(widget::icon::from_name("user-trash-symbolic")).tooltip(t!("Убрать")).on_press_maybe(self.start_msg(&["mirrors", "del", u])),
                ));
            }
        }
        let url = self.mirror_input.trim().to_string();
        let add = (url.starts_with("https://") || url.starts_with("http://")).then(|| Message::Start(vec!["mirrors".into(), "add".into(), url.clone()])).filter(|_| self.can_act());
        pref = pref.add(settings::item_row(vec![
            widget::text_input("https://…/$repo/os/$arch", &self.mirror_input).on_input(Message::MirrorInput).width(Length::Fill).into(),
            button::standard(t!("Добавить")).on_press_maybe(add).into(),
        ]));
        col = col.push(pref);
        col.into()
    }

    // ---------- VPN ----------

    fn vpn_view(&self) -> Element<'_, Message> {
        let sp = cosmic::theme::spacing();
        let s = &self.summary;
        let st = &self.vpn_state;
        let mut col = widget::column::with_capacity(10).spacing(sp.space_m);
        if !s.features.vpn || !s.vpn.installed {
            col = col.push(ui::banner("dialog-information-symbolic", t!("Служба VPN не установлена — sudo upd install").into(), None));
        }
        // состояние и переключатель
        let state = if s.vpn.failed {
            t!("ошибка запуска").to_string()
        } else if s.vpn.active {
            match self.snap.as_ref().and_then(model::vpn_current) {
                Some((n, d)) => t!("подключён · {0} · {1}", vpn::label(&n), model::delay_text(d)),
                None => t!("подключён").into(),
            }
        } else {
            t!("выключен").into()
        };
        let mut sub = String::new();
        if let Some(sn) = self.snap.as_ref().filter(|x| x.running) {
            sub = t!("ядро {0} · ↓ {1} ↑ {2} · соединений {3}", sn.version, fmt_bytes(sn.down), fmt_bytes(sn.up), sn.conns);
        } else if !st.core_version.is_empty() {
            sub = format!("mihomo {}", st.core_version);
        }
        let can_toggle = s.vpn.has_subs && self.can_act();
        let toggle = widget::toggler(s.vpn.active).on_toggle_maybe(can_toggle.then_some(|on| Message::Start(vec!["vpn".into(), if on { "start" } else { "stop" }.into()])));
        col = col.push(hrow(vec![
            widget::column::with_capacity(2).push(txt(text::title3(format!("VPN: {state}")))).push(txt(text::caption(sub))).width(Length::Fill).into(),
            toggle.into(),
        ]));
        if !self.snap_error.is_empty() && s.vpn.active {
            col = col.push(ui::banner("dialog-warning-symbolic", self.snap_error.clone(), Some((t!("Повторить"), Message::RefreshVpn))));
        }
        if !st.event.is_empty() {
            col = col.push(ui::banner("dialog-information-symbolic", st.event.clone(), None));
        }

        // подписки
        let mut subs = settings::section().title(t!("Подписки"));
        if st.subs.is_empty() {
            subs = subs.add(settings::item_row(vec![text::body(t!("Подписок нет — добавьте адрес ниже")).into()]));
        }
        for x in &st.subs {
            let mut desc = t!("{0} · серверов {1} · обновлена {2}", x.host, x.nodes, fmt_ago(x.updated));
            if let Some(i) = &x.info {
                desc += &upd::sub_info(i);
            }
            if !x.error.is_empty() {
                desc += &format!("\n⚠ {}", x.error);
            }
            let del_label = if self.confirm_delete.as_deref() == Some(&x.id) { t!("Точно удалить?") } else { t!("Удалить") };
            let mut controls: Vec<Element<'_, Message>> = vec![];
            if !x.active {
                controls.push(button::text(t!("Сделать активной")).on_press_maybe(self.start_msg(&["vpn", "use", &format!("id:{}", x.id)])).into());
            }
            controls.push(button::text(del_label).on_press_maybe(self.can_act().then(|| Message::DeleteSub(x.id.clone()))).into());
            let title = if x.active { format!("● {}", x.name) } else { x.name.clone() };
            subs = subs.add(settings::item::builder(title).description(desc).control(widget::Row::with_children(controls).spacing(sp.space_xxs)));
        }
        let mut url_input = widget::secure_input(t!("Адрес подписки (https://…)"), &self.sub_url, Some(Message::ToggleSubHidden), self.sub_hidden);
        let mut name_input = widget::text_input(t!("Название (необязательно)"), &self.sub_name);
        if self.submitted_sub.is_none() { url_input = url_input.on_input(Message::SubUrl); name_input = name_input.on_input(Message::SubName); }
        subs = subs.add(settings::item_row(vec![
            url_input.width(Length::FillPortion(3)).into(),
            name_input.width(Length::FillPortion(2)).into(),
            button::standard(t!("Добавить")).on_press_maybe(self.can_act().then_some(Message::AddSub)).into(),
        ]));
        if !st.subs.is_empty() {
            subs = subs.add(settings::item::builder(t!("Обновить подписки сейчас")).control(button::text(t!("Обновить")).on_press_maybe(self.start_msg(&["vpn", "update"]))));
        }
        col = col.push(subs);

        // серверы
        if let Some(sn) = self.snap.as_ref().filter(|x| x.running) {
            for g in sn.groups.iter().filter(|g| g.kind == "Selector" || g.kind == "URLTest").take(4) {
                let mut sec = settings::section().title(format!("{} → {}", vpn::label(&g.name), vpn::label(&g.now)));
                sec = sec.add(settings::item::builder(t!("Серверов: {0}", g.all.len())).control(button::text(t!("Проверить задержки")).on_press(Message::Delay(g.name.clone()))));
                let mut list: Vec<(&String, Option<u64>)> = g.all.iter().map(|n| (n, sn.delay.get(n).copied().filter(|d| *d > 0))).collect();
                list.sort_by_key(|(_, d)| d.unwrap_or(u64::MAX));
                for (n, d) in list.into_iter().take(60) {
                    let current = *n == g.now;
                    let label = vpn::label(n);
                    let ctl: Element<'_, Message> = if g.kind == "Selector" && !current {
                        widget::Row::with_children(vec![text::body(model::delay_text(d)).into(), button::text(t!("Выбрать")).on_press(Message::Select(g.name.clone(), n.clone())).into()])
                            .spacing(sp.space_xs)
                            .align_y(Alignment::Center)
                            .into()
                    } else {
                        text::body(model::delay_text(d)).into()
                    };
                    let item = settings::item::builder(label);
                    let item = if current { item.icon(widget::icon::from_name("object-select-symbolic").size(16)) } else { item.icon(widget::space::horizontal().width(16)) };
                    sec = sec.add(item.control(ctl));
                }
                col = col.push(sec);
            }
        }

        // режим и маршрутизация
        if let Some(c) = &self.config {
            let modes = vec![t!("TUN — вся система").to_string(), t!("только прокси 127.0.0.1:{0}", c.vpn_port)];
            let routes = vec![t!("по правилам").to_string(), t!("всё через VPN").into(), t!("всё напрямую").into()];
            let can = self.can_act();
            let mut sec = settings::section().title(t!("Режим"));
            let mode = if c.vpn_tun { 0 } else { 1 };
            let route = c.vpn_mode.min(2) as usize;
            let mode_control: Element<'_, Message> = if can {
                widget::dropdown(modes.clone(), Some(mode), |i| Message::Start(vec!["vpn".into(), if i == 0 { "tun" } else { "proxy" }.into()])).into()
            } else { text(modes[mode].clone()).into() };
            let route_control: Element<'_, Message> = if can {
                widget::dropdown(routes.clone(), Some(route), |i| Message::Start(vec!["vpn".into(), ["rule", "global", "direct"][i.min(2)].into()])).into()
            } else { text(routes[route].clone()).into() };
            sec = sec.add(settings::item::builder(t!("Режим")).control(mode_control));
            sec = sec.add(settings::item::builder(t!("Маршрутизация")).control(route_control));
            for (key, label) in [
                ("vpn_autostart", t!("Запуск при загрузке")),
                ("vpn_auto_select", t!("Автовыбор сервера (⚡ Авто)")),
                ("vpn_direct_ru", t!("Россия напрямую (геофайлы)")),
                ("vpn_direct_lan", t!("Локальная сеть напрямую")),
                ("vpn_dns", t!("Свой DNS (fake-ip)")),
                ("vpn_ipv6", "IPv6"),
                ("vpn_allow_lan", t!("Прокси для устройств в сети")),
            ] {
                sec = sec.add(self.toggle_item(key, label));
            }
            for key in ["vpn_port", "vpn_sub_update_h", "vpn_core_check_h"] {
                sec = sec.add(self.number_item(key));
            }
            col = col.push(sec);
        }

        let mut tools = settings::section().title(t!("Ядро и правила"));
        let core = if st.core_version.is_empty() { t!("не установлено").to_string() } else { format!("mihomo {}", st.core_version) };
        let latest = if !st.core_latest.is_empty() && st.core_latest != st.core_version { t!(" · доступно {0}", st.core_latest) } else { String::new() };
        tools = tools.add(settings::item::builder(t!("Ядро mihomo")).description(format!("{core}{latest}")).control(button::text(t!("Обновить")).on_press_maybe(self.start_msg(&["vpn", "core", "update"]))));
        let geo = if st.geo_updated > 0 { t!("от {0}", fmt_ago(st.geo_updated)) } else { t!("не скачаны").into() };
        tools = tools.add(settings::item::builder(t!("Геофайлы")).description(geo).control(button::text(t!("Обновить")).on_press_maybe(self.start_msg(&["vpn", "geo"]))));
        tools = tools.add(settings::item::builder(t!("Свои правила")).description(t!("Открывается редактор в терминале")).control(button::text(t!("Открыть…")).on_press(Message::Terminal(vec!["vpn".into(), "rules".into()]))));
        col = col.push(tools);
        col.into()
    }

    // ---------- обслуживание ----------

    fn maintenance_view(&self) -> Element<'_, Message> {
        let sp = cosmic::theme::spacing();
        let s = &self.summary;
        let mut col = widget::column::with_capacity(8).spacing(sp.space_m);

        let mut sys = settings::section().title(t!("Система"));
        sys = sys.add(settings::item::builder(t!("Пакетный менеджер")).control(text::body(s.features.backend.clone())));
        if let Some(st) = self.status.as_ref().map(|x| &x.0) {
            sys = sys.add(settings::item::builder(t!("Последнее обновление")).control(text::body(fmt_time(st.last_tx))));
            let free = st.free.map(fmt_bytes).unwrap_or_else(|| t!("неизвестно").into());
            sys = sys.add(settings::item::builder(t!("Свободно на диске")).control(text::body(free)));
            sys = sys.add(settings::item::builder(t!("Упавших служб")).control(text::body(st.failed.to_string())));
            sys = sys.add(settings::item::builder(t!("Фоновая проверка")).control(text::body(unit_label(&st.auto_timer))));
            sys = sys.add(settings::item::builder(t!("Слежение за сетью")).control(text::body(unit_label(&st.net_timer))));
        }
        if s.reboot {
            let label = if self.confirm_reboot { t!("Точно перезагрузить?") } else { t!("Перезагрузить") };
            sys = sys.add(settings::item::builder(t!("Требуется перезагрузка")).control(button::destructive(label).on_press(Message::Reboot)));
        }
        col = col.push(sys);

        let mut clean = settings::section().title(t!("Очистка"));
        let desc = match self.status.as_ref().map(|x| &x.0) {
            Some(st) => t!("кэш {0} · ненужных пакетов {1}", fmt_bytes(st.cache), st.orphans),
            None => t!("Подсчёт…").into(),
        };
        clean = clean.add(settings::item::builder(t!("Кэш и ненужные пакеты")).description(desc).control(button::standard(t!("Очистить…")).on_press_maybe(self.start_msg(&["clean"]))));
        col = col.push(clean);

        let mut cfgs = settings::section().title(t!("Новые файлы настроек"));
        match self.status.as_ref().map(|x| &x.0.pending) {
            Some(p) if p.is_empty() => cfgs = cfgs.add(settings::item_row(vec![text::body(t!("Новых файлов настроек нет")).into()])),
            Some(p) => {
                for f in p.iter().take(30) {
                    cfgs = cfgs.add(settings::item_row(vec![text::body(f.clone()).into()]));
                }
                if s.features.merge {
                    cfgs = cfgs.add(settings::item::builder(t!("Слить изменения")).description(t!("pacdiff откроется в терминале")).control(button::text(t!("Открыть…")).on_press(Message::Terminal(vec!["merge".into()]))));
                }
            }
            None => {}
        }
        col = col.push(cfgs);

        let mut rs = settings::section().title(t!("Службы со старыми библиотеками"));
        match &self.restart {
            Some(Ok(r)) => {
                if r.services.is_empty() && r.critical.is_empty() && r.apps.is_empty() {
                    rs = rs.add(settings::item_row(vec![text::body(t!("Перезапускать нечего")).into()]));
                }
                if !r.services.is_empty() {
                    rs = rs.add(settings::item::builder(r.services.join(", ")).control(button::standard(t!("Перезапустить")).on_press_maybe(self.start_msg(&["restart"]))));
                }
                if !r.critical.is_empty() {
                    rs = rs.add(settings::item::builder(t!("Нужна перезагрузка: {0}", r.critical.join(", "))).control(widget::space::horizontal()));
                }
                if !r.apps.is_empty() {
                    rs = rs.add(settings::item::builder(t!("Перезапустите программы: {0}", r.apps.join(", "))).control(widget::space::horizontal()));
                }
            }
            Some(Err(e)) => rs = rs.add(settings::item_row(vec![text::body(e.clone()).into()])),
            None => rs = rs.add(settings::item_row(vec![text::body(t!("Подсчёт…")).into()])),
        }
        col = col.push(rs);

        let lines_section = |title: &'static str, data: &Loaded<Vec<String>>, limit: usize, rev: bool| {
            let mut sec = settings::section().title(title);
            match data {
                Some(Ok(v)) if v.is_empty() => sec = sec.add(settings::item_row(vec![text::body(t!("пусто")).into()])),
                Some(Ok(v)) => {
                    let mut lines: Vec<&String> = v.iter().collect();
                    if rev {
                        lines.reverse();
                    }
                    for l in lines.into_iter().take(limit) {
                        sec = sec.add(settings::item_row(vec![text::monotext(l.clone()).wrapping(cosmic::iced::widget::text::Wrapping::WordOrGlyph).into()]));
                    }
                }
                Some(Err(e)) => sec = sec.add(settings::item_row(vec![text::body(e.clone()).into()])),
                None => sec = sec.add(settings::item_row(vec![text::body(t!("Загрузка…")).into()])),
            }
            sec
        };
        let snap_title = self.status.as_ref().map(|x| x.0.snapshots.clone()).unwrap_or_default();
        col = col.push(txt(text::caption(t!("Снапшоты: {0}", upd::or_dash(&snap_title)))));
        col = col.push(lines_section(t!("Снапшоты и откат"), &self.snapshots, 60, false));
        col = col.push(lines_section(t!("Журнал пакетов"), &self.history, 150, true));
        col.into()
    }

    // ---------- настройки ----------

    fn toggle_item<'a>(&'a self, key: &'static str, label: &'a str) -> ListButton<'a, Message> {
        let on = self.display_config().and_then(|c| c.get(key)).unwrap_or(0) != 0;
        let enabled = !self.pending && helper::available();
        let mut item = settings::item::builder(label);
        if let Some(doc) = setting_doc(key).filter(|d| *d != label) {
            item = item.description(doc);
        }
        item.toggler_maybe(on, enabled.then_some(move |v: bool| Message::Set(key, if v { "1" } else { "0" }.into())))
    }

    fn number_item(&self, key: &'static str) -> Element<'_, Message> {
        let (min, max, step) = number_range(key);
        let v = self.display_config().and_then(|c| c.get(key)).unwrap_or(min);
        let label = setting_label(key);
        let mut item = settings::item::builder(label);
        if let Some(doc) = setting_doc(key).filter(|d| *d != label) {
            item = item.description(doc);
        }
        let (min, max) = if self.pending || !helper::available() { (v, v) } else { (min, max) };
        item.control(widget::spin_button(v.to_string(), label, v, step, min, max, move |n| Message::Set(key, n.to_string()))).into()
    }

    fn settings_view(&self) -> Element<'_, Message> {
        let sp = cosmic::theme::spacing();
        let s = &self.summary;
        let mut col = widget::column::with_capacity(6).spacing(sp.space_m);

        let lang = self.display_config().map(|c| c.lang.clone()).unwrap_or_else(|| "auto".into());
        let sel = i18n::ALL.iter().position(|l| l.code() == lang).map(|i| i + 1).unwrap_or(0);
        let language: Element<'_, Message> = if self.pending || !helper::available() {
            text(lang_options().get(sel).cloned().unwrap_or_default()).into()
        } else { widget::dropdown(lang_options(), Some(sel), Message::Lang).into() };
        let general = settings::section()
            .title(t!("Общие"))
            .add(settings::item::builder(t!("Язык / Language")).control(language))
            .add(settings::item::builder(t!("Уведомления")).description(t!("Новые обновления, новости Arch, перезагрузка, итог операций")).toggler(self.prefs.notifications, Message::Notifications));
        col = col.push(general);

        let mut bg = settings::section().title(t!("Фоновая работа"));
        for key in ["prefetch", "prefetch_on_battery", "prefetch_on_metered"] {
            bg = bg.add(self.toggle_item(key, setting_label(key)));
        }
        for key in ["min_free_gb", "retries"] {
            bg = bg.add(self.number_item(key));
        }
        col = col.push(bg);

        let mut src = settings::section().title(t!("Источники обновлений"));
        let f = &s.features;
        for (key, show) in [("flatpak", f.flatpak), ("aur", f.aur), ("firmware", f.firmware), ("news", f.news), ("snapshot", true)] {
            // возможности, которых нет на этой системе, не показываются
            if show {
                src = src.add(self.toggle_item(key, setting_label(key)));
            }
        }
        col = col.push(src);

        if s.mirrors.managed {
            let mut mir = settings::section().title(t!("Зеркала"));
            for key in ["keep", "timeout", "extra_from_list", "rescan_count", "mirror_max_age_h", "network_memory_days", "max_lag_h", "parallel", "parallel_vpn"] {
                mir = mir.add(self.number_item(key));
            }
            col = col.push(mir);
        }
        col = col.push(txt(text::caption(t!("Системные настройки хранятся в {0}; изменение требует пароля администратора. Настройки VPN — в разделе VPN.", upd::common::conf_path()))));
        col.into()
    }
}

/// Описание из файла настроек без пометки «(1 — да, 0 — нет)»: в интерфейсе это переключатель.
fn setting_doc(key: &str) -> Option<&'static str> {
    Config::doc(key).map(|d| d.find(" (1").map(|i| &d[..i]).unwrap_or(d))
}

/// Короткое название настройки для интерфейса (полное описание — под ним).
fn setting_label(key: &str) -> &'static str {
    match key {
        "prefetch" => t!("Скачивать обновления заранее"),
        "prefetch_on_battery" => t!("Загрузка от батареи"),
        "prefetch_on_metered" => t!("Загрузка в лимитной сети"),
        "min_free_gb" => t!("Запас места, ГБ"),
        "retries" => t!("Попыток загрузки"),
        "flatpak" => "Flatpak",
        "aur" => "AUR",
        "firmware" => t!("Прошивки"),
        "news" => t!("Новости Arch"),
        "snapshot" => t!("Снапшот перед обновлением"),
        "keep" => t!("Лучших зеркал в начале списка"),
        "timeout" => t!("Таймаут замера, с"),
        "extra_from_list" => t!("Дополнительно из текущего списка"),
        "rescan_count" => t!("Из автопоиска при смене сети"),
        "mirror_max_age_h" => t!("Перепроверять через, ч"),
        "network_memory_days" => t!("Помнить сети, дней"),
        "max_lag_h" => t!("Допустимое отставание, ч"),
        "parallel" => t!("Замеров одновременно"),
        "parallel_vpn" => t!("Замеров одновременно при VPN"),
        "vpn_port" => t!("Порт прокси"),
        "vpn_sub_update_h" => t!("Обновлять подписки, ч"),
        "vpn_core_check_h" => t!("Проверять ядро, ч"),
        _ => "",
    }
}

/// Numeric controls use the shared config schema, including values above the former GUI-only limits.
fn number_range(key: &str) -> (i64, i64, i64) {
    let schema = upd::common::number_setting(key).expect("numeric control requires schema");
    (schema.min, schema.max, schema.step)
}

#[cfg(test)]
mod mutation_tests {
    use super::*;
    fn window() -> Window { Window::demo(Summary::default(), UpdState::default(), OpView::default(), Page::Settings) }
    fn send(w: &mut Window, message: Message) { drop(<Window as Application>::update(w, message)); }
    #[test]
    fn polkit_refusal_restores_committed_value_and_rejects_stale_reply() {
        let mut w = window();
        let committed = w.config.as_ref().unwrap().keep;
        send(&mut w, Message::Set("keep", "7".into()));
        let request = w.setting_mutation.as_ref().unwrap().id;
        assert_eq!(w.display_config().unwrap().keep, 7);
        assert_eq!(w.config.as_ref().unwrap().keep, committed);
        send(&mut w, Message::Set("keep", "8".into()));
        assert_eq!(w.setting_mutation.as_ref().unwrap().id, request);
        let mut unrelated = Config::defaults(vec![]); unrelated.keep = 9;
        send(&mut w, Message::Mirrors(Box::default(), Some(Box::new(unrelated))));
        assert!(w.pending);
        assert_eq!(w.config.as_ref().unwrap().keep, committed);
        send(&mut w, Message::Settings(request + 1, Err("stale".into())));
        assert!(w.pending);
        send(&mut w, Message::Settings(request, Err("polkit window closed".into())));
        assert!(!w.pending);
        assert_eq!(w.display_config().unwrap().keep, committed);
        assert!(w.error.contains("polkit"));
    }
    #[test]
    fn saved_apply_failure_displays_actual_saved_value() {
        let mut w = window();
        send(&mut w, Message::Set("vpn_port", "7891".into()));
        let request = w.setting_mutation.as_ref().unwrap().id;
        let mut saved = w.config.clone().unwrap(); saved.vpn_port = 7891;
        send(&mut w, Message::Settings(request, Ok(helper::SettingsReply {
            config: saved, revision: "fixture".into(), runtime: helper::SettingsRuntime::SavedButNotApplied {
                reason: "runtime unavailable".into(), retry: "upd vpn apply".into(),
            },
        })));
        assert_eq!(w.config.as_ref().unwrap().vpn_port, 7891);
        assert!(!w.pending);
        assert!(w.error.contains("runtime unavailable"));
    }
    #[test]
    fn subscription_form_survives_busy_and_refusal_and_clears_only_matching_success() {
        let mut w = window();
        w.sub_url = "https://example.invalid/private-token".into(); w.sub_name = "fixture".into();
        assert!(!format!("{:?}", Message::SubUrl(w.sub_url.clone())).contains("private-token"));
        w.busy = true; send(&mut w, Message::AddSub);
        assert!(w.pending_start.is_none()); assert!(!w.sub_url.is_empty());
        w.busy = false; send(&mut w, Message::AddSub);
        let first = w.pending_start.unwrap();
        assert!(!w.sub_url.is_empty());
        send(&mut w, Message::Started(first, Err("helper busy".into())));
        assert!(!w.sub_url.is_empty()); assert_eq!(w.sub_name, "fixture");
        send(&mut w, Message::AddSub);
        let second = w.pending_start.unwrap();
        send(&mut w, Message::Started(first, Ok(helper::OperationId("stale".into()))));
        assert!(!w.sub_url.is_empty());
        send(&mut w, Message::Started(second, Ok(helper::OperationId("accepted".into()))));
        assert!(w.sub_url.is_empty()); assert!(w.sub_name.is_empty());
    }
    #[test]
    fn gui_ranges_show_large_legal_values_without_clamping() {
        for key in ["mirror_max_age_h", "vpn_sub_update_h", "vpn_core_check_h", "max_lag_h"] {
            assert_eq!(number_range(key).1, upd::common::MAX_HOURS);
        }
        assert_eq!(number_range("min_free_gb").1, 1 << 20);
        let mut w = window(); w.config.as_mut().unwrap().vpn_sub_update_h = 10000;
        assert_eq!(w.display_config().unwrap().get("vpn_sub_update_h"), Some(10000));
    }
}

#[cfg(test)]
mod operation_tests {
    use super::*;
    fn window() -> Window {
        let mut w = Window::demo(Summary::default(), UpdState::default(), OpView::default(), Page::Updates);
        w.operation_id = Some(helper::OperationId("fixture".into())); w.prompt_id = Some(helper::PromptId(1));
        w.op.apply(helper::Event::Reset { command: "update".into(), started: 1 });
        w.op.apply(helper::Event::Prompt { text: "Continue?".into(), kind: PromptKind::YesNo { default_yes: true } }); w
    }
    fn send(w: &mut Window, m: Message) { drop(<Window as Application>::update(w, m)); }
    #[test]
    fn double_answer_is_blocked_and_failure_can_be_retried() {
        let mut w = window(); w.op.answer = "y".into(); send(&mut w, Message::Answer("y".into())); let id = w.answer_request.unwrap();
        send(&mut w, Message::Answer("n".into())); assert_eq!(w.answer_request, Some(id)); assert_eq!(w.op.phase, op::Phase::Answering);
        send(&mut w, Message::AnswerResult(id, helper::OperationId("fixture".into()), helper::PromptId(1), Err("failed".into())));
        assert!(w.op.prompt.is_some()); assert_eq!(w.op.answer, "y"); assert!(w.answer_request.is_none());
        send(&mut w, Message::Answer("y".into())); let retry = w.answer_request.unwrap(); assert_ne!(retry, id);
        send(&mut w, Message::AnswerResult(retry, helper::OperationId("fixture".into()), helper::PromptId(1), Ok(())));
        assert!(w.op.prompt.is_none()); assert!(w.op.answer.is_empty());
    }
    #[test]
    fn cancel_refusal_and_disconnect_are_explicit() {
        let mut w = window(); send(&mut w, Message::Cancel); send(&mut w, Message::Cancel); let id = w.cancel_request.unwrap();
        send(&mut w, Message::CancelResult(id, helper::OperationId("fixture".into()), Err("rejected".into())));
        assert!(!w.op.cancel_sent); assert!(matches!(w.op.phase, op::Phase::CancelRejected(_)));
        let generation = w.op_gen; send(&mut w, Message::OpEvent(generation, Err("disconnected".into())));
        assert_eq!(w.op.phase, op::Phase::Disconnected); assert!(w.op.prompt.is_some());
        send(&mut w, Message::Answer("y".into())); assert!(w.answer_request.is_none());
    }
    #[test]
    fn restarted_helper_and_delayed_callbacks_never_report_false_success() {
        let mut w = window(); let old = w.operation_id.clone().unwrap();
        let generation = w.op_gen;
        send(&mut w, Message::OpEvent(generation, Ok(None)));
        send(&mut w, Message::OpStatus(OpStatus::default()));
        assert_eq!(w.op.phase, op::Phase::Disconnected); assert_eq!(w.op.exit, None);
        assert_eq!(w.operation_id.as_ref(), Some(&old));
        send(&mut w, Message::Reattach);
        w.start_pending = true;
        let new = helper::OperationId("operation-B".into());
        let frame = |id, event| helper::OperationEvent { protocol_version: helper::PROTOCOL_VERSION, operation_id: id, prompt_id: None, event };
        let generation = w.op_gen;
        send(&mut w, Message::OpEvent(generation, Ok(Some(frame(new.clone(), helper::Event::Reset { command: "check".into(), started: 2 })))));
        w.start_pending = false;
        for _ in 0..100 {
            send(&mut w, Message::OpEvent(generation, Ok(Some(frame(old.clone(), helper::Event::Exit { code: 0 })))));
            send(&mut w, Message::OpEvent(generation.saturating_sub(1), Ok(Some(frame(new.clone(), helper::Event::Exit { code: 0 })))));
            send(&mut w, Message::AnswerResult(99, old.clone(), helper::PromptId(1), Ok(())));
            send(&mut w, Message::CancelResult(99, old.clone(), Ok(())));
        }
        assert_eq!(w.operation_id, Some(new)); assert_eq!(w.op.exit, None);
        assert_eq!(w.op.command, "check"); assert_eq!(w.op.phase, op::Phase::Running);
    }

}

#[cfg(test)]
mod startup_tests {
    use super::*;
    #[test]
    fn update_activation_waits_for_summary_before_choosing_terminal() {
        let mut w = Window::demo(Summary::default(), UpdState::default(), OpView::default(), Page::Updates);
        w.summary_loaded = false;
        drop(<Window as Application>::update(&mut w, Message::Start(vec!["update".into()])));
        assert!(w.queued_start.is_some()); assert!(w.pending_start.is_none());
        drop(<Window as Application>::update(&mut w, Message::Summary(Box::default(), Box::default())));
        assert!(w.queued_start.is_none()); assert!(w.pending_start.is_some());
    }
}
