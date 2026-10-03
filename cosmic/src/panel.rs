//! Native COSMIC symbols and compact counters at the panel's actual icon size.
use cosmic::widget::{self, Icon};
use cm::summary::Badge;

/// Открытый TUI виден как работа; ошибки и перезагрузка сохраняют приоритет.
pub fn activity_badge(badge: Badge, tui_running: bool, error: bool) -> Badge {
    if error { return Badge::Error; }
    if tui_running && matches!(badge, Badge::Idle | Badge::Updates(_) | Badge::Stale(_) | Badge::Unverified) {
        Badge::Busy(None)
    } else { badge }
}

#[cfg(test)]
pub fn name(badge: &Badge) -> &'static str {
    match badge {
        Badge::Idle => "cm-idle-symbolic",
        Badge::Updates(_) => "cm-updates-symbolic",
        Badge::Busy(_) => "cm-busy-symbolic",
        Badge::Waiting => "cm-waiting-symbolic",
        Badge::Error => "cm-error-symbolic",
        Badge::Reboot => "cm-reboot-symbolic",
        Badge::Stale(_) => "cm-stale-symbolic",
        Badge::Unverified => "cm-unverified-symbolic",
    }
}

pub fn icon(badge: &Badge, size: u16) -> Icon {
    let bytes: &'static [u8] = match badge {
        Badge::Idle => include_bytes!("../res/icons/cm-idle-symbolic.svg"),
        Badge::Updates(_) => include_bytes!("../res/icons/cm-updates-symbolic.svg"),
        Badge::Busy(_) => include_bytes!("../res/icons/cm-busy-symbolic.svg"),
        Badge::Waiting => include_bytes!("../res/icons/cm-waiting-symbolic.svg"),
        Badge::Error => include_bytes!("../res/icons/cm-error-symbolic.svg"),
        Badge::Reboot => include_bytes!("../res/icons/cm-reboot-symbolic.svg"),
        Badge::Stale(_) => include_bytes!("../res/icons/cm-stale-symbolic.svg"),
        Badge::Unverified => include_bytes!("../res/icons/cm-unverified-symbolic.svg"),
    };
    let mut handle = widget::icon::from_svg_bytes(bytes);
    handle.symbolic = true;
    handle.icon().size(size).opacity(if matches!(badge, Badge::Stale(_)) { 0.65 } else { 1.0 })
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
    fn open_tui_is_visible_without_hiding_errors_or_operations() {
        assert!(matches!(activity_badge(Badge::Idle, true, false), Badge::Busy(None)));
        assert!(matches!(activity_badge(Badge::Updates(4), true, false), Badge::Busy(None)));
        assert!(matches!(activity_badge(Badge::Idle, false, false), Badge::Idle));
        assert!(matches!(activity_badge(Badge::Error, true, false), Badge::Error));
        assert!(matches!(activity_badge(Badge::Reboot, true, false), Badge::Reboot));
        assert!(matches!(activity_badge(Badge::Waiting, true, false), Badge::Waiting));
        assert!(matches!(activity_badge(Badge::Busy(Some((2, 4))), true, false), Badge::Busy(Some((2, 4)))));
        assert!(matches!(activity_badge(Badge::Idle, true, true), Badge::Error));
    }
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
