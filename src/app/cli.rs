//! `cm app`: проверка описания, ярлык и запуск через контроллер.

use std::path::Path;

use crate::controller::client::{self, ClientError};
use crate::controller::protocol::Op;
use crate::profiles::ApplicationDefinition;

use super::desktop::desktop_entry;
use super::spec::{self, SpecError};

pub fn dispatch(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("check") => check_file(args.get(1).map(String::as_str)),
        Some("desktop") => desktop(args),
        Some("run") => run(args),
        _ => {
            print!(
                "{}",
                t!(
                    "cm app check ФАЙЛ\ncm app desktop ID ИМЯ [--icon ЗНАЧОК]\ncm app run ЭКЗЕМПЛЯР -- ПРОГРАММА [АРГУМЕНТЫ]\n"
                )
            );
            2
        }
    }
}

fn check_file(path: Option<&str>) -> i32 {
    let Some(path) = path else {
        return 2;
    };
    let Ok(bytes) = std::fs::read(path) else {
        return 2;
    };
    let Ok(definition) = serde_json::from_slice::<ApplicationDefinition>(&bytes) else {
        return 2;
    };
    match spec::check(&definition) {
        Ok(spec) => {
            let names: Vec<&str> = spec.env.keys().map(String::as_str).collect();
            println!(
                "{}",
                t!(
                    "программа {0}, аргументов {1}, переменные {2}",
                    spec.program.display(),
                    spec.args.len(),
                    names.join(" ")
                )
            );
            0
        }
        Err(error) => {
            eprintln!("[{}]", error_token(&error));
            2
        }
    }
}

fn desktop(args: &[String]) -> i32 {
    let (Some(id), Some(name)) = (args.get(1), args.get(2)) else {
        return 2;
    };
    let mut icon = None;
    let mut index = 3;
    while index < args.len() {
        if args[index] == "--icon" {
            icon = args.get(index + 1).map(String::as_str);
            index += 2;
        } else {
            return 2;
        }
    }
    match desktop_entry(id, name, icon) {
        Ok(text) => {
            print!("{text}");
            0
        }
        Err(_) => 2,
    }
}

fn run(args: &[String]) -> i32 {
    let Some(instance) = args.get(1).cloned() else {
        return 2;
    };
    let Some(split) = args.iter().position(|arg| arg == "--") else {
        return 2;
    };
    let Some(program) = args.get(split + 1).cloned() else {
        return 2;
    };
    if !program.starts_with('/') {
        return 2;
    }
    let rest = args.get(split + 2..).unwrap_or(&[]).to_vec();
    if spec::program_rejected(&program, &rest) {
        return 2;
    }
    let socket = std::env::var("CM_CONTROLLER_SOCKET")
        .unwrap_or_else(|_| "/run/cm/controller.sock".to_owned());
    let status = match client::call(
        Path::new(&socket),
        Op::WorkerStatus {
            instance: instance.clone(),
        },
    ) {
        Ok(reply) => reply,
        Err(_) => return 4,
    };
    let Some(crate::controller::protocol::ReplyData::Status {
        generation: Some(generation),
        ..
    }) = status.data
    else {
        eprintln!("[{}]", status.code);
        return 3;
    };
    match client::call(
        Path::new(&socket),
        Op::AppLaunch {
            instance,
            generation,
            program,
            args: rest,
        },
    ) {
        Ok(reply) if reply.ok => {
            if let Some(crate::controller::protocol::ReplyData::Launched { pid }) = reply.data {
                println!("{pid}");
            }
            0
        }
        Ok(reply) => {
            eprintln!("[{}]", reply.code);
            3
        }
        Err(ClientError::Unavailable) => 4,
        Err(_) => 4,
    }
}

fn error_token(error: &SpecError) -> &'static str {
    match error {
        SpecError::NotAbsolute => "NotAbsolute",
        SpecError::HasNul => "HasNul",
        SpecError::DotDot => "DotDot",
        SpecError::TooManyArgs => "TooManyArgs",
        SpecError::ArgTooLong => "ArgTooLong",
        SpecError::BadEnvName => "BadEnvName",
        SpecError::ForbiddenEnv(_) => "ForbiddenEnv",
        SpecError::DuplicateEnv => "DuplicateEnv",
        SpecError::BadCwd => "BadCwd",
        SpecError::BadId => "BadId",
    }
}
