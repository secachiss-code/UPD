//! Мелкие строительные блоки интерфейса: ряды с учётом направления текста, баннеры, приглушённый текст.

use crate::model::rtl;
use cosmic::iced::{Alignment, Color, Length};
use cosmic::widget::{self, button, text};
use cosmic::{Element, Theme};

pub type Text<'a> = widget::Text<'a, Theme, cosmic::Renderer>;

/// Текст с выравниванием по направлению письма (арабский — по правому краю).
pub fn txt(t: Text<'_>) -> Text<'_> {
    if rtl() { t.align_x(cosmic::iced::alignment::Horizontal::Right).width(Length::Fill) } else { t }
}

/// Ряд слева направо; для арабского порядок элементов зеркалится.
pub fn hrow<'a, M: 'a>(mut children: Vec<Element<'a, M>>) -> widget::Row<'a, M, Theme, cosmic::Renderer> {
    if rtl() {
        children.reverse();
    }
    widget::Row::with_children(children).spacing(cosmic::theme::spacing().space_xs).align_y(Alignment::Center)
}

/// Второстепенный текст (устаревшее число на панели).
pub fn dim_text(theme: &Theme) -> cosmic::iced::widget::text::Style {
    let mut c: Color = theme.cosmic().on_bg_color().into();
    c.a = 0.55;
    cosmic::iced::widget::text::Style { color: Some(c), selected_fill: Color::TRANSPARENT }
}

/// Карточка-предупреждение со значком, текстом и необязательной кнопкой.
pub fn banner<'a, M: Clone + 'static>(icon: &'static str, message: String, action: Option<(&'a str, M)>) -> Element<'a, M> {
    let sp = cosmic::theme::spacing();
    let mut items: Vec<Element<'a, M>> = vec![
        widget::icon::from_name(icon).size(16).into(),
        widget::container(widget::scrollable(txt(text::body(message)).width(Length::Fill).wrapping(cosmic::iced::widget::text::Wrapping::WordOrGlyph)).height(Length::Shrink)).max_height(96.0).width(Length::Fill).into(),
    ];
    if let Some((label, m)) = action {
        items.push(button::text(label).on_press(m).into());
    }
    widget::container(hrow(items))
        .padding([sp.space_xs, sp.space_s])
        .class(cosmic::theme::Container::Card)
        .width(Length::Fill)
        .apply(|c| widget::container(c).padding([sp.space_xxs, sp.space_s]))
        .into()
}

/// Строка предупреждения во всплывающем окне: значок, текст, одно действие.
pub fn warn_row<'a, M: 'static>(icon: &'static str, message: String, control: Element<'a, M>) -> Element<'a, M> {
    hrow(vec![widget::icon::from_name(icon).size(16).into(), txt(text::body(message)).width(Length::Fill).into(), control]).into()
}

trait Apply: Sized {
    fn apply<R>(self, f: impl FnOnce(Self) -> R) -> R {
        f(self)
    }
}

impl<T> Apply for T {}
