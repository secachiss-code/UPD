//! Progress reported by package/build tools; an unknown duration stays unknown.

#[derive(Default)]
pub(super) struct Progress {
    pub aur: bool,
    pub phase: &'static str,
    pub ratio: Option<f64>,
}

impl Progress {
    pub fn observe(&mut self, line: &str) {
        let text = line.trim();
        let low = text.to_lowercase();
        let has = |words: &[&str]| words.iter().any(|word| low.contains(word));
        let makepkg = text.starts_with("==>");
        let new_package = has(&[
            "making package:",
            "создание пакета:",
            "erstelle paket:",
            "creazione del pacchetto:",
            "正在创建软件包：",
        ]) || low.starts_with("==> сборка пакета ");
        let phase = if new_package
            || has(&["updating aur packages", "→ aur"])
            || (makepkg
                && has(&[
                    "retrieving sources",
                    "получение исходных",
                    "получение исходников",
                    "загрузка исходных",
                    "empfange quellen",
                    "download dei sorgenti",
                    "获取源代码",
                ]))
        {
            self.aur = true;
            Some("Загрузка исходников")
        } else if self.aur
            && makepkg
            && has(&[
                "validating source",
                "проверка исходных",
                "проверка исходников",
                "проверка файлов",
                "überprüfe",
                "validazione di",
                "正在验证",
            ])
        {
            Some("Проверка исходников")
        } else if self.aur
            && makepkg
            && has(&[
                "extracting sources",
                "распаковка исходных",
                "распаковка исходников",
                "entpacke quellen",
                "estrazione dei sorgenti",
                "正在释放源码",
            ])
        {
            Some("Распаковка исходников")
        } else if self.aur && makepkg && has(&["build()", "начало сборки"]) {
            Some("Сборка")
        } else if self.aur && makepkg && has(&["check()", "проверка сборки"]) {
            Some("Проверка сборки")
        } else if self.aur
            && makepkg
            && has(&[
                "package()",
                "fakeroot",
                "creating package",
                "finished making",
                "упаковка",
                "создание пакета",
                "сборка завершена",
                "сборка пакета завершена",
                "завершена сборка пакета",
                "erstelle paket \"",
                "beendete erstellung",
                "creazione del pacchetto \"",
                "compilazione terminata",
                "正在构建软件包",
                "完成创建",
            ])
        {
            Some("Упаковка")
        } else if self.aur
            && has(&[
                "installing package",
                "установка пакета",
                " upgrading ",
                " installing ",
                ") обновление ",
                ") установка ",
                "installiere paket",
                "installazione del pacchetto",
                "正在安装软件包",
            ])
        {
            Some("Установка")
        } else {
            None
        };
        if let Some(phase) = phase {
            if phase != self.phase || new_package {
                self.ratio = None;
            }
            self.phase = phase;
        }
        // Ninja reports build units; pacman reports package counters. These are
        // local to the current phase, not a promise about the entire update.
        let counter = text
            .strip_prefix('[')
            .or_else(|| text.strip_prefix('('))
            .and_then(|rest| rest.split_once([']', ')']))
            .and_then(|(count, rest)| {
                if rest.starts_with(' ') && crate::tui::process::is_update_stage(text) {
                    return None;
                }
                let (done, total) = count.split_once('/')?;
                let done: u32 = done.trim().parse().ok()?;
                let total: u32 = total.trim().parse().ok()?;
                (total > 0 && done <= total).then_some(done as f64 / total as f64)
            });
        let percent = text.find('%').and_then(|end| {
            let start = text[..end]
                .rfind(|c: char| !c.is_ascii_digit() && c != '.')
                .map_or(0, |n| n + 1);
            if text[..start].ends_with('-') {
                return None;
            }
            let percent: f64 = text[start..end].parse().ok()?;
            (percent.is_finite() && (0.0..=100.0).contains(&percent)).then_some(percent / 100.0)
        });
        if let Some(ratio) = counter.or(percent) {
            self.ratio = Some(ratio);
        }
    }

