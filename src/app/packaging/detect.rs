//! Способ запуска определяется по пути, аргументам и началу файла. Имён приложений здесь нет.

use std::fs::File;
use std::io::Read;
use std::path::Path;

const LAUNCHERS: &[&str] = &[
    "systemd-run",
    "gtk-launch",
    "gio",
    "gapplication",
    "dbus-send",
    "busctl",
    "xdg-open",
    "kioclient",
    "kioclient5",
    "kioclient6",
    "dex",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Packaging {
    Native,
    AppImage,
    Flatpak { app_id: Option<String> },
    Snap,
    Launcher,
}

pub fn detect(program: &Path, args: &[String], head: Option<&[u8]>) -> Packaging {
    let name = program
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    if name == "flatpak" && args.first().is_some_and(|arg| arg == "run") {
        let app_id = args
            .iter()
            .skip(1)
            .find(|arg| !arg.starts_with('-'))
            .cloned();
        return Packaging::Flatpak { app_id };
    }
    if program.starts_with("/snap/bin/")
        || (name == "snap" && args.first().is_some_and(|arg| arg == "run"))
    {
        return Packaging::Snap;
    }
    if LAUNCHERS.contains(&name) {
        return Packaging::Launcher;
    }
    if head.is_some_and(is_appimage) {
        return Packaging::AppImage;
    }
    Packaging::Native
}

pub fn read_head(program: &Path) -> Option<[u8; 16]> {
    let mut file = File::open(program).ok()?;
    let mut head = [0u8; 16];
    file.read_exact(&mut head).ok()?;
    Some(head)
}

fn is_appimage(head: &[u8]) -> bool {
    head.len() >= 11
        && head.starts_with(b"\x7fELF")
        && (head[8..11] == [0x41, 0x49, 0x02] || head[8..11] == [0x41, 0x49, 0x01])
}
