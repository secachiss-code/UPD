//! Живое соединение не переадресуется молча: смена назначения требует перезапуска.

use crate::profiles::{ApplicationAssignment, Session, SessionLifecycle};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reassign {
    NoChange,
    AppliesToNextLaunch,
    RestartRequired { reason: ReassignReason },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReassignReason {
    TunnelChanged,
    GenerationChanged,
    NodeGone,
}

pub fn decide(
    session: Option<&Session>,
    current: &ApplicationAssignment,
    wanted: &ApplicationAssignment,
    tunnel_generation: u64,
    node_present: bool,
) -> Reassign {
    if !node_present {
        return Reassign::RestartRequired {
            reason: ReassignReason::NodeGone,
        };
    }
    let Some(session) = session.filter(|session| session.lifecycle == SessionLifecycle::Active)
    else {
        return if current == wanted {
            Reassign::NoChange
        } else {
            Reassign::AppliesToNextLaunch
        };
    };
    if assignment_target(current) != assignment_target(wanted) {
        return Reassign::RestartRequired {
            reason: ReassignReason::TunnelChanged,
        };
    }
    if tunnel_generation != session.tunnel_generation {
        return Reassign::RestartRequired {
            reason: ReassignReason::GenerationChanged,
        };
    }
    Reassign::NoChange
}

fn assignment_target(assignment: &ApplicationAssignment) -> String {
    match assignment {
        ApplicationAssignment::OwnTunnel { tunnel_instance_id } => {
            format!("tunnel:{}", tunnel_instance_id.as_str())
        }
        ApplicationAssignment::Group { group_id } => format!("group:{}", group_id.as_str()),
    }
}
