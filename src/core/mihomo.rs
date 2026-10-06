//! Pinned mihomo descriptor. The tables are the I03 capability slices, not a second copy.

use crate::profiles::NodeProtocol;
use crate::sources::{
    ImportFormat, PINNED_CORE_COMMIT, PINNED_CORE_VERSION, Transport, UriSchemeCapability,
    supported_import_formats, supported_transports, supported_uri_schemes,
};

/// Lifecycle claim for mihomo 1.19.32. Protocol support is queried from I03.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MihomoDescriptor {
    pub version: &'static str,
    pub commit: &'static str,
    pub reload_without_restart: bool,
    pub delay_probe: bool,
}

pub fn descriptor() -> MihomoDescriptor {
    MihomoDescriptor {
        version: PINNED_CORE_VERSION,
        commit: PINNED_CORE_COMMIT,
        reload_without_restart: true,
        delay_probe: true,
    }
}

pub fn import_formats() -> &'static [ImportFormat] {
    supported_import_formats()
}

pub fn uri_schemes() -> &'static [UriSchemeCapability] {
    supported_uri_schemes()
}

pub fn transports(
    protocol: NodeProtocol,
) -> Result<&'static [Transport], crate::sources::CapabilityError> {
    supported_transports(protocol)
}
