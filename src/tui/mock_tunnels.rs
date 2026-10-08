//! I17-D.T03.a: page mock for app tunnels on fixtures, only with `CM_TUI_MOCK_TUNNELS=1`.
//!
//! The four axes NET / REGION / STATE / APP and the age of each axis's evidence are
//! shown separately. There is no single green lamp: no row sums the axes into "ok".
//! Stop follows I17D-NAV: a question that names the consequence, `y`/`Enter` confirm,
//! `n`/`Esc` cancel, other keys keep the question. The mock never runs a command.
use cm::t;
use ratatui::Frame;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Cell, Paragraph, Row, Table, Wrap};
use serde::Deserialize;
use std::collections::BTreeMap;

const FIXTURES: &str = include_str!("../../tests/fixtures/i17/states.json");
const AXES: [&str; 4] = ["net", "region", "state", "app"];
/// Evidence older than this is marked stale next to its age.
const STALE_AFTER_S: u64 = 3600;

pub fn enabled() -> bool {
    std::env::var("CM_TUI_MOCK_TUNNELS").ok().as_deref() == Some("1")
}

#[derive(Clone, Debug, Deserialize)]
pub struct Scenario {
    pub name: String,
    pub host: String,
    pub apps: String,
    pub condition: String,
    pub axes: BTreeMap<String, String>,
    pub age_s: BTreeMap<String, Option<u64>>,
    pub failure: Option<String>,
    pub sessions: u32,
}

pub fn scenarios() -> Vec<Scenario> {
    serde_json::from_str(FIXTURES).expect("tests/fixtures/i17/states.json")
}

#[derive(Default)]
pub struct MockPage {
    pub index: usize,
    pub confirm: bool,
}

/// What a key did. `Back` leaves the page; `Message` goes to the status line.
pub enum Outcome {
    Stay,
    Back,
    Message(&'static str),
}

impl MockPage {
    pub fn key(&mut self, k: KeyCode, count: usize) -> Outcome {
        if self.confirm {
            return match k {
                KeyCode::Char('y') | KeyCode::Enter => {
                    self.confirm = false;
                    Outcome::Message(t!("макет: ничего не выполнено"))
                }
                KeyCode::Char('n') | KeyCode::Esc => {
                    self.confirm = false;
                    Outcome::Message(t!("остановка отменена"))
                }
                _ => Outcome::Stay,
            };
        }
        match k {
            KeyCode::Char('q') | KeyCode::Esc | KeyCode::Backspace => return Outcome::Back,
            KeyCode::Right | KeyCode::Tab => self.index = (self.index + 1) % count.max(1),
            KeyCode::Left | KeyCode::BackTab => {
                self.index = (self.index + count.max(1) - 1) % count.max(1)
            }
            KeyCode::Char('x') => self.confirm = true,
            _ => {}
        }
        Outcome::Stay
    }

    pub fn keys(&self) -> Vec<(&'static str, &'static str)> {
        if self.confirm {
            vec![("y/enter", t!("остановить")), ("n/esc", t!("отмена"))]
        } else {
            vec![
                ("q", t!("назад")),
                ("←→", t!("сценарий")),
                ("x", t!("остановить")),
            ]
        }
    }
}

fn host_label(host: &str) -> &'static str {
    match host {
        "off" => t!("выкл"),
        "proxy" => t!("прокси"),
        "tunnel" => t!("туннель"),
        _ => t!("неизвестно"),
    }
}

fn apps_label(apps: &str) -> &'static str {
    match apps {
        "shared" => t!("общий туннель"),
        _ => t!("отдельные туннели"),
    }
}

fn condition_label(condition: &str) -> &'static str {
    match condition {
        "blocked" => t!("заблокировано"),
        "degraded" => t!("снижено"),
        _ => t!("неизвестно"),
    }
}

fn failure_label(code: &str) -> &'static str {
    match code {
        "api-down" => t!("API ядра не отвечает"),
        "dns-check" => t!("проверка DNS не пройдена"),
        "region-mismatch" => t!("страна выхода не совпала"),
        _ => t!("ошибка"),
    }
}

