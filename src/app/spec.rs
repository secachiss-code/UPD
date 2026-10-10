//! Проверка описания приложения до запуска. Поиск по PATH не выполняется.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::profiles::ApplicationDefinition;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpecError {
    NotAbsolute,
    HasNul,
    DotDot,
    TooManyArgs,
    ArgTooLong,
    BadEnvName,
    ForbiddenEnv(String),
    DuplicateEnv,
    BadCwd,
    BadId,
}

impl std::fmt::Display for SpecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAbsolute => f.write_str(t!("путь программы не абсолютный")),
            Self::HasNul => f.write_str(t!("путь или аргумент содержит запрещённый символ")),
            Self::DotDot => f.write_str(t!("путь содержит ..")),
            Self::TooManyArgs => f.write_str(t!("слишком много аргументов")),
            Self::ArgTooLong => f.write_str(t!("аргумент слишком длинный")),
            Self::BadEnvName => f.write_str(t!("неверное имя переменной")),
            Self::ForbiddenEnv(name) => write!(f, "{}", t!("запрещённая переменная {0}", name)),
            Self::DuplicateEnv => f.write_str(t!("переменная повторяется")),
            Self::BadCwd => f.write_str(t!("неверный рабочий каталог")),
            Self::BadId => f.write_str(t!("неверный идентификатор")),
        }
    }
}

pub fn check(definition: &ApplicationDefinition) -> Result<LaunchSpec, SpecError> {
    let program = check_path(&definition.executable)?;
    if definition.argv.len() > 256 {
        return Err(SpecError::TooManyArgs);
    }
    for arg in &definition.argv {
        if arg.as_bytes().contains(&0) {
            return Err(SpecError::HasNul);
        }
        if arg.len() > 8192 {
            return Err(SpecError::ArgTooLong);
        }
    }
    let cwd = match &definition.cwd {
        Some(cwd) => Some(check_cwd(cwd)?),
        None => None,
    };
    let mut env = BTreeMap::new();
    for variable in &definition.environment {
        check_env_name(&variable.name)?;
        if variable.value.as_bytes().contains(&0) {
            return Err(SpecError::HasNul);
        }
        if env
            .insert(variable.name.clone(), variable.value.clone())
            .is_some()
        {
            return Err(SpecError::DuplicateEnv);
        }
    }
    Ok(LaunchSpec {
        program,
        args: definition.argv.clone(),
        cwd,
        env,
    })
}

pub fn program_rejected(program: &str, args: &[String]) -> bool {
    check_absolute(program).is_err()
        || args
            .iter()
            .any(|arg| arg.as_bytes().contains(&0) || arg.len() > 8192)
}

fn check_path(text: &str) -> Result<PathBuf, SpecError> {
    check_absolute(text)?;
    if text.split('/').any(|segment| segment == "..") {
        return Err(SpecError::DotDot);
    }
    Ok(PathBuf::from(text))
}

fn check_cwd(text: &str) -> Result<PathBuf, SpecError> {
    if text.as_bytes().contains(&0)
        || !text.starts_with('/')
        || text.split('/').any(|segment| segment == "..")
    {
        return Err(SpecError::BadCwd);
    }
    Ok(PathBuf::from(text))
}

fn check_absolute(text: &str) -> Result<(), SpecError> {
    if !text.starts_with('/') {
        return Err(SpecError::NotAbsolute);
    }
    if text.as_bytes().contains(&0) {
        return Err(SpecError::HasNul);
    }
    Ok(())
}

fn check_env_name(name: &str) -> Result<(), SpecError> {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return Err(SpecError::BadEnvName),
    }
    if !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
        return Err(SpecError::BadEnvName);
    }
    if forbidden(name) {
        return Err(SpecError::ForbiddenEnv(name.to_owned()));
    }
    Ok(())
}

fn forbidden(name: &str) -> bool {
    name.starts_with("LD_")
        || name.starts_with("LC_")
        || matches!(
            name,
            "TZ" | "LANG"
                | "LANGUAGE"
                | "PATH"
                | "HOME"
                | "USER"
                | "LOGNAME"
                | "SHELL"
                | "DBUS_SESSION_BUS_ADDRESS"
                | "XDG_RUNTIME_DIR"
        )
}
