//! Команды `cm identity`.

use super::axes::{self, Axes, AxisValue};
use super::engine::{self, EngineError};
use super::guard;
use super::launch::{self, LaunchError};
use super::model::{self, BrowserIdentityProfile, ModelError, Strategy};
use super::presets::{self, LanguageChoice, PresetError};
use super::store::{self, IdentityStore, StoreError};
use super::tzdata::SYSTEM_TZDIR;
use super::validate::{self, Violation};
use std::path::{Path, PathBuf};

pub const HELP: &str = "cm identity — личности браузера (стратегии local и crowd)\n\n  cm identity list\n  cm identity create ID --browser PATH|auto --strategy local|crowd [--country CC] [--zone ZONE] [--lang local[:N]|en|TAG] [-- EXTRA...]\n  cm identity show ID\n  cm identity check ID\n  cm identity launch ID [URL] [--lab]\n  cm identity confirm ID\n  cm identity remove ID [--purge]\n\nlocal согласует язык и часовой пояс со страной. crowd запускает только Gecko.\nЛабораторный флаг --lab принимается только при CM_IDENTITY_LAB=1.\n";

const AUTO_BROWSERS: &[&str] = &[
    "/usr/bin/google-chrome-stable",
    "/usr/bin/chromium",
    "/usr/bin/brave",
    "/usr/bin/librewolf",
    "/usr/bin/firefox",
];

pub fn dispatch(args: &[String]) -> i32 {
    match execute(args) {
        Ok(()) => 0,
        Err(error) => error.emit(),
    }
}

fn execute(args: &[String]) -> Result<(), CliError> {
    if args.is_empty() || matches!(args[0].as_str(), "help" | "--help" | "-h") {
        return Err(CliError::Help);
    }
    match args[0].as_str() {
        "list" => {
            if args.len() != 1 {
                return Err(CliError::Usage);
            }
            list()
        }
        "create" => create(&args[1..]),
        "show" => show(&args[1..]),
        "check" => check(&args[1..]),
        "launch" => launch_cmd(&args[1..]),
        "confirm" => confirm_cmd(&args[1..]),
        "remove" => remove_cmd(&args[1..]),
        _ => Err(CliError::Unknown),
    }
}

fn list() -> Result<(), CliError> {
    let store = open_store()?;
    for id in store.list()? {
        let profile = store.load(&id)?;
        let country = profile
            .environment
            .as_ref()
            .map(|item| item.country.as_str())
            .unwrap_or("-");
        println!(
            "{}  {}  {} {}  {}",
            profile.id,
            profile.strategy.as_str(),
            profile.engine.brand.as_str(),
            profile.engine.major,
            country
        );
    }
    Ok(())
}

fn create(args: &[String]) -> Result<(), CliError> {
    let request = parse_create(args)?;
    model::validate_id(&request.id)?;
    if request.strategy == Strategy::Crowd
        && (request.country.is_some() || request.zone.is_some() || request.language.is_some())
    {
        return Err(CliError::CrowdOptions);
    }
    if request.strategy == Strategy::Local && request.country.is_none() {
        return Err(CliError::NeedCountry);
    }
    let browser = resolve_browser(&request.browser)?;
    let engine = engine::detect(&browser)?;
    let environment = if request.strategy == Strategy::Local {
        let language = match request.language.as_deref() {
            None => LanguageChoice::Local(0),
            Some(value) => parse_language(value)?,
        };
        Some(presets::select(
            request.country.as_deref().unwrap_or(""),
            request.zone.as_deref(),
            &language,
        )?)
    } else {
        None
    };
    let profile = BrowserIdentityProfile {
        schema_version: 1,
        id: request.id,
        strategy: request.strategy,
        browser,
        engine: engine.clone(),
        environment,
        extra_args: request.extra_args,
        created_at: crate::common::now(),
        generation: 0,
        history: Vec::new(),
        bypass_suspected: false,
    };
    let report = validate::validate(&profile, &engine, Path::new(SYSTEM_TZDIR), false);
    if !report.errors.is_empty() {
        return Err(CliError::Violations(report.errors));
    }
    open_store()?.create(&profile)?;
    print_environment(&profile);
    for warning in &report.warnings {
        print_coded(warning, warning.code());
    }
    Ok(())
}