/// Symbol first, so the meaning survives a narrow cell and a terminal without colour.
fn status(value: &str) -> (&'static str, &'static str, Style) {
    match value {
        "verified" => ("✓", t!("подтверждено"), Style::new().fg(Color::Green)),
        "partial" => ("◐", t!("частично"), Style::new().fg(Color::Yellow)),
        "blocked" => ("✕", t!("заблокировано"), Style::new().fg(Color::Red)),
        "error" => (
            "!",
            t!("ошибка"),
            Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        _ => ("?", t!("неизвестно"), Style::new().fg(Color::DarkGray)),
    }
}

/// Age of one axis's evidence. A narrow cell marks stale evidence with `⚠` instead of a word.
pub fn age_cell(age: Option<u64>, narrow: bool) -> String {
    match age {
        Some(seconds) if narrow && seconds > STALE_AFTER_S => {
            format!("{} ⚠", age_value(seconds))
        }
        _ => age_text(age),
    }
}

fn age_value(seconds: u64) -> String {
    match seconds {
        0..60 => t!("{0} с", seconds),
        60..3600 => t!("{0} мин", seconds / 60),
        3600..86400 => t!("{0} ч", seconds / 3600),
        _ => t!("{0} д", seconds / 86400),
    }
}

pub fn age_text(age: Option<u64>) -> String {
    let Some(seconds) = age else {
        return "—".to_owned();
    };
    let text = age_value(seconds);
    if seconds > STALE_AFTER_S {
        format!("{text} · {}", t!("устарело"))
    } else {
        text
    }
}

pub fn draw(f: &mut Frame, area: Rect, page: &MockPage, all: &[Scenario]) {
    let Some(scenario) = all.get(page.index) else {
        return;
    };
    let roomy = area.height >= 14 && area.width >= 60;
    let inner = if roomy {
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .title(format!(
                " {} · {} · {} ",
                t!("Туннели (макет)"),
                t!("макет {0}/{1}", page.index + 1, all.len()),
                scenario.name
            ));
        let inner = block.inner(area);
        f.render_widget(block, area);
        inner
    } else {
        area
    };
    let dim = Style::new().fg(Color::DarkGray);
    // Host mode and live app sessions share a line: "host off, app tunnels alive" (Q22)
    // stays visible at 40 columns.
    let mut head = vec![Line::from(vec![
        Span::styled(
            t!("Хост: {0}", host_label(&scenario.host)),
            Style::new().bold(),
        ),
        Span::raw(" · "),
        Span::raw(t!("Сессий приложений: {0}", scenario.sessions)),
    ])];
    if !roomy {
        head.insert(
            0,
            Line::styled(
                format!(
                    "{} · {} · {}",
                    t!("Туннели (макет)"),
                    t!("макет {0}/{1}", page.index + 1, all.len()),
                    scenario.name
                ),
                dim,
            ),
        );
    }
    head.push(Line::from(vec![
        Span::raw(t!("Состояние: {0}", condition_label(&scenario.condition))),
        Span::raw(" · "),
        Span::styled(apps_label(&scenario.apps), dim),
    ]));
    let narrow = inner.width < 56;
    // Below ten rows the column header gives way to the failure line.
    let header = inner.height >= 10;
    let rows = AXES.iter().map(|axis| {
        let value = scenario.axes.get(*axis).map(String::as_str).unwrap_or("");
        let (symbol, label, style) = status(value);
        let age = age_cell(scenario.age_s.get(*axis).copied().flatten(), narrow);
        Row::new(vec![
            Cell::from(axis.to_ascii_uppercase()),
            Cell::from(format!("{symbol} {label}")).style(style),
            Cell::from(age).style(dim),
        ])
    });
    let widths = if narrow {
        [
            Constraint::Length(7),
            Constraint::Min(10),
            Constraint::Length(9),
        ]
    } else {
        [
            Constraint::Length(8),
            Constraint::Length(20),
            Constraint::Min(12),
        ]
    };
    let mut table = Table::new(rows, widths);
    if header {
        table = table.header(Row::new(vec![t!("Ось"), t!("Статус"), t!("Возраст")]).style(dim));
    }
    let failure = scenario
        .failure
        .as_deref()
        .map(|code| t!("Последний сбой: {0}", failure_label(code)));
    let [top, mid, bottom] = Layout::vertical([
        Constraint::Length(head.len() as u16),
        Constraint::Length(if header { 5 } else { 4 }),
        Constraint::Min(0),
    ])
    .areas(inner);
    f.render_widget(Paragraph::new(head), top);
    if page.confirm {
        // While the stop question is open it takes the table's place, so even a
        // 40x12 terminal shows the whole consequence next to the y/n keys.
        let question = Line::styled(
            t!(
                "Остановить туннель? Сессии приложений этого туннеля останутся заблокированными. VPN хоста этой клавишей не переключается."
            ),
            Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        );
        let rest = Rect::new(mid.x, mid.y, mid.width, mid.height + bottom.height);
        f.render_widget(Paragraph::new(question).wrap(Wrap { trim: true }), rest);
        return;
    }
    f.render_widget(table, mid);
    // The failure stays one truncated line: wrapping would push its end off a 12-row screen.
    if let Some(text) = failure {
        f.render_widget(Line::styled(text, Style::new().fg(Color::Red)), bottom);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cm::i18n::{self, Lang};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::path::PathBuf;

    const SIZES: [(u16, u16); 4] = [(40, 12), (60, 18), (80, 24), (120, 32)];
    /// The richest fixture: failure line, blocked axes, sessions.
    const SNAPSHOT_SCENARIO: &str = "host-off-app-active";

    /// Same split as `App::draw`: one header line, three footer lines.
    fn body(width: u16, height: u16) -> Rect {
        Rect::new(0, 1, width, height - 4)
    }

    fn render(lang: Lang, width: u16, height: u16, page: &MockPage) -> String {
        i18n::set_thread(lang);
        let all = scenarios();
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|f| draw(f, body(width, height), page, &all))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..height {
            let mut line = String::new();
            let mut x = 0;
            while x < width {
                let symbol = buffer[(x, y)].symbol();
                line.push_str(symbol);
                // A wide symbol (CJK) covers the next cell too; that cell is not text.
                x += Span::raw(symbol).width().max(1) as u16;
            }
            out.push_str(line.trim_end());
            out.push('\n');
        }
        out
    }

    /// Text without spaces and box borders: CJK cells and wrapping change layout, not content.
    fn squeeze(text: &str) -> String {
        text.chars()
            .filter(|c| !c.is_whitespace() && !('\u{2500}'..='\u{257F}').contains(c))
            .collect()
    }

    fn snapshot_path(lang: Lang, width: u16, height: u16) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            "tests/fixtures/i17/snapshots/{}-{width}x{height}.txt",
            lang.code()
        ))
    }

    fn page_for(name: &str) -> MockPage {
        let index = scenarios().iter().position(|s| s.name == name).unwrap();
        MockPage {
            index,
            confirm: false,
        }
    }

    /// I17-D.T05.a: one snapshot per size and language. `CM_UPDATE_SNAPSHOTS=1` rewrites them.
    #[test]
    fn snapshots_per_size_and_language() {
        let page = page_for(SNAPSHOT_SCENARIO);
        let update = std::env::var("CM_UPDATE_SNAPSHOTS").ok().as_deref() == Some("1");
        for lang in i18n::ALL {
            for (width, height) in SIZES {
                let text = render(lang, width, height, &page);
                for axis in ["NET", "REGION", "STATE", "APP"] {
                    assert!(
                        text.contains(axis),
                        "{}-{width}x{height}: {axis}\n{text}",
                        lang.code()
                    );
                }
                // The failure and live sessions must survive the smallest terminal.
                let failure = t!("Последний сбой: {0}", "");
                let sessions = t!("Сессий приложений: {0}", 1);
                for needle in [failure.trim_end(), sessions.as_str()] {
                    assert!(
                        squeeze(&text).contains(&squeeze(needle)),
                        "{}-{width}x{height}: {needle}\n{text}",
                        lang.code()
                    );
                }
                if lang != Lang::Ru {
                    assert!(
                        !text.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c)),
                        "{}-{width}x{height}: untranslated text\n{text}",
                        lang.code()
                    );
                }
                let path = snapshot_path(lang, width, height);
                if update {
                    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                    std::fs::write(&path, &text).unwrap();
                    continue;
                }
                let expected = std::fs::read_to_string(&path)
                    .unwrap_or_else(|_| panic!("missing snapshot {}", path.display()));
                assert_eq!(text, expected, "{}", path.display());
            }
        }
    }

    #[test]
    fn every_fixture_renders_four_axes_and_no_ok_summary() {
        for (index, scenario) in scenarios().iter().enumerate() {
            let page = MockPage {
                index,
                confirm: false,
            };
            let text = render(Lang::En, 80, 24, &page);
            for axis in ["NET", "REGION", "STATE", "APP"] {
                assert!(text.contains(axis), "{}: {axis}", scenario.name);
            }
            for word in ["OK", "Protected", "Anonymous", "Secure"] {
                assert!(!text.contains(word), "{}: {word}\n{text}", scenario.name);
            }
        }
    }

    #[test]
    fn evidence_age_is_shown_and_stale_is_marked() {
        i18n::set_thread(Lang::Ru);
        assert_eq!(age_text(None), "—");
        assert_eq!(age_text(Some(40)), "40 с");
        assert_eq!(age_text(Some(300)), "5 мин");
        assert!(age_text(Some(5400)).ends_with("устарело"));
        assert_eq!(age_cell(Some(5400), true), "1 ч ⚠");
        let text = render(Lang::Ru, 80, 24, &page_for("host-proxy"));
        assert!(text.contains("40 с"), "{text}");
        assert!(text.contains("устарело"), "{text}");
    }

    #[test]
    fn stop_question_names_the_consequence_and_waits() {
        let all = scenarios();
        let mut page = MockPage::default();
        assert!(matches!(
            page.key(KeyCode::Char('x'), all.len()),
            Outcome::Stay
        ));
        assert!(page.confirm);
        // Other keys keep the question; q does not leave the page.
        assert!(matches!(
            page.key(KeyCode::Char('q'), all.len()),
            Outcome::Stay
        ));
        assert!(page.confirm);
        for (width, height) in SIZES {
            let text = render(Lang::En, width, height, &page);
            assert!(
                squeeze(&text).contains("thistunnelstayblocked.ThiskeydoesnotswitchthehostVPN."),
                "{width}x{height}\n{text}"
            );
        }
        assert!(matches!(
            page.key(KeyCode::Esc, all.len()),
            Outcome::Message(_)
        ));
        assert!(!page.confirm);
        page.key(KeyCode::Char('x'), all.len());
        assert!(matches!(
            page.key(KeyCode::Enter, all.len()),
            Outcome::Message(_)
        ));
        assert!(matches!(
            page.key(KeyCode::Char('q'), all.len()),
            Outcome::Back
        ));
    }
}
