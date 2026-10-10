//! Счётчик держателей общего туннеля. Туннель хоста сессиями не останавливается.

use std::collections::{BTreeMap, BTreeSet};

use crate::profiles::{Id, Session, SessionLifecycle, TunnelOwner};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TunnelRefs {
    holders: BTreeMap<Id, BTreeSet<Id>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefAction {
    None,
    StartTunnel(Id),
    StopTunnel(Id),
}

impl TunnelRefs {
    pub fn acquire(&mut self, tunnel: &Id, session: &Id) -> RefAction {
        let set = self.holders.entry(tunnel.clone()).or_default();
        if !set.insert(session.clone()) {
            return RefAction::None;
        }
        if set.len() == 1 {
            RefAction::StartTunnel(tunnel.clone())
        } else {
            RefAction::None
        }
    }

    pub fn release(&mut self, tunnel: &Id, session: &Id, owner: &TunnelOwner) -> RefAction {
        let Some(set) = self.holders.get_mut(tunnel) else {
            return RefAction::None;
        };
        if !set.remove(session) {
            return RefAction::None;
        }
        if !set.is_empty() {
            return RefAction::None;
        }
        self.holders.remove(tunnel);
        if matches!(owner, TunnelOwner::Host) {
            RefAction::None
        } else {
            RefAction::StopTunnel(tunnel.clone())
        }
    }

    pub fn holders(&self, tunnel: &Id) -> usize {
        self.holders.get(tunnel).map(BTreeSet::len).unwrap_or(0)
    }
}

pub fn rebuild(sessions: &[Session]) -> TunnelRefs {
    let mut refs = TunnelRefs::default();
    for session in sessions
        .iter()
        .filter(|session| session.lifecycle == SessionLifecycle::Active)
    {
        refs.acquire(&session.tunnel_instance_id, &session.id);
    }
    refs
}
