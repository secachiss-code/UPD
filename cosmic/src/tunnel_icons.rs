//! I17-D.T04.a: applet symbols for app tunnels.
//!
//! Mock stage: the applet shows them once the controller reports real tunnel
//! states (I17). Until then only the tests use them, but the build embeds every
//! file, so a missing or renamed SVG fails `cargo build`.
#![cfg_attr(not(test), allow(dead_code))]

use cosmic::widget::{self, Icon};

/// Tunnel states that the update badge cannot express.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TunnelBadge {
    /// Host VPN is off while at least one app tunnel is alive.
    HostOffAppActive,
    /// Some axes or some tunnels failed, others work.
    Partial,
    /// Traffic of the tunnel is blocked (fail-closed).
    Blocked,
}

impl TunnelBadge {
    pub const ALL: [Self; 3] = [Self::HostOffAppActive, Self::Partial, Self::Blocked];

    pub fn name(self) -> &'static str {
        match self {
            Self::HostOffAppActive => "cm-tunnel-app-only-symbolic",
            Self::Partial => "cm-tunnel-partial-symbolic",
            Self::Blocked => "cm-tunnel-blocked-symbolic",
        }
    }

    pub fn svg(self) -> &'static [u8] {
        match self {
            Self::HostOffAppActive => {
                include_bytes!("../res/icons/cm-tunnel-app-only-symbolic.svg")
            }
            Self::Partial => include_bytes!("../res/icons/cm-tunnel-partial-symbolic.svg"),
            Self::Blocked => include_bytes!("../res/icons/cm-tunnel-blocked-symbolic.svg"),
        }
    }
}

/// Worst tunnel condition wins. An empty list leaves the update icon alone.
pub fn badge_for(views: &[cm::status::tunnels::TunnelView]) -> Option<TunnelBadge> {
    if views.iter().any(|view| view.condition == "blocked") {
        return Some(TunnelBadge::Blocked);
    }
    if views.iter().any(|view| view.condition == "degraded") {
        return Some(TunnelBadge::Partial);
    }
    if views
        .iter()
        .any(|view| view.host == "off" && view.sessions > 0)
    {
        return Some(TunnelBadge::HostOffAppActive);
    }
    None
}

/// Symbolic, so light and dark panel themes recolour it like the other badges.
pub fn icon(badge: TunnelBadge, size: u16) -> Icon {
    let mut handle = widget::icon::from_svg_bytes(badge.svg());
    handle.symbolic = true;
    handle.icon().size(size)
}

#[cfg(test)]
mod tests {
    use super::*;

    const UPDATE_BADGES: [&[u8]; 8] = [
        include_bytes!("../res/icons/cm-idle-symbolic.svg"),
        include_bytes!("../res/icons/cm-updates-symbolic.svg"),
        include_bytes!("../res/icons/cm-busy-symbolic.svg"),
        include_bytes!("../res/icons/cm-waiting-symbolic.svg"),
        include_bytes!("../res/icons/cm-error-symbolic.svg"),
        include_bytes!("../res/icons/cm-reboot-symbolic.svg"),
        include_bytes!("../res/icons/cm-stale-symbolic.svg"),
        include_bytes!("../res/icons/cm-unverified-symbolic.svg"),
    ];

    fn view(host: &str, sessions: u32, condition: &str) -> cm::status::tunnels::TunnelView {
        cm::status::tunnels::TunnelView {
            name: "t".to_owned(),
            host: host.to_owned(),
            apps: "separate".to_owned(),
            condition: condition.to_owned(),
            axes: Default::default(),
            age_s: Default::default(),
            failure: None,
            sessions,
        }
    }

    /// M16: the worst tunnel condition wins; without tunnels the applet keeps its icon.
    #[test]
    fn badge_reflects_the_worst_condition() {
        assert_eq!(badge_for(&[]), None);
        assert_eq!(
            badge_for(&[view("proxy", 1, "blocked"), view("proxy", 1, "degraded")]),
            Some(TunnelBadge::Blocked)
        );
        assert_eq!(
            badge_for(&[view("proxy", 1, "degraded"), view("off", 2, "unknown")]),
            Some(TunnelBadge::Partial)
        );
        assert_eq!(
            badge_for(&[view("off", 2, "unknown")]),
            Some(TunnelBadge::HostOffAppActive)
        );
        assert_eq!(badge_for(&[view("tunnel", 2, "unknown")]), None);
        assert_eq!(badge_for(&[view("off", 0, "unknown")]), None);
    }

    #[test]
    fn tunnel_symbols_are_symbolic_16px_and_distinct() {
        let mut seen: Vec<&[u8]> = UPDATE_BADGES.to_vec();
        for badge in TunnelBadge::ALL {
            let text = std::str::from_utf8(badge.svg()).unwrap();
            assert!(text.starts_with("<svg "), "{}", badge.name());
            assert!(text.contains(r#"viewBox="0 0 16 16""#), "{}", badge.name());
            // One colour only: the theme replaces it, so the icon works on light and dark panels.
            let colours: Vec<&str> = text
                .match_indices('#')
                .map(|(at, _)| &text[at..at + 7])
                .collect();
            assert!(
                colours.iter().all(|colour| *colour == "#2e3436"),
                "{}: {colours:?}",
                badge.name()
            );
            assert!(
                !seen.contains(&badge.svg()),
                "{} repeats an icon",
                badge.name()
            );
            seen.push(badge.svg());
            assert!(badge.name().ends_with("-symbolic"));
            let _ = icon(badge, 16);
        }
    }
}