fn show(args: &[String]) -> Result<(), CliError> {
    let profile = load_one(args)?;
    let report = validate::validate(&profile, &profile.engine, Path::new(SYSTEM_TZDIR), false);
    print_profile(&profile);
    print_axes(&axes::axes(&profile, &report));
    for item in report.errors.iter().chain(&report.warnings) {
        print_coded(item, item.code());
    }
    Ok(())
}

fn check(args: &[String]) -> Result<(), CliError> {
    let profile = load_one(args)?;
    let report = validate::validate(&profile, &profile.engine, Path::new(SYSTEM_TZDIR), false);
    print_axes(&axes::axes(&profile, &report));
    for warning in &report.warnings {
        print_coded(warning, warning.code());
    }
    if report.errors.is_empty() {
        Ok(())
    } else {
        Err(CliError::Violations(report.errors))
    }
}

fn launch_cmd(args: &[String]) -> Result<(), CliError> {
    let (id, url, lab) = parse_launch(args)?;
    if lab && std::env::var("CM_IDENTITY_LAB").ok().as_deref() != Some("1") {
        return Err(CliError::LabDisabled);
    }
    let code = launch::launch(&open_store()?, &id, url.as_deref(), lab)?;
    println!("{}", t!("код выхода браузера: {0}", code));
    Ok(())
}

fn confirm_cmd(args: &[String]) -> Result<(), CliError> {
    let id = one_id(args)?;
    let store = open_store()?;
    let mut profile = store.load(&id)?;
    let suspected = profile.bypass_suspected;
    guard::confirm(&store, &mut profile, crate::common::now())?;
    if suspected {
        println!("{}", t!("обход профиля подтверждён"));
    } else {
        println!("{}", t!("подтверждение не требуется"));
    }
    Ok(())
}

fn remove_cmd(args: &[String]) -> Result<(), CliError> {
    let (id, purge) = parse_remove(args)?;
    open_store()?.remove(&id, purge)?;
    println!("{}", t!("личность удалена"));
    Ok(())
}

struct CreateRequest {
    id: String,
    browser: String,
    strategy: Strategy,
    country: Option<String>,
    zone: Option<String>,
    language: Option<String>,
    extra_args: Vec<String>,
}

fn parse_create(args: &[String]) -> Result<CreateRequest, CliError> {
    if args.is_empty() || args[0].starts_with('-') {
        return Err(CliError::Usage);
    }
    let mut browser = None;
    let mut strategy = None;
    let mut country = None;
    let mut zone = None;
    let mut language = None;
    let mut extra_args = Vec::new();
    let mut index = 1;
    while index < args.len() {
        if args[index] == "--" {
            extra_args.extend(args[index + 1..].iter().cloned());
            break;
        }
        let value = args
            .get(index + 1)
            .map(String::as_str)
            .filter(|item| !item.starts_with("--"))
            .ok_or(CliError::Usage)?;
        match args[index].as_str() {
            "--browser" => set_once(&mut browser, value)?,
            "--strategy" => set_once(&mut strategy, value)?,
            "--country" => set_once(&mut country, value)?,
            "--zone" => set_once(&mut zone, value)?,
            "--lang" => set_once(&mut language, value)?,
            _ => return Err(CliError::Usage),
        }
        index += 2;
    }
    Ok(CreateRequest {
        id: args[0].clone(),
        browser: browser.ok_or(CliError::Usage)?,
        strategy: parse_strategy(&strategy.ok_or(CliError::Usage)?)?,
        country,
        zone,
        language,
        extra_args,
    })
}

