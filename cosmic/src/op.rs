//! Экран долгой операции: этап, живой вывод, вопросы и отмена. Вывод приходит потоком событий помощника.

use cosmic::iced::futures::{SinkExt, Stream};
use upd::helper::{self, Event, OperationEvent, PromptKind};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Ограниченный доступный журнал; копирование не обещает удалённую историю.
const KEEP: usize = 5000;
const KEEP_BYTES: usize = 3 << 20;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Phase { #[default] Idle, Starting, Running, Prompt, Answering, Cancelling, CancelAccepted, CancelRejected(String), Disconnected, Finished(i32) }
#[derive(Clone, Debug)]
pub struct Tail { pub follow: bool, pub unread: usize }
impl Default for Tail { fn default() -> Self { Self { follow: true, unread: 0 } } }

#[derive(Clone, Debug, Default)]
pub struct OpView {
    pub command: String,
    pub started: i64,
    pub lines: VecDeque<String>,
    line_bytes: usize,
    pub tail: Tail,
    pub phase: Phase,
    pub partial: String,
    pub stage: Option<(u32, u32, String)>,
    pub prompt: Option<(String, PromptKind)>,
    pub exit: Option<i32>,
    /// История или поток событий были усечены по бюджету helper-а.
    pub gap: bool,
    /// ответ, набираемый в поле ввода
    pub answer: String,
    pub reveal: bool,
    /// первая отмена — как Ctrl+C; повторная завершает команду
    pub cancel_sent: bool,
}

impl OpView {
    pub fn apply(&mut self, ev: Event) {
        match ev {
            Event::Reset { command, started } => {
                *self = OpView { command, started, phase: Phase::Running, ..Default::default() };
            }
            Event::Line { text } => {
                self.line_bytes += text.len() + 1;
                self.lines.push_back(text);
                self.partial.clear(); self.trim();
                if !self.tail.follow { self.tail.unread = self.tail.unread.saturating_add(1); }
            }
            Event::Partial { mut text } => {
                if text.len() > 16 << 10 {
                    let mut end = 16 << 10; while !text.is_char_boundary(end) { end -= 1; }
                    text.truncate(end); self.gap = true;
                }
                self.partial = text; self.trim();
                if !self.tail.follow { self.tail.unread = self.tail.unread.max(1); }
            }
            Event::Stage { n, m, title } => self.stage = Some((n, m, title)),
            Event::Prompt { text, kind } => {
                self.answer.clear();
                self.prompt = Some((text, kind));
                self.phase = Phase::Prompt;
            }
            Event::Answered => { self.prompt = None; self.answer.clear(); self.phase = Phase::Running; },
            Event::ReplayComplete => {}
            Event::Gap => self.gap = true,
            Event::Exit { code } => {
                self.exit = Some(code);
                self.prompt = None; self.answer.clear(); self.phase = Phase::Finished(code);
            }
        }
    }

    fn trim(&mut self) {
        while self.lines.len() > KEEP || self.line_bytes + self.partial.len() > KEEP_BYTES {
            let Some(line) = self.lines.pop_front() else { break };
            self.line_bytes -= line.len() + 1; self.gap = true;
        }
    }
    pub fn scroll(&mut self, at_end: bool) { self.tail.follow = at_end; if at_end { self.tail.unread = 0; } }
    pub fn connected(&mut self) {
        self.phase = if let Some(code) = self.exit { Phase::Finished(code) } else if self.prompt.is_some() { Phase::Prompt }
            else if self.cancel_sent { Phase::CancelAccepted } else if self.running() { Phase::Running } else { Phase::Idle };
    }
    pub fn running(&self) -> bool {
        !self.command.is_empty() && self.exit.is_none()
    }

    /// Доля выполненного по этапам (для полосы прогресса).
    pub fn progress(&self) -> Option<f32> {
        match (self.exit, &self.stage) {
            (Some(0), _) => Some(1.0),
            (Some(_), _) => None,
            (None, Some((n, m, _))) if *m > 0 => Some(((*n as f32 - 0.5) / *m as f32).clamp(0.0, 0.99)),
            _ => None,
        }
    }

    pub fn available_log(&self) -> String {
        let mut s = self.lines.iter().map(String::as_str).collect::<Vec<_>>().join("\n");
        if self.gap { s.insert_str(0, "[журнал усечён]\n"); }
        if !self.partial.is_empty() {
            s.push('\n');
            s.push_str(&self.partial);
        }
        s
    }
}

/// Поток событий операции. Чтение сокета блокирующее — идёт в отдельном потоке.
/// Поток заканчивается, когда помощник закрыл соединение (завершился); `None` в конце — сигнал переподключиться.
pub fn events(_generation: &u64) -> impl Stream<Item = Result<Option<OperationEvent>, String>> + use<> {
    cosmic::iced::stream::channel(256, async move |mut out| {
        let cancellation = Arc::new(EventCancellation::default());
        let _cancel_on_drop = CancelOnDrop(cancellation.clone());
        let (tx, mut rx) = tokio::sync::mpsc::channel(256);
        let worker_cancellation = cancellation.clone();
        let worker = std::thread::Builder::new().name("upd-cosmic-events".into()).spawn(move || {
            match helper::attach_events_observed(|handle| worker_cancellation.install(handle)) {
                Ok(events) => {
                    match events.cancel_handle() {
                        Ok(handle) => worker_cancellation.install(handle),
                        Err(error) => {
                            let _ = tx.blocking_send(Err(format!("event cancellation setup failed: {error}")));
                            return;
                        }
                    }
                    for item in events {
                        if worker_cancellation.is_cancelled() {
                            break;
                        }
                        match item {
                            Ok(event) => {
                                if tx.blocking_send(Ok(Some(event))).is_err() {
                                    worker_cancellation.cancel();
                                    return;
                                }
                            }
                            Err(error) => {
                                if !worker_cancellation.is_cancelled() {
                                    let _ = tx.blocking_send(Err(error));
                                }
                                return;
                            }
                        }
                    }
                }
                Err(error) => { let _ = tx.blocking_send(Err(error)); return; }
            }
            worker_cancellation.clear();
            if !worker_cancellation.is_cancelled() {
                let _ = tx.blocking_send(Ok(None));
            }
        });
        if let Err(error) = worker {
            let _ = out.send(Err(format!("event reader worker spawn failed: {error}"))).await;
            return;
        }
        while let Some(item) = rx.recv().await {
            let end = matches!(item, Ok(None) | Err(_));
            if out.send(item).await.is_err() || end {
                break;
            }
        }
        cancellation.cancel();
    })
}

#[derive(Default)]
struct EventCancellation {
    cancelled: AtomicBool,
    handle: Mutex<Option<helper::EventCancelHandle>>,
}

impl EventCancellation {
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    fn install(&self, handle: helper::EventCancelHandle) {
        if self.is_cancelled() {
            handle.cancel();
            return;
        }
        let mut slot = self.handle.lock().unwrap_or_else(|error| error.into_inner());
        if self.is_cancelled() {
            handle.cancel();
        } else {
            *slot = Some(handle);
        }
    }

    fn clear(&self) {
        self.handle.lock().unwrap_or_else(|error| error.into_inner()).take();
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        if let Some(handle) = self.handle.lock().unwrap_or_else(|error| error.into_inner()).take() {
            handle.cancel();
        }
    }
}

struct CancelOnDrop(Arc<EventCancellation>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_build_the_view() {
        let mut v = OpView::default();
        v.apply(Event::Reset { command: "update".into(), started: 1 });
        assert!(v.running());
        v.apply(Event::Stage { n: 3, m: 6, title: "x".into() });
        v.apply(Event::Line { text: "a".into() });
        v.apply(Event::Partial { text: "Go? [Y/n] ".into() });
        v.apply(Event::Prompt { text: "Go? [Y/n]".into(), kind: PromptKind::YesNo { default_yes: true } });
        assert!(v.prompt.is_some());
        assert!((v.progress().unwrap() - 2.5 / 6.0).abs() < 1e-6);
        v.apply(Event::Answered);
        v.apply(Event::Exit { code: 0 });
        assert!(!v.running() && v.prompt.is_none());
        assert_eq!(v.progress(), Some(1.0));
        assert_eq!(v.available_log(), "a\nGo? [Y/n] ");
    }
    #[test]
    fn dropping_stream_guard_cancels_reader() {
        let cancellation = Arc::new(EventCancellation::default());
        let guard = CancelOnDrop(cancellation.clone());
        assert!(!cancellation.is_cancelled());
        drop(guard);
        assert!(cancellation.is_cancelled());
        cancellation.cancel();
    }

    #[test]
    fn slow_gui_queue_and_journal_are_bounded() {
        let (tx, _rx) = tokio::sync::mpsc::channel(256);
        for n in 0..256 { tx.try_send(n).unwrap(); }
        assert!(matches!(tx.try_send(256), Err(tokio::sync::mpsc::error::TrySendError::Full(_))));
        let mut view = OpView::default();
        for _ in 0..1000 { view.apply(Event::Line { text: "x".repeat(16 << 10) }); }
        assert!(view.lines.iter().map(String::len).sum::<usize>() <= KEEP_BYTES);
        assert!(view.gap);
        assert!(view.available_log().starts_with("[журнал усечён]"));
    }

}

#[cfg(test)]
mod history_tests {
    use super::*;
    #[test]
    fn scroll_holds_history_until_follow_tail() {
        let mut v = OpView::default(); v.apply(Event::Reset { command: "update".into(), started: 1 }); v.scroll(false);
        for n in 0..6000 { v.apply(Event::Line { text: format!("line {n}") }); }
        assert_eq!(v.lines.len(), KEEP); assert_eq!(v.lines.front().unwrap(), "line 1000");
        assert_eq!(v.tail.unread, 6000); assert!(!v.tail.follow); assert!(v.gap);
        assert!(v.available_log().contains("line 1000")); v.scroll(true); assert_eq!(v.tail.unread, 0);
    }
    #[test]
    fn erroneous_exit_and_zero_stage_never_report_success_progress() {
        let mut v = OpView::default(); v.apply(Event::Stage { n: 1, m: 0, title: "invalid".into() }); assert_eq!(v.progress(), None);
        v.apply(Event::Exit { code: 7 }); assert_eq!(v.progress(), None); assert_eq!(v.phase, Phase::Finished(7));
        v.apply(Event::Exit { code: 0 }); assert_eq!(v.progress(), Some(1.0));
    }
}
