//! Назначение приложения доводится до профиля подключения и источника.

use crate::core::instance::InstanceId;
use crate::net::host_plan::HOST_INSTANCE;
use crate::profiles::{DnsPolicy, GraphSnapshot, Id, KernelKind, NodeSelection, TunnelOwner};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedTunnel {
    pub tunnel_instance_id: Id,
    pub instance: String,
    pub generation: u64,
    pub connection_profile_id: Id,
    pub source_id: Id,
    pub selection: NodeSelection,
    pub kernel: KernelKind,
    pub dns: DnsPolicy,
    pub owner: TunnelOwner,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedApp {
    pub application_id: Id,
    pub tunnel: ResolvedTunnel,
    pub shared: bool,
    pub preset_id: Id,
    pub data_profile_id: Id,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssignError {
    UnknownApplication,
    UnknownGroup,
    UnknownTunnel,
    UnknownProfile,
    UnknownSource,
    HostTunnel,
    OwnerMismatch,
    BadInstanceName,
}

pub fn tunnel(snapshot: &GraphSnapshot, tunnel_id: &Id) -> Result<ResolvedTunnel, AssignError> {
    let instance = snapshot
        .tunnel_instances
        .get(tunnel_id)
        .ok_or(AssignError::UnknownTunnel)?;
    let name = tunnel_id.as_str();
    if name == HOST_INSTANCE || InstanceId::new(name).is_err() {
        return Err(AssignError::BadInstanceName);
    }
    let profile = snapshot
        .connection_profiles
        .get(&instance.connection_profile_id)
        .ok_or(AssignError::UnknownProfile)?;
    if !snapshot.sources.contains_key(&profile.source_id) {
        return Err(AssignError::UnknownSource);
    }
    Ok(ResolvedTunnel {
        tunnel_instance_id: instance.id.clone(),
        instance: name.to_owned(),
        generation: instance.generation,
        connection_profile_id: profile.id.clone(),
        source_id: profile.source_id.clone(),
        selection: profile.node_selection.clone(),
        kernel: profile.kernel,
        dns: profile.dns.clone(),
        owner: instance.owner.clone(),
    })
}

pub fn application(snapshot: &GraphSnapshot, app_id: &Id) -> Result<ResolvedApp, AssignError> {
    let application = snapshot
        .applications
        .get(app_id)
        .ok_or(AssignError::UnknownApplication)?;
    let (resolved, shared) = match &application.assignment {
        crate::profiles::ApplicationAssignment::OwnTunnel { tunnel_instance_id } => {
            let resolved = tunnel(snapshot, tunnel_instance_id)?;
            if matches!(resolved.owner, TunnelOwner::Host) {
                return Err(AssignError::HostTunnel);
            }
            match &resolved.owner {
                TunnelOwner::Application { application_id } if application_id == app_id => {}
                _ => return Err(AssignError::OwnerMismatch),
            }
            (resolved, false)
        }
        crate::profiles::ApplicationAssignment::Group { group_id } => {
            let group = snapshot
                .application_groups
                .get(group_id)
                .ok_or(AssignError::UnknownGroup)?;
            let resolved = tunnel(snapshot, &group.tunnel_instance_id)?;
            if matches!(resolved.owner, TunnelOwner::Host) {
                return Err(AssignError::HostTunnel);
            }
            match &resolved.owner {
                TunnelOwner::Group { group_id: owner } if owner == group_id => {}
                _ => return Err(AssignError::OwnerMismatch),
            }
            (resolved, true)
        }
    };
    Ok(ResolvedApp {
        application_id: application.id.clone(),
        tunnel: resolved,
        shared,
        preset_id: application.environment_preset_id.clone(),
        data_profile_id: application.data_profile_id.clone(),
    })
}
