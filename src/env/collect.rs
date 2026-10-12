//! Проба страны идёт из сети туннеля: тот же путь, что у приложения.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::common::test_mode;
use crate::controller::client::{self, ClientError};
use crate::controller::protocol::{Op, ReplyData};
use crate::sources::negotiation::{ConfiguredEndpoint, RequestSpec, UserAgent};
use crate::sources::transport::FetchTransport;

use super::geo::Observation;
use super::sources::{self, GeoSource, MAX_GEO_BODY};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollectError {
    Launch,
    Timeout,
    Malformed,
    BadPath,
}

pub const MAX_RESULT_BYTES: usize = 16 << 10;
pub const PROBE_TIMEOUT_MS: u64 = 5_000;

pub fn probe_main(args: &[String]) -> i32 {
    if args.first().map(String::as_str) != Some("probe") {
        return 2;
    }
    let mut out = None;
    let mut sources = Vec::new();
    let mut rest = args[1..].iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--out" => {
                let Some(value) = rest.next() else {
                    return 2;
                };
                out = Some(value.clone());
            }
            "--source" => {
                let Some(value) = rest.next() else {
                    return 2;
                };
                let Some(source) = sources::parse_source_arg(value) else {
                    return 2;
                };
                sources.push(source);
            }
            _ => return 2,
        }
    }
    let Some(out) = out else {
        return 2;
    };
    if !out_allowed(Path::new(&out)) {
        return 2;
    }
    let observations = probe_sources(&sources);
    let bytes = serde_json::to_vec(&observations).unwrap_or_else(|_| b"[]".to_vec());
    if write_private(Path::new(&out), &bytes).is_err() {
        return 2;
    }
    0
}

pub fn read_result(path: &Path) -> Result<Vec<Observation>, CollectError> {
    let bytes = fs::read(path).map_err(|_| CollectError::Malformed)?;
    if bytes.len() > MAX_RESULT_BYTES {
        return Err(CollectError::Malformed);
    }
    serde_json::from_slice(&bytes).map_err(|_| CollectError::Malformed)
}

pub fn collect(
    socket: &Path,
    instance: &str,
    sources: &[GeoSource],
    runtime: &Path,
    wait_ms: u64,
) -> Result<Vec<Observation>, CollectError> {
    let dir = runtime.join("cm");
    fs::create_dir_all(&dir).map_err(|_| CollectError::BadPath)?;
    let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
    let name = format!("probe-{}.json", random_hex(6)?);
    let path = dir.join(name);
    let status = client::call(
        socket,
        Op::WorkerStatus {
            instance: instance.to_owned(),
        },
    )
    .map_err(|_| CollectError::Launch)?;
    let Some(ReplyData::Status {
        generation: Some(generation),
        running: true,
        ..
    }) = status.data
    else {
        return Err(CollectError::Launch);
    };
    if !status.ok {
        return Err(CollectError::Launch);
    }
    let program = std::env::current_exe().map_err(|_| CollectError::Launch)?;
    let mut args = vec![
        "env".to_owned(),
        "probe".to_owned(),
        "--out".to_owned(),
        path.to_string_lossy().into_owned(),
    ];
    for source in sources {
        args.push("--source".to_owned());
        args.push(source_arg(source));
    }
    let launched = client::call(
        socket,
        Op::AppLaunch {
            instance: instance.to_owned(),
            generation,
            program: program.to_string_lossy().into_owned(),
            args,
            env: Default::default(),
            session: None,
        },
    )
    .map_err(|error| match error {
        ClientError::Unavailable | ClientError::BadReply => CollectError::Launch,
    })?;
    if !launched.ok {
        return Err(CollectError::Launch);
    }
    let steps = wait_ms / 100;
    for _ in 0..=steps {
        if path.is_file() {
            let result = read_result(&path);
            let _ = fs::remove_file(&path);
            return result;
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err(CollectError::Timeout)
}

fn probe_sources(sources: &[GeoSource]) -> Vec<Observation> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0);
    let transport = FetchTransport::new();
    let mut found = Vec::new();
    for source in sources {
        let Ok(endpoint) = ConfiguredEndpoint::new(&source.url, test_mode()) else {
            continue;
        };
        let Ok(agent) = UserAgent::new("cm") else {
            continue;
        };
        let spec = RequestSpec::for_bounded_get(
            &endpoint,
            &agent,
            Duration::from_millis(PROBE_TIMEOUT_MS),
            MAX_GEO_BODY,
        );
        let Ok(response) = transport.fetch(spec) else {
            continue;
        };
        if response.status() != 200 {
            continue;
        }
        if let Some(country) = sources::parse(&source.format, response.body()) {
            found.push(Observation {
                source: source.id.clone(),
                country,
                at_unix_ms: now,
            });
        }
    }
    found
}

fn out_allowed(path: &Path) -> bool {
    let Ok(runtime) = std::env::var("XDG_RUNTIME_DIR") else {
        return false;
    };
    if runtime.is_empty() {
        return false;
    }
    let root = PathBuf::from(runtime).join("cm");
    path.starts_with(&root) && path != root && !path.exists()
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(tmp, path)
}

fn source_arg(source: &GeoSource) -> String {
    let format = match &source.format {
        sources::GeoFormat::Trace => "trace".to_owned(),
        sources::GeoFormat::Plain => "plain".to_owned(),
        sources::GeoFormat::Json(field) => format!("json:{field}"),
    };
    format!("{}={format}={}", source.id, source.url)
}

fn random_hex(bytes: usize) -> Result<String, CollectError> {
    let mut buf = vec![0u8; bytes];
    let mut file = fs::File::open("/dev/urandom").map_err(|_| CollectError::BadPath)?;
    std::io::Read::read_exact(&mut file, &mut buf).map_err(|_| CollectError::BadPath)?;
    Ok(buf.iter().map(|byte| format!("{byte:02x}")).collect())
}
