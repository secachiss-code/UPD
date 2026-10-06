//! Own server entered by hand (`I03.T04.u`): `SourceKind::ManualServer`, origin local, no
//! fetch settings and no User-Agent.
//!
//! Secrets never travel through argv: any secret-looking flag, inline `--flag=value` form or
//! share URI on the command line is refused with [`ManualError::SecretInArgv`]. The secret is
//! read from stdin or from a file whose *path* is given. An edit publishes a new generation of
//! the same Source; per the I02 model, Node IDs are issued once and belong to a generation,
//! so the edited node gets a new Node ID while the Source ID stays.

use super::artifact::{ArtifactError, GlobalDefaults, SourceImportInput};
use super::parser::{ParserError, parse_share_uri};
use crate::profiles::NodeProtocol;
use serde_json::{Map, Value, json};
use std::fmt;
use std::io::Read;
use std::path::PathBuf;

/// Upper bound for one secret line (UUID, password, key) or a share URI read from stdin.
const MAX_SECRET_INPUT: u64 = 8192;

/// Flags that would carry a secret value. Their presence alone is refused.
const SECRET_FLAGS: &[&str] = &[
    "--uuid",
    "--password",
    "--secret",
    "--private-key",
    "--pre-shared-key",
    "--psk",
    "--token",
    "--obfs-password",
    "--uri",
];

#[derive(Debug)]
pub enum ManualError {
    /// A secret (or a URI that embeds one) was given on the command line.
    SecretInArgv,
    InvalidArguments,
    /// The secret input was missing, too long, not UTF-8 or contained control characters.
    InvalidSecret,
    SecretUnreadable,
    /// The secret file is accessible to group/others or not owned by the caller.
    InsecureCredentialFile,
    Parser(ParserError),
    Artifact(ArtifactError),
}

impl fmt::Display for ManualError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SecretInArgv => {
                f.write_str("secrets must not be passed as command-line arguments")
            }
            Self::InvalidArguments => f.write_str("invalid manual server arguments"),
            Self::InvalidSecret => f.write_str("secret input is empty, too long or malformed"),
            Self::SecretUnreadable => f.write_str("secret input could not be read"),
            Self::InsecureCredentialFile => f.write_str(
                "secret file must be owned by the current user and not accessible to others",
            ),
            Self::Parser(error) => fmt::Display::fmt(error, f),
            Self::Artifact(error) => fmt::Display::fmt(error, f),
        }
    }
}

impl std::error::Error for ManualError {}

impl From<ParserError> for ManualError {
    fn from(error: ParserError) -> Self {
        Self::Parser(error)
    }
}

impl From<ArtifactError> for ManualError {
    fn from(error: ArtifactError) -> Self {
        Self::Artifact(error)
    }
}

/// Where the secret comes from. Only a path is ever kept in argv.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SecretSource {
    Stdin,
    File(PathBuf),
}

/// What the secret input contains.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecretKind {
    /// The protocol's credential (UUID or password).
    Credential,
    /// A whole share URI; protocol/server/port then come from it.
    ShareUri,
}

/// Validated, secret-free command-line arguments of `cm source add-server`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManualServerArgs {
    pub name: Option<String>,
    pub protocol: Option<NodeProtocol>,
    pub server: Option<String>,
    pub port: Option<u16>,
    pub cipher: Option<String>,
    pub sni: Option<String>,
    pub tls: bool,
    pub secret_source: SecretSource,
    pub secret_kind: SecretKind,
}

/// Parse `add-server` flags. Refuses any secret on the command line before anything else.
pub fn parse_add_server_args(argv: &[String]) -> Result<ManualServerArgs, ManualError> {
    for arg in argv {
        let flag = arg.split_once('=').map_or(arg.as_str(), |(flag, _)| flag);
        if SECRET_FLAGS.contains(&flag) || arg.contains("://") {
            return Err(ManualError::SecretInArgv);
        }
    }
    let mut args = ManualServerArgs {
        name: None,
        protocol: None,
        server: None,
        port: None,
        cipher: None,
        sni: None,
        tls: false,
        secret_source: SecretSource::Stdin,
        secret_kind: SecretKind::Credential,
    };
    let mut secret_source = None;
    let mut iter = argv.iter();
    while let Some(arg) = iter.next() {
        let mut value = || iter.next().cloned().ok_or(ManualError::InvalidArguments);
        match arg.as_str() {
            "--name" => args.name = Some(value()?),
            "--protocol" => args.protocol = Some(protocol(&value()?)?),
            "--server" => args.server = Some(value()?),
            "--port" => {
                args.port = Some(
                    value()?
                        .parse()
                        .map_err(|_| ManualError::InvalidArguments)?,
                )
            }
            "--cipher" => args.cipher = Some(value()?),
            "--sni" => args.sni = Some(value()?),
            "--tls" => args.tls = true,
            "--secret-stdin" => set_once(&mut secret_source, SecretSource::Stdin)?,
            "--secret-file" => set_once(&mut secret_source, SecretSource::File(value()?.into()))?,
            "--uri-stdin" => {
                set_once(&mut secret_source, SecretSource::Stdin)?;
                args.secret_kind = SecretKind::ShareUri;
            }
            _ => return Err(ManualError::InvalidArguments),
        }
    }
    args.secret_source = secret_source.ok_or(ManualError::InvalidArguments)?;
    if args.secret_kind == SecretKind::Credential
        && (args.protocol.is_none() || args.server.is_none() || args.port.is_none())
    {
        return Err(ManualError::InvalidArguments);
    }
    Ok(args)
}

