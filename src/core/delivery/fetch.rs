//! Получение бинарника: загрузка, хеш архива, распаковка, хеш бинарника. Первый отказ прерывает цепочку.

use std::time::Duration;

use crate::sources::negotiation::{ConfiguredEndpoint, RequestSpec, UserAgent};
use crate::sources::transport::FetchTransport;

use super::pin::{Archive, DeliveryError, Pin, verify_sha256};
use super::unpack::{unpack_gzip, unpack_zip_member};

const DOWNLOAD_DEADLINE: Duration = Duration::from_secs(600);

pub trait Download {
    fn get(&self, url: &str, max_bytes: u64) -> Result<Vec<u8>, DeliveryError>;
}

pub struct TransportDownload;

impl Download for TransportDownload {
    fn get(&self, url: &str, max_bytes: u64) -> Result<Vec<u8>, DeliveryError> {
        if !url.starts_with("https://") {
            return Err(DeliveryError::Fetch);
        }
        let max = usize::try_from(max_bytes).map_err(|_| DeliveryError::TooLarge)?;
        let endpoint = ConfiguredEndpoint::new(url, false).map_err(|_| DeliveryError::Fetch)?;
        let agent = UserAgent::new("cm").map_err(|_| DeliveryError::Fetch)?;
        let spec = RequestSpec::for_bounded_get(&endpoint, &agent, DOWNLOAD_DEADLINE, max);
        let response = FetchTransport::new()
            .fetch(spec)
            .map_err(|_| DeliveryError::Fetch)?;
        if !(200..300).contains(&response.status()) {
            return Err(DeliveryError::Fetch);
        }
        let body = response.body();
        if body.len() > max {
            return Err(DeliveryError::TooLarge);
        }
        Ok(body.to_vec())
    }
}

pub fn obtain(pin: &Pin, download: &dyn Download) -> Result<Vec<u8>, DeliveryError> {
    let archive = download.get(pin.url, pin.max_archive)?;
    verify_sha256(&archive, pin.archive_sha256)?;
    let binary = match pin.archive {
        Archive::Gzip => unpack_gzip(&archive, pin.max_binary)?,
        Archive::Zip { member } => unpack_zip_member(&archive, member, pin.max_binary)?,
    };
    verify_sha256(&binary, pin.binary_sha256)?;
    Ok(binary)
}
