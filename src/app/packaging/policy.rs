//! Обещание CM для способа запуска. Flatpak и Snap не проверены на настоящих приложениях.

use super::detect::Packaging;
use crate::env::private::{DataState, IpcPolicy};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit {
    SessionBus,
    FuseHelper,
    DataOutsideXdg,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    ExecutorOutsideNetns,
    NotVerified,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Support {
    Supported,
    Partial(Vec<Limit>),
    Unsupported(Reason),
}

pub fn support(packaging: &Packaging, ipc: IpcPolicy, data: DataState) -> Support {
    match packaging {
        Packaging::Launcher => Support::Unsupported(Reason::ExecutorOutsideNetns),
        Packaging::Flatpak { .. } | Packaging::Snap => Support::Unsupported(Reason::NotVerified),
        Packaging::AppImage => Support::Partial(limits(true, ipc, data)),
        Packaging::Native => {
            let found = limits(false, ipc, data);
            if found.is_empty() {
                Support::Supported
            } else {
                Support::Partial(found)
            }
        }
    }
}

pub fn limit_token(limit: Limit) -> &'static str {
    match limit {
        Limit::SessionBus => "limit_session_bus",
        Limit::FuseHelper => "limit_fuse_helper",
        Limit::DataOutsideXdg => "limit_data_outside_xdg",
    }
}

pub fn reason_token(reason: Reason) -> &'static str {
    match reason {
        Reason::ExecutorOutsideNetns => "unsupported_executor_outside",
        Reason::NotVerified => "unsupported_not_verified",
    }
}

fn limits(fuse: bool, ipc: IpcPolicy, data: DataState) -> Vec<Limit> {
    let mut found = Vec::new();
    if fuse {
        found.push(Limit::FuseHelper);
    }
    if ipc == IpcPolicy::Shared {
        found.push(Limit::SessionBus);
    }
    if data != DataState::Existing {
        found.push(Limit::DataOutsideXdg);
    }
    found
}