fn set_once(slot: &mut Option<SecretSource>, value: SecretSource) -> Result<(), ManualError> {
    if slot.replace(value).is_some() {
        return Err(ManualError::InvalidArguments);
    }
    Ok(())
}

fn protocol(name: &str) -> Result<NodeProtocol, ManualError> {
    Ok(match name {
        "vless" => NodeProtocol::Vless,
        "vmess" => NodeProtocol::Vmess,
        "trojan" => NodeProtocol::Trojan,
        "hysteria2" | "hy2" => NodeProtocol::Hysteria2,
        "ss" | "shadowsocks" => NodeProtocol::Shadowsocks,
        _ => return Err(ManualError::InvalidArguments),
    })
}

/// Read one secret line from `reader` (stdin, or an opened file). Bounded, UTF-8, no controls.
pub fn read_secret(reader: impl Read) -> Result<String, ManualError> {
    let mut text = String::new();
    reader
        .take(MAX_SECRET_INPUT + 1)
        .read_to_string(&mut text)
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::InvalidData => ManualError::InvalidSecret,
            _ => ManualError::SecretUnreadable,
        })?;
    if text.len() as u64 > MAX_SECRET_INPUT {
        return Err(ManualError::InvalidSecret);
    }
    let line = text.strip_suffix('\n').unwrap_or(&text);
    let line = line.strip_suffix('\r').unwrap_or(line);
    if line.is_empty() || line.chars().any(char::is_control) {
        return Err(ManualError::InvalidSecret);
    }
    Ok(line.to_owned())
}

/// Open the secret file without following a final symlink and read it.
///
/// Like ssh with private keys, a file accessible to group/others or owned by another user is
/// refused (decision 2026-10-06, I03-DECISIONS §Ручной сервер).
pub fn read_secret_file(path: &std::path::Path) -> Result<String, ManualError> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| ManualError::SecretUnreadable)?;
    let metadata = file.metadata().map_err(|_| ManualError::SecretUnreadable)?;
    if !metadata.is_file() {
        return Err(ManualError::SecretUnreadable);
    }
    // SAFETY: geteuid has no preconditions and cannot fail.
    let euid = unsafe { libc::geteuid() };
    if metadata.mode() & 0o077 != 0 || metadata.uid() != euid {
        return Err(ManualError::InsecureCredentialFile);
    }
    read_secret(file)
}

/// Build the node object for these arguments and secret, without validating it yet.
pub fn manual_node(args: &ManualServerArgs, secret: &str) -> Result<Value, ManualError> {
    if args.secret_kind == SecretKind::ShareUri {
        let mut node = parse_share_uri(secret)?;
        if let (Some(name), Some(object)) = (&args.name, node.as_object_mut()) {
            object.insert("name".into(), Value::String(name.clone()));
        }
        return Ok(node);
    }
    let protocol = args.protocol.ok_or(ManualError::InvalidArguments)?;
    let server = args.server.clone().ok_or(ManualError::InvalidArguments)?;
    let port = args.port.ok_or(ManualError::InvalidArguments)?;
    let name = args
        .name
        .clone()
        .unwrap_or_else(|| format!("{server}:{port}"));
    let mut node = Map::new();
    node.insert("name".into(), Value::String(name));
    node.insert("server".into(), Value::String(server));
    node.insert("port".into(), json!(port));
    let (kind, secret_field) = match protocol {
        NodeProtocol::Vless => ("vless", "uuid"),
        NodeProtocol::Vmess => ("vmess", "uuid"),
        NodeProtocol::Trojan => ("trojan", "password"),
        NodeProtocol::Hysteria2 => ("hysteria2", "password"),
        NodeProtocol::Shadowsocks => ("ss", "password"),
        _ => return Err(ManualError::InvalidArguments),
    };
    node.insert("type".into(), json!(kind));
    node.insert(secret_field.into(), Value::String(secret.to_owned()));
    match protocol {
        NodeProtocol::Vmess => {
            node.insert(
                "cipher".into(),
                json!(args.cipher.as_deref().unwrap_or("auto")),
            );
            node.insert("alterId".into(), json!(0));
        }
        NodeProtocol::Shadowsocks => {
            node.insert(
                "cipher".into(),
                json!(
                    args.cipher
                        .as_deref()
                        .ok_or(ManualError::InvalidArguments)?
                ),
            );
        }
        _ if args.cipher.is_some() => return Err(ManualError::InvalidArguments),
        _ => {}
    }
    if matches!(protocol, NodeProtocol::Vless | NodeProtocol::Vmess) {
        node.insert("tls".into(), Value::Bool(args.tls));
    } else if args.tls {
        return Err(ManualError::InvalidArguments);
    }
    if let Some(sni) = &args.sni {
        let field = if matches!(protocol, NodeProtocol::Vless | NodeProtocol::Vmess) {
            "servername"
        } else {
            "sni"
        };
        node.insert(field.into(), Value::String(sni.clone()));
    }
    Ok(Value::Object(node))
}

/// Validate the node exactly like an imported one and wrap it as a local source input.
pub fn manual_source_input(
    core_version: &str,
    node: Value,
    accepted_at_unix_ms: i64,
) -> Result<SourceImportInput, ManualError> {
    let (_, definition) = super::parser::validate_single_node(core_version, node.clone())?;
    let body = serde_json::to_vec(&json!({ "proxies": [node] }))
        .map_err(|_| ManualError::InvalidSecret)?;
    Ok(SourceImportInput::local(
        body,
        accepted_at_unix_ms,
        vec![definition],
        GlobalDefaults::default(),
    )?)
}
