//! Экран долгой операции: этап, живой вывод, вопросы и отмена. Вывод приходит потоком событий помощника.

use cosmic::iced::futures::{SinkExt, Stream};
use upd::helper::{self, Event, PromptKind};

/// Сколько строк держать для показа (полный журнал можно скопировать).
const KEEP: usize = 5000;

#[derive(Clone, Debug, Default)]
pub struct OpView {
    pub command: String,
    pub started: i64,
    pub lines: Vec<String>,
    pub partial: String,
    pub stage: Option<(u32, u32, String)>,
    pub prompt: Option<(String, PromptKind)>,
    pub exit: Option<i32>,
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
                *self = OpView { command, started, ..Default::default() };
            }
            Event::Line { text } => {
                self.lines.push(text);
                if self.lines.len() > KEEP {
                    self.lines.drain(..self.lines.len() - KEEP);
                }
                self.partial.clear();
            }
            Event::Partial { text } => self.partial = text,
            Event::Stage { n, m, title } => self.stage = Some((n, m, title)),
            Event::Prompt { text, kind } => {
                self.answer.clear();
                self.prompt = Some((text, kind));
            }
            Event::Answered => self.prompt = None,
            Event::Exit { code } => {
                self.exit = Some(code);
                self.prompt = None;
            }
        }
    }

    pub fn running(&self) -> bool {
        !self.command.is_empty() && self.exit.is_none()
    }

    /// Доля выполненного по этапам (для полосы прогресса).
    pub fn progress(&self) -> Option<f32> {
        match (self.exit, &self.stage) {
            (Some(_), _) => Some(1.0),
            (None, Some((n, m, _))) => Some((*n as f32 - 0.5) / *m as f32),
            _ => None,
        }
    }

    pub fn full_log(&self) -> String {
        let mut s = self.lines.join("\n");
        if !self.partial.is_empty() {
            s.push('\n');
            s.push_str(&self.partial);
        }
        s
    }
}

/// Поток событий операции. Чтение сокета блокирующее — идёт в отдельном потоке.
/// Поток заканчивается, когда помощник закрыл соединение (завершился); `None` в конце — сигнал переподключиться.
pub fn events(_generation: &u64) -> impl Stream<Item = Option<Event>> + use<> {
    cosmic::iced::stream::channel(256, async move |mut out| {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        std::thread::spawn(move || {
            match helper::attach_events() {
                Ok(it) => {
                    for ev in it {
                        if tx.send(Some(ev)).is_err() {
                            return;
                        }
                    }
                }
                Err(_) => std::thread::sleep(std::time::Duration::from_secs(2)),
            }
            let _ = tx.send(None);
        });
        while let Some(ev) = rx.recv().await {
            let end = ev.is_none();
            if out.send(ev).await.is_err() || end {
                break;
            }
        }
    })
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
        assert_eq!(v.full_log(), "a\nGo? [Y/n] ");
    }
}