fn parse_launch(args: &[String]) -> Result<(String, Option<String>, bool), CliError> {
    let mut id = None;
    let mut url = None;
    let mut lab = false;
    for arg in args {
        if arg == "--lab" {
            if lab {
                return Err(CliError::Usage);
            }
            lab = true;
            continue;
        }
        if arg.starts_with('-') {
            return Err(CliError::Usage);
        }
        if id.is_none() {
            id = Some(arg.clone());
        } else if url.is_none() {
            url = Some(arg.clone());
        } else {
            return Err(CliError::Usage);
        }
    }
    Ok((id.ok_or(CliError::Usage)?, url, lab))
}

fn parse_remove(args: &[String]) -> Result<(String, bool), CliError> {
    let mut id = None;
    let mut purge = false;
    for arg in args {
        if arg == "--purge" {
            if purge {
                return Err(CliError::Usage);
            }
            purge = true;
            continue;
        }
        if arg.starts_with('-') || id.is_some() {
            return Err(CliError::Usage);
        }
        id = Some(arg.clone());
    }
    Ok((id.ok_or(CliError::Usage)?, purge))
}

fn one_id(args: &[String]) -> Result<String, CliError> {
    if args.len() != 1 || args[0].starts_with('-') {
        Err(CliError::Usage)
    } else {
        Ok(args[0].clone())
    }
}

fn load_one(args: &[String]) -> Result<BrowserIdentityProfile, CliError> {
    let id = one_id(args)?;
    Ok(open_store()?.load(&id)?)
}

fn set_once(slot: &mut Option<String>, value: &str) -> Result<(), CliError> {
    if slot.is_some() {
        return Err(CliError::Usage);
    }
    *slot = Some(value.to_string());
    Ok(())
}

fn parse_strategy(value: &str) -> Result<Strategy, CliError> {
    match value {
        "local" => Ok(Strategy::Local),
        "crowd" => Ok(Strategy::Crowd),
        _ => Err(CliError::Usage),
    }
}

fn parse_language(value: &str) -> Result<LanguageChoice, CliError> {
    if value == "en" {
        return Ok(LanguageChoice::English);
    }
    if value == "local" {
        return Ok(LanguageChoice::Local(0));
    }
    if let Some(index) = value.strip_prefix("local:") {
        return Ok(LanguageChoice::Local(
            index.parse().map_err(|_| CliError::Usage)?,
        ));
    }
    Ok(LanguageChoice::Custom(value.to_string()))
}

fn resolve_browser(value: &str) -> Result<PathBuf, CliError> {
    if value != "auto" {
        return Ok(PathBuf::from(value));
    }
    AUTO_BROWSERS
        .iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
        .ok_or(CliError::NoBrowser)
}

fn open_store() -> Result<IdentityStore, CliError> {
    Ok(IdentityStore::open(store::root())?)
}

fn print_environment(profile: &BrowserIdentityProfile) {
    println!("{}", t!("идентификатор: {0}", profile.id));
    println!("{}", t!("стратегия: {0}", profile.strategy.as_str()));
    if let Some(environment) = &profile.environment {
        println!("{}", t!("страна: {0}", environment.country));
        println!("{}", t!("часовой пояс: {0}", environment.timezone));
        println!("{}", t!("языки: {0}", environment.accept_language()));
        println!("{}", t!("локаль: {0}", environment.posix_locale));
    }
}

fn print_profile(profile: &BrowserIdentityProfile) {
    print_environment(profile);
    println!("{}", t!("браузер: {0}", profile.browser.display()));
    println!(
        "{}",
        t!(
            "движок: {0} {1}",
            profile.engine.brand.as_str(),
            profile.engine.major
        )
    );
    println!("{}", t!("поколение: {0}", profile.generation));
    println!(
        "{}",
        t!("дополнительные аргументы: {0}", profile.extra_args.len())
    );
    let bypass = if profile.bypass_suspected {
        t!("да")
    } else {
        t!("нет")
    };
    println!("{}", t!("обход профиля: {0}", bypass));
}

fn print_axes(axes: &Axes) {
    for (name, axis) in [
        ("NET", axes.net),
        ("REGION", axes.region),
        ("STATE", axes.state),
        ("APP", axes.app),
    ] {
        println!(
            "{name}  {}  {}",
            axis_value(axis.value),
            crate::i18n::tr(axis.reason)
        );
    }
}

