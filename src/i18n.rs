//! Языки интерфейса. Ключ — русский текст (он же вывод для ru), переводы — в i18n_table.rs.
//! Подстановки: `{}` по порядку и `{N}` по номеру аргумента, спецификаторы ширины и точности как в format!.

use std::collections::HashMap;
use std::fmt::{Display, Write};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Ru,
    En,
    De,
    It,
    Zh,
    Ar,
}

pub const ALL: [Lang; 6] = [Lang::Ru, Lang::En, Lang::De, Lang::It, Lang::Zh, Lang::Ar];

impl Lang {
    pub fn code(self) -> &'static str {
        match self {
            Lang::Ru => "ru",
            Lang::En => "en",
            Lang::De => "de",
            Lang::It => "it",
            Lang::Zh => "zh",
            Lang::Ar => "ar",
        }
    }
    /// Название языка на нём самом — так его узнают в списке выбора
    pub fn name(self) -> &'static str {
        match self {
            Lang::Ru => "Русский",
            Lang::En => "English",
            Lang::De => "Deutsch",
            Lang::It => "Italiano",
            Lang::Zh => "中文",
            Lang::Ar => "العربية",
        }
    }
    pub fn from_code(s: &str) -> Option<Lang> {
        let s = s.trim().to_ascii_lowercase();
        ALL.into_iter().find(|l| s == l.code() || s.starts_with(&format!("{}_", l.code())) || s.starts_with(&format!("{}-", l.code())))
    }
    fn idx(self) -> usize {
        self as usize
    }
}

// Язык общий для всех потоков (фоновые потоки TUI и апплета пишут на выбранном языке).
// Поток может задать свой язык (`set_thread`): так параллельные тесты, в том числе тесты
// бинарника и апплета, не мешают друг другу; в тестах библиотеки `set` действует только на поток.
static CUR: AtomicU8 = AtomicU8::new(0);
thread_local! {
    static THREAD: std::cell::Cell<u8> = const { std::cell::Cell::new(u8::MAX) };
}

pub fn set(l: Lang) {
    #[cfg(not(test))]
    CUR.store(l as u8, Ordering::Relaxed);
    #[cfg(test)]
    set_thread(l);
}

/// Язык только для текущего потока (тесты).
pub fn set_thread(l: Lang) {
    THREAD.with(|c| c.set(l as u8));
}

pub fn cur() -> Lang {
    let v = match THREAD.with(|c| c.get()) {
        u8::MAX => CUR.load(Ordering::Relaxed),
        v => v,
    };
    ALL[v as usize % ALL.len()]
}

/// Язык из окружения (LC_ALL → LC_MESSAGES → LANG); незнакомый — английский, пустой (службы без локали) — русский.
pub fn from_env() -> Lang {
    for k in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(v) = std::env::var(k) {
            if v.is_empty() {
                continue;
            }
            if v == "C" || v == "POSIX" || v.starts_with("C.") {
                return Lang::En;
            }
            return Lang::from_code(&v).unwrap_or(Lang::En);
        }
    }
    Lang::Ru
}

/// Значение настройки lang: код языка или auto.
pub fn resolve(setting: &str) -> Lang {
    Lang::from_code(setting).unwrap_or_else(from_env)
}

