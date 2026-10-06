//! systemd unit text for one core instance.
//!
//! Capabilities are present only when the instance needs them. The unit is not
//! installed by this module.

use super::instance::{InstanceId, SYSTEM_ROOT};

/// Verified core binaries live outside every instance's writable directory: a compromised
/// core cannot replace the binary it will be restarted from.
pub const CORE_BIN: &str = "/var/lib/cm/cores/mihomo/mihomo";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NetPrivileges {
    pub admin: bool,
    pub raw: bool,
    pub bind_service: bool,
}

pub fn render_core_unit(id: &InstanceId, privileges: NetPrivileges) -> String {
    let root = format!("{SYSTEM_ROOT}/instances/{}", id.as_str());
    let caps = capability_list(privileges);
    let capability_lines = if caps.is_empty() {
        "CapabilityBoundingSet=\n".to_owned()
    } else {
        format!("CapabilityBoundingSet={caps}\nAmbientCapabilities={caps}\n")
    };
    // TUN needs /dev/net/tun only when the instance may administer the network.
    let device_lines = if privileges.admin {
        "DevicePolicy=closed\nDeviceAllow=/dev/net/tun rw\n"
    } else {
        "PrivateDevices=yes\n"
    };
    format!(
        "[Unit]\n\
         Description=cm core instance {id}\n\
         Wants=network-online.target\n\
         After=network-online.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={bin} -d {root} -f {root}/config/config.yaml\n\
         Restart=on-failure\n\
         RestartSec=5\n\
         TimeoutStartSec=5min\n\
         LimitNOFILE=1048576\n\
         UMask=0077\n\
         NoNewPrivileges=yes\n\
         {capability_lines}\
         ProtectSystem=strict\n\
         ReadWritePaths=-{root}\n\
         RuntimeDirectory=cm-core-{id}\n\
         RuntimeDirectoryMode=0700\n\
         ProtectHome=yes\n\
         PrivateTmp=yes\n\
         ProtectKernelModules=yes\n\
         ProtectControlGroups=yes\n\
         ProtectKernelTunables=yes\n\
         ProtectKernelLogs=yes\n\
         ProtectClock=yes\n\
         ProtectHostname=yes\n\
         RestrictNamespaces=yes\n\
         RestrictRealtime=yes\n\
         RestrictSUIDSGID=yes\n\
         LockPersonality=yes\n\
         SystemCallArchitectures=native\n\
         RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6 AF_NETLINK\n\
         {device_lines}\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        id = id.as_str(),
        bin = CORE_BIN,
    )
}

fn capability_list(privileges: NetPrivileges) -> String {
    let mut caps = Vec::new();
    if privileges.admin {
        caps.push("CAP_NET_ADMIN");
    }
    if privileges.raw {
        caps.push("CAP_NET_RAW");
    }
    if privileges.bind_service {
        caps.push("CAP_NET_BIND_SERVICE");
    }
    caps.join(" ")
}

/// Hardening directives the legacy `cm-vpn.service` already has.
pub fn legacy_hardening_directives() -> &'static [&'static str] {
    &[
        "NoNewPrivileges=yes",
        "ProtectSystem=strict",
        "ProtectHome=yes",
        "PrivateTmp=yes",
        "ProtectKernelModules=yes",
        "ProtectControlGroups=yes",
        "UMask=0077",
    ]
}