fn axis_value(value: AxisValue) -> &'static str {
    match value {
        AxisValue::Verified => "Verified",
        AxisValue::Partial => "Partial",
        AxisValue::Unknown => "Unknown",
        AxisValue::Blocked => "Blocked",
    }
}

fn print_coded(error: &impl std::fmt::Display, code: &str) {
    eprintln!("cm identity: {error} [{code}]");
}

enum CliError {
    Help,
    Usage,
    Unknown,
    NeedCountry,
    CrowdOptions,
    NoBrowser,
    LabDisabled,
    Preset(PresetError),
    Model(ModelError),
    Engine(EngineError),
    Store(StoreError),
    Violations(Vec<Violation>),
    Launch(LaunchError),
}

impl CliError {
    fn emit(self) -> i32 {
        match self {
            Self::Help => {
                print!("{}", t!(HELP));
                2
            }
            Self::Usage => line(t!("неверные аргументы cm identity"), "Usage", 2),
            Self::Unknown => line(t!("неизвестная команда identity"), "Usage", 2),
            Self::NeedCountry => line(t!("для стратегии local нужна страна"), "Usage", 2),
            Self::CrowdOptions => line(t!("стратегия crowd не принимает страну"), "Usage", 2),
            Self::NoBrowser => line(t!("браузер не найден"), "Usage", 2),
            Self::LabDisabled => line(t!("лабораторный запуск выключен"), "Usage", 2),
            Self::Preset(error) => line(error.to_string(), error.code(), 2),
            Self::Model(ModelError::BadId) => line(ModelError::BadId.to_string(), "BadId", 2),
            Self::Model(error) => line(error.to_string(), error.code(), 2),
            Self::Engine(error) => line(error.to_string(), error.code(), engine_exit(error)),
            Self::Store(error) => line(error.to_string(), error.code(), store_exit(error)),
            Self::Violations(items) => {
                for item in &items {
                    eprintln!("cm identity: {item} [{}]", item.code());
                }
                2
            }
            Self::Launch(error) => emit_launch(error),
        }
    }
}

fn emit_launch(error: LaunchError) -> i32 {
    match error {
        LaunchError::Invalid(items) => {
            for item in &items {
                eprintln!("cm identity: {item} [{}]", item.code());
            }
            2
        }
        LaunchError::Guard(error) => line(error.to_string(), error.code(), 3),
        LaunchError::AlreadyRunning => {
            line(LaunchError::AlreadyRunning.to_string(), "AlreadyRunning", 3)
        }
        LaunchError::RunAsRoot => line(LaunchError::RunAsRoot.to_string(), "RunAsRoot", 4),
        LaunchError::Spawn => line(LaunchError::Spawn.to_string(), "Spawn", 4),
        LaunchError::Engine(error) => line(
            error.to_string(),
            error.code(),
            if error == EngineError::NotAbsolute {
                2
            } else {
                4
            },
        ),
        LaunchError::Store(error) => line(error.to_string(), error.code(), store_exit(error)),
    }
}

fn line(phrase: impl AsRef<str>, code: &str, exit_code: i32) -> i32 {
    eprintln!("cm identity: {} [{code}]", phrase.as_ref());
    exit_code
}

fn engine_exit(error: EngineError) -> i32 {
    match error {
        EngineError::NotAbsolute | EngineError::UnknownEngine => 2,
        EngineError::Timeout | EngineError::VersionFailed => 4,
    }
}

fn store_exit(error: StoreError) -> i32 {
    if error == StoreError::BadId { 2 } else { 5 }
}

impl From<StoreError> for CliError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

impl From<PresetError> for CliError {
    fn from(error: PresetError) -> Self {
        Self::Preset(error)
    }
}

impl From<ModelError> for CliError {
    fn from(error: ModelError) -> Self {
        Self::Model(error)
    }
}

impl From<EngineError> for CliError {
    fn from(error: EngineError) -> Self {
        Self::Engine(error)
    }
}

impl From<LaunchError> for CliError {
    fn from(error: LaunchError) -> Self {
        Self::Launch(error)
    }
}