    pub fn update_stage(&mut self) {
        self.aur = false;
        self.phase = "";
        self.ratio = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installed_makepkg_translations_report_the_same_phases() {
        for lines in [
            [
                "==> Сборка пакета fixture 1",
                "==> Получение исходных файлов...",
                "==> Проверка файлов source с использованием sha256sums...",
                "==> Распаковка исходных файлов...",
                "==> Запускается build()...",
                "==> Завершена сборка пакета fixture 1",
                "==> Установка пакета 'fixture' с помощью 'pacman'...",
            ],
            [
                "==> Erstelle Paket: fixture 1",
                "==> Empfange Quellen...",
                "==> Überprüfe source Dateien mit sha256sums...",
                "==> Entpacke Quellen...",
                "==> Beginne build()...",
                "==> Beendete Erstellung: fixture 1",
                "==> Installiere Paket fixture mit pacman...",
            ],
            [
                "==> Creazione del pacchetto: fixture 1",
                "==> Download dei sorgenti in corso...",
                "==> Validazione di source file con sha256sums...",
                "==> Estrazione dei sorgenti in corso...",
                "==> Avvio di build() in corso...",
                "==> Compilazione terminata: fixture 1",
                "==> Installazione del pacchetto fixture con pacman in corso...",
            ],
            [
                "==> 正在创建软件包：fixture 1",
                "==> 获取源代码...",
                "==> 正在验证 source 文件，使用sha256sums...",
                "==> 正在释放源码...",
                "==> 正在开始 build()...",
                "==> 完成创建：fixture 1",
                "==> 正在安装软件包 fixture，使用 pacman...",
            ],
        ] {
            let mut progress = Progress::default();
            for (line, phase) in lines.into_iter().zip([
                "Загрузка исходников",
                "Загрузка исходников",
                "Проверка исходников",
                "Распаковка исходников",
                "Сборка",
                "Упаковка",
                "Установка",
            ]) {
                progress.observe(line);
                assert!(progress.aur);
                assert_eq!(progress.phase, phase, "{line}");
            }
        }
    }
    #[test]
    fn aur_download_build_package_and_install_are_distinct() {
        let mut p = Progress::default();
        for (line, phase) in [
            ("==> Making package: example 1.2", "Загрузка исходников"),
            ("==> Retrieving sources...", "Загрузка исходников"),
            (
                "==> Validating source files with sha256sums...",
                "Проверка исходников",
            ),
            ("==> Extracting sources...", "Распаковка исходников"),
            ("==> Starting build()...", "Сборка"),
            ("==> Starting check()...", "Проверка сборки"),
            ("==> Starting package()...", "Упаковка"),
            ("(1/2) upgrading example", "Установка"),
        ] {
            p.observe(line);
            assert!(p.aur);
            assert_eq!(p.phase, phase);
        }
        assert_eq!(p.ratio, Some(0.5));
        p.observe("==> Making package: next 3.4");
        assert_eq!(p.phase, "Загрузка исходников");
        assert_eq!(p.ratio, None);
    }
    #[test]
    fn build_progress_and_unknown_durations_do_not_fake_completion() {
        let mut p = Progress::default();
        p.observe("==> Making package: example 1.2");
        p.observe("==> Starting build()...");
        assert_eq!(p.ratio, None);
        p.observe("[42/100] Building CXX object");
        assert_eq!(p.ratio, Some(0.42));
        p.observe("==> Starting package()...");
        assert_eq!(p.ratio, None);
        p.observe("Progress: [ 75%]");
        assert_eq!(p.ratio, Some(0.75));
        p.update_stage();
        p.observe("[5/6] Установка");
        assert_eq!(p.ratio, None);
        p.observe("invalid 150%");
        assert_eq!(p.ratio, None);
        p.observe("invalid -5%");
        assert_eq!(p.ratio, None);
    }
}
