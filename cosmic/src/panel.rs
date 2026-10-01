//! Native COSMIC symbols and compact counters at the panel's actual icon size.
use cosmic::widget::{self, Icon};
use upd::summary::Badge;

pub fn name(badge: &Badge) -> &'static str {
    match badge {
        Badge::Idle => "emblem-ok-symbolic",
        Badge::Updates(_) => "software-update-available-symbolic",
        Badge::Busy(_) => "emblem-synchronizing-symbolic",
        Badge::Waiting => "media-playback-pause-symbolic",
        Badge::Error => "dialog-warning-symbolic",
        Badge::Reboot => "system-reboot-symbolic",
        Badge::Stale(_) => "appointment-soon-symbolic",
        Badge::Unverified => "dialog-question-symbolic",
    }
}

pub fn icon(badge: &Badge, size: u16) -> Icon {
    widget::icon::from_name(name(badge))
        .symbolic(true)
        .size(size)
        .icon()
        .size(size)
        .opacity(if matches!(badge, Badge::Stale(_)) {
            0.65
        } else {
            1.0
        })
}

pub fn button<'a, M: Clone + 'static>(
    badge: &Badge,
    size: u16,
    padding: (u16, u16),
    horizontal: bool,
) -> widget::Button<'a, M> {
    use cosmic::iced::{Alignment, Border, Color, Length};
    let mut row = widget::row::with_capacity(2)
        .push(icon(badge, size))
        .spacing(5)
        .align_y(Alignment::Center);
    if let Some(label) = counter(badge).filter(|_| horizontal) {
        let width = (label.len() as f32 * 7.0 + 10.0).max(28.0);
        let text = widget::text(label)
            .size(if size < 20 { 11.0 } else { 12.0 })
            .font(cosmic::font::semibold())
            .line_height(1.0);
        let dim = matches!(badge, Badge::Stale(_));
        let count = widget::container(text)
            .width(Length::Fixed(width))
            .height(Length::Fixed(f32::from(size.min(20))))
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .class(cosmic::theme::Container::custom(move |theme| {
                let mut foreground: Color = theme.cosmic().background(theme.transparent).on.into();
                let mut background = foreground;
                background.a = 0.12;
                if dim {
                    foreground.a = 0.65;
                }
                cosmic::iced::widget::container::Style {
                    text_color: Some(foreground),
                    background: Some(background.into()),
                    border: Border {
                        radius: 5.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }
            }));
        row = row.push(count);
    }
    widget::button::custom(row)
        .class(cosmic::theme::Button::AppletIcon)
        .padding([padding.1, padding.0])
        .height(Length::Fixed(f32::from(size + 2 * padding.1)))
}

pub fn counter(badge: &Badge) -> Option<String> {
    match badge {
        Badge::Busy(Some((n, m))) if *n > 0 && n <= m => Some(format!("{n}/{m}")),
        Badge::Updates(n) | Badge::Stale(n) if *n > 0 => {
            Some(if *n > 99 { "99+".into() } else { n.to_string() })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn states_have_distinct_icons_and_bounded_counters() {
        let badges = [
            Badge::Idle,
            Badge::Updates(7),
            Badge::Busy(Some((3, 6))),
            Badge::Waiting,
            Badge::Error,
            Badge::Reboot,
            Badge::Stale(7),
            Badge::Unverified,
        ];
        for (i, badge) in badges.iter().enumerate() {
            assert!(name(badge).ends_with("-symbolic"));
            assert!(badges[..i].iter().all(|other| name(other) != name(badge)));
        }
        assert_eq!(counter(&Badge::Updates(1500)).as_deref(), Some("99+"));
        assert_eq!(counter(&Badge::Busy(Some((3, 6)))).as_deref(), Some("3/6"));
        assert_eq!(counter(&Badge::Stale(0)), None);
        assert_eq!(counter(&Badge::Unverified), None);
        assert_eq!(counter(&Badge::Busy(Some((0, 0)))), None);
    }
}