fn table() -> &'static HashMap<&'static str, &'static [&'static str; 5]> {
    static T: OnceLock<HashMap<&'static str, &'static [&'static str; 5]>> = OnceLock::new();
    T.get_or_init(|| crate::i18n_table::T.iter().map(|(k, v)| (*k, v)).collect())
}

/// Перевод строки на текущий язык; нет перевода — русский оригинал.
pub fn tr(s: &'static str) -> &'static str {
    tr_to(cur(), s)
}

pub fn tr_to(l: Lang, s: &'static str) -> &'static str {
    if l == Lang::Ru {
        return s;
    }
    match table().get(s) {
        Some(v) => {
            let t = v[l.idx() - 1];
            if t.is_empty() {
                // нет перевода на этот язык — лучше английский, чем русский
                let en = v[0];
                if en.is_empty() {
                    s
                } else {
                    en
                }
            } else {
                t
            }
        }
        None => s,
    }
}

/// Идентификаторы, которые хранятся в mirrors.json и участвуют в логике (источник и ошибка замера зеркала):
/// (идентификатор, ключ перевода). Не зависят от языка; до 0.2.5 хранились русскими — их тоже узнаём.
pub const DATA: &[(&str, &str)] = &[
    ("no DNS", "нет DNS"),
    ("timeout", "таймаут"),
    ("dropped", "обрыв"),
    ("TLS error", "ошибка TLS"),
    ("no connection", "нет связи"),
    ("not measured", "не замерено"),
    ("config", "конфиг"),
    ("pinned", "закреп"),
    ("search", "поиск"),
    ("list", "список"),
];

/// Показ сохранённого идентификатора из DATA (и «lag N h») на текущем языке; остальное — как есть.
pub fn tr_data(s: &str) -> String {
    if let Some((_, k)) = DATA.iter().find(|(id, k)| *id == s || *k == s) {
        return tr(k).to_string();
    }
    let lag = s.strip_prefix("lag ").and_then(|r| r.strip_suffix(" h")).or_else(|| s.strip_prefix("отстаёт ").and_then(|r| r.strip_suffix(" ч")));
    if let Some(n) = lag {
        return fill(tr("отстаёт {} ч"), &[&n]);
    }
    s.to_string()
}

/// Подстановка аргументов в переведённый шаблон.
pub fn fill(tpl: &str, args: &[&dyn Display]) -> String {
    let mut out = String::with_capacity(tpl.len() + 16);
    let mut next = 0usize;
    let mut it = tpl.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '{' if it.peek() == Some(&'{') => {
                it.next();
                out.push('{');
            }
            '}' if it.peek() == Some(&'}') => {
                it.next();
                out.push('}');
            }
            '{' => {
                let mut spec = String::new();
                for c in it.by_ref() {
                    if c == '}' {
                        break;
                    }
                    spec.push(c);
                }
                let (pos, fmt) = spec.split_once(':').unwrap_or((&spec, ""));
                let i = if pos.is_empty() {
                    next += 1;
                    next - 1
                } else {
                    pos.parse().unwrap_or(usize::MAX)
                };
                match args.get(i) {
                    Some(a) => push_fmt(&mut out, *a, fmt),
                    None => {
                        out.push('{');
                        out.push_str(&spec);
                        out.push('}');
                    }
                }
            }
            c => out.push(c),
        }
    }
    out
}

/// Подмножество спецификаторов format!: [[fill]align][width][.precision]
fn push_fmt(out: &mut String, a: &dyn Display, spec: &str) {
    if spec.is_empty() {
        let _ = write!(out, "{a}");
        return;
    }
    let mut chars: Vec<char> = spec.chars().collect();
    let mut align = '\0';
    let mut fill_ch = ' ';
    if chars.len() >= 2 && matches!(chars[1], '<' | '>' | '^') {
        fill_ch = chars[0];
        align = chars[1];
        chars.drain(0..2);
    } else if !chars.is_empty() && matches!(chars[0], '<' | '>' | '^') {
        align = chars[0];
        chars.remove(0);
    }
    let rest: String = chars.into_iter().collect();
    let (w, p) = rest.split_once('.').map(|(w, p)| (w, Some(p))).unwrap_or((rest.as_str(), None));
    let width: usize = w.parse().unwrap_or(0);
    let body = match p.and_then(|p| p.parse::<usize>().ok()) {
        Some(p) => format!("{a:.p$}"),
        None => format!("{a}"),
    };
    let len = body.chars().count();
    if len >= width {
        out.push_str(&body);
        return;
    }
    let pad = width - len;
    // по умолчанию, как у format! для строк, — влево
    let (l, r) = match align {
        '>' => (pad, 0),
        '^' => (pad / 2, pad - pad / 2),
        _ => (0, pad),
    };
    out.extend(std::iter::repeat(fill_ch).take(l));
    out.push_str(&body);
    out.extend(std::iter::repeat(fill_ch).take(r));
}

/// `t!("текст")` — &'static str; `t!("шаблон {}", a, b)` — String.
#[macro_export]
macro_rules! t {
    ($k:expr) => {
        $crate::i18n::tr($k)
    };
    ($k:expr, $($a:expr),+ $(,)?) => {
        $crate::i18n::fill($crate::i18n::tr($k), &[$(&$a as &dyn std::fmt::Display),+])
    };
}

// ---------- раскладки клавиатуры ----------

/// Символ с той же клавиши в латинской раскладке (US): для горячих клавиш и ответов y/n,
/// набранных в русской, арабской, немецкой (QWERTZ) раскладке или полноширинными символами китайского ввода.
pub fn latin_key(c: char) -> char {
    const RU: &str = "йцукенгшщзфывапролдячсмитьхъжэбюё";
    const RU_LAT: &str = "qwertyuiopasdfghjklzxcvbnm[];',.`";
    const AR: &str = "ضصثقفغعهخحشسيبلاتنمئءؤرىةوزظ";
    const AR_LAT: &str = "qwertyuiopasdfghjklzxcvnm,./";
    let lc: Vec<char> = c.to_lowercase().collect();
    let c1 = if lc.len() == 1 { lc[0] } else { c };
    if let Some(i) = RU.chars().position(|x| x == c1) {
        return RU_LAT.chars().nth(i).unwrap_or(c);
    }
    if let Some(i) = AR.chars().position(|x| x == c1) {
        return AR_LAT.chars().nth(i).unwrap_or(c);
    }
    // полноширинные ASCII (китайский/японский ввод): U+FF01..U+FF5E
    if ('\u{FF01}'..='\u{FF5E}').contains(&c) {
        return char::from_u32(c as u32 - 0xFEE0).unwrap_or(c).to_ascii_lowercase();
    }
    c
}

/// Ответ на вопрос да/нет на любом из языков и раскладок. Some(None) — пустой (по умолчанию), None — непонятный.
/// Однобуквенные ответы с неоднозначной клавишей (русская «н» — это Y, но и «нет») не угадываются.
pub fn yes_no(s: &str) -> Option<Option<bool>> {
    let a = s.trim().to_lowercase();
    if a.is_empty() {
        return Some(None);
    }
    const YES: &[&str] = &["y", "yes", "да", "д", "ja", "j", "si", "sì", "s", "是", "是的", "好", "对", "نعم", "ن"];
    const NO: &[&str] = &["n", "no", "нет", "nein", "否", "不", "不是", "لا", "ل"];
    if YES.contains(&a.as_str()) {
        return Some(Some(true));
    }
    if NO.contains(&a.as_str()) {
        return Some(Some(false));
    }
    // одна клавиша в другой раскладке: т → n, غ → y, ى → n, z (Y на QWERTZ) → y, ｙ → y
    let mut cs = a.chars();
    if let (Some(c), None) = (cs.next(), cs.next()) {
        if c == 'н' {
            return None;
        }
        return match latin_key(c) {
            'y' => Some(Some(true)),
            'n' => Some(Some(false)),
            // немецкая QWERTZ: на месте Y — Z
            'z' if cur() == Lang::De => Some(Some(true)),
            _ => None,
        };
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_positional_indexed_and_specs() {
        assert_eq!(fill("a {} b {}", &[&1, &"x"]), "a 1 b x");
        assert_eq!(fill("{1} before {0}", &[&"a", &"b"]), "b before a");
        assert_eq!(fill("[{:>5}] [{:<4}] [{:.2}] {{}}", &[&"ab", &"c", &1.23456]), "[   ab] [c   ] [1.23] {}");
        assert_eq!(fill("{:^5}|{:*>3}", &[&"a", &7]), "  a  |**7");
        assert_eq!(fill("нет {}", &[]), "нет {}");
    }

    #[test]
    fn layouts_map_to_latin() {
        assert_eq!(latin_key('й'), 'q');
        assert_eq!(latin_key('Н'), 'y');
        assert_eq!(latin_key('т'), 'n');
        assert_eq!(latin_key('غ'), 'y');
        assert_eq!(latin_key('ى'), 'n');
        assert_eq!(latin_key('ｑ'), 'q');
        assert_eq!(latin_key('q'), 'q');
    }

    #[test]
    fn yes_no_many_languages() {
        for y in ["y", "Yes", "да", "Д", "ja", "J", "sì", "si", "是", "نعم", "ｙ", "غ"] {
            assert_eq!(yes_no(y), Some(Some(true)), "{y}");
        }
        for n in ["n", "No", "нет", "т", "nein", "否", "لا", "ى", "ｎ"] {
            assert_eq!(yes_no(n), Some(Some(false)), "{n}");
        }
        assert_eq!(yes_no(""), Some(None));
        assert_eq!(yes_no("н"), None);
        assert_eq!(yes_no("maybe"), None);
    }

    #[test]
    fn lang_codes() {
        assert_eq!(Lang::from_code("de_DE.UTF-8"), Some(Lang::De));
        assert_eq!(Lang::from_code("zh-CN"), Some(Lang::Zh));
        assert_eq!(Lang::from_code("auto"), None);
        assert_eq!(resolve("it"), Lang::It);
    }

    /// Строковый литерал Rust (без кавычек) → строка.
    fn unescape(lit: &str) -> String {
        let mut out = String::new();
        let mut it = lit.chars().peekable();
        while let Some(c) = it.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match it.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('0') => out.push('\0'),
                Some('x') => {
                    let h: String = it.by_ref().take(2).collect();
                    out.push(u8::from_str_radix(&h, 16).unwrap() as char);
                }
                Some('u') => {
                    let h: String = it.by_ref().skip(1).take_while(|c| *c != '}').collect();
                    out.push(char::from_u32(u32::from_str_radix(&h, 16).unwrap()).unwrap());
                }
                Some('\n') => {
                    while it.peek().map(|c| c.is_whitespace()).unwrap_or(false) {
                        it.next();
                    }
                }
                Some(c) => out.push(c),
                None => {}
            }
        }
        out
    }

    /// Ключи вида t!("…") из исходников (без тестовых модулей).
    fn source_keys() -> Vec<String> {
        let files = [
            include_str!("backend.rs"),
            include_str!("common.rs"),
            include_str!("extras.rs"),
            include_str!("helper.rs"),
            include_str!("main.rs"),
            include_str!("mirrors.rs"),
            include_str!("status.rs"),
            include_str!("summary.rs"),
            include_str!("tui.rs"),
            include_str!("tui/process.rs"),
            include_str!("tui/host.rs"),
            include_str!("vpn.rs"),
            // графический интерфейс COSMIC — отдельный крейт, но переводы общие
            include_str!("../cosmic/src/applet.rs"),
            include_str!("../cosmic/src/main.rs"),
            include_str!("../cosmic/src/model.rs"),
            include_str!("../cosmic/src/jobs.rs"),
            include_str!("../cosmic/src/tui_launch.rs"),
        ];
        let mut keys = vec![];
        for src in files {
            // тестовые модули не переводятся; отдельные #[cfg(test)]-функции посреди файла не обрывают просмотр
            let code = src.split("#[cfg(test)]\nmod ").next().unwrap_or(src);
            let mut rest = code;
            while let Some(i) = rest.find("t!(\"") {
                // format!( и подобные тоже кончаются на t!(
                let macro_t = i == 0 || !rest.as_bytes()[i - 1].is_ascii_alphanumeric() && rest.as_bytes()[i - 1] != b'_';
                rest = &rest[i + 4..];
                if !macro_t {
                    continue;
                }
                let mut end = 0;
                let b = rest.as_bytes();
                while b[end] != b'"' {
                    end += if b[end] == b'\\' { 2 } else { 1 };
                }
                keys.push(unescape(&rest[..end]));
                rest = &rest[end..];
            }
        }
        keys
    }

    #[test]
    fn every_source_key_is_translated() {
        let missing: Vec<String> = source_keys().into_iter().filter(|k| k.chars().any(|c| ('а'..='я').contains(&c.to_ascii_lowercase()) || ('А'..='Я').contains(&c)) && !table().contains_key(k.as_str())).collect();
        assert!(missing.is_empty(), "нет перевода для {} строк: {:#?}", missing.len(), missing);
        for (_, k) in DATA {
            assert!(table().contains_key(k), "нет перевода для данных: {k}");
        }
        assert!(table().contains_key("отстаёт {} ч"));
    }

    #[test]
    fn translations_keep_placeholders() {
        let count = |s: &str| s.matches('{').count() - 2 * s.matches("{{").count();
        for (k, v) in crate::i18n_table::T {
            for t in v.iter().filter(|t| !t.is_empty()) {
                assert_eq!(count(k), count(t), "{k} → {t}");
            }
        }
    }

    #[test]
    fn switching_language_translates() {
        set(Lang::De);
        assert_eq!(t!("Выход"), "Beenden");
        assert_eq!(t!("последний замер: {}", "vor 5 Min."), "letzte Messung: vor 5 Min.");
        assert_eq!(tr_data("lag 7 h"), "7 Std. zurück");
        assert_eq!(tr_data("отстаёт 7 ч"), "7 Std. zurück", "значения до 0.2.5");
        assert_eq!(tr_data("pinned"), "fixiert");
        set(Lang::Zh);
        assert_eq!(t!("Выход"), "退出");
        set(Lang::Ru);
        assert_eq!(t!("Выход"), "Выход");
        assert_eq!(tr_data("timeout"), "таймаут");
        assert_eq!(tr_data("lag 3 h"), "отстаёт 3 ч");
    }

    #[test]
    fn table_has_no_duplicate_keys() {
        let mut seen = std::collections::HashSet::new();
        for (k, _) in crate::i18n_table::T {
            assert!(seen.insert(*k), "повтор ключа: {k}");
        }
    }
}
