# Grok: направление BI «Личность браузера» — весь код за один проход

Дата: 2026-10-08. Координатор: Claude. Граф и контракты: [BI-DAG.md](BI-DAG.md). Почему именно так: [ADR-BROWSER-IDENTITY.md](ADR-BROWSER-IDENTITY.md), [I15R-LAB.md](I15R-LAB.md).

## Задача

Реализовать модуль `src/identity/` и команду `cm identity` целиком: все вершины ниже, в порядке зависимостей.
Это личность браузера, которую CM запускает согласованно со страной:
- Chromium — через флаги `--lang`/`--accept-lang` и переменные `TZ`/`LANG`;
- Firefox/LibreWolf — через `user.js`.
UA, Client Hints и протокол отладки не трогаются: лаборатория показала, что так браузер неотличим от настоящего жителя страны.

Тесты пишет роль 2 по рёбрам. Поэтому точные значения из рёбер — часть спецификации, а не пожелание.

## Правила

1. **Перед сдачей — полный gate, EXIT=0:**
   `CARGO_TARGET_DIR=$HOME/.cache/cm-grok-target CM_TEST_MIHOMO=$HOME/.cache/cm-cores/mihomo/mihomo sh tests/check_audit.sh`.
2. **Переформатирование существующих файлов запрещено.**
   - Запрещены `cargo fmt` и `rustfmt` по любому существующему файлу. `rustfmt src/main.rs` форматирует ещё `tui.rs` и `tui/*`, а `rustfmt src/lib.rs` — весь крейт.
   - В diff существующих файлов (`src/lib.rs`, `src/main.rs`, `src/i18n_table.rs`, `tests/check_audit.sh`) — только добавленные смысловые строки.
   - `rustfmt --edition 2024` разрешён только для новых файлов `src/identity/*.rs`, и их нужно добавить в строку `rustfmt --check` в `tests/check_audit.sh`.
   - Проверка перед сдачей: `git diff --stat` по существующим файлам — единицы и десятки строк, не сотни.
3. Сборка только с `--offline --locked`. Новых зависимостей нет: `sha2`, `serde`, `serde_json`, `libc` уже в `Cargo.toml`.
4. Без установки, `systemctl`, изменения сети хоста и файлов вне `CM_IDENTITY_ROOT`. Настоящие браузеры в этой итерации не запускаются: это делает роль 2.
5. Новый `unsafe` — только с `// SAFETY:` (gate проверяет).
6. **Тесты пишет роль 2 следующей итерацией.** От тебя:
   - код, который проходит контракты рёбер ниже: точные значения, коды, порядок аргументов;
   - сохранность существующего gate;
   - без новых тестовых файлов, кроме однострочных проверок компиляции, если они нужны.
7. Неподдержанное — явная ошибка с кодом, а не молчаливый пропуск. Ошибки и `Debug` — без путей пользователя, argv и env.
8. Коммит — только по поручению пользователя. Сдача — отчёт: что сделано по каждой вершине, `git diff --stat`, EXIT gate.

## Вершины по порядку

### BI.R1 — Задать таблицу пресетов стран.

Файлы: `src/identity/presets.rs`. Зависит от: —.

`pub struct LanguageSet { pub tags: &'static [&'static str], pub posix: &'static str }`
`pub struct Preset { pub country: &'static str, pub zones: &'static [&'static str], pub language_sets: &'static [LanguageSet] }`
`pub static PRESETS: &[Preset]` — ровно эти страны и значения (первая зона и первый набор — по умолчанию):

| country | zones | language_sets (tags → posix) |
|---|---|---|
| AT | Europe/Vienna | de-AT,de,en-US,en → de_AT.UTF-8 |
| CA | America/Toronto, America/Vancouver, America/Edmonton, America/Winnipeg, America/Halifax | en-CA,en → en_CA.UTF-8; fr-CA,fr,en-CA,en → fr_CA.UTF-8 |
| CH | Europe/Zurich | de-CH,de,en-US,en → de_CH.UTF-8; fr-CH,fr,en-US,en → fr_CH.UTF-8 |
| DE | Europe/Berlin | de-DE,de,en-US,en → de_DE.UTF-8 |
| EE | Europe/Tallinn | et-EE,et,en-US,en → et_EE.UTF-8 |
| FI | Europe/Helsinki | fi-FI,fi,en-US,en → fi_FI.UTF-8 |
| FR | Europe/Paris | fr-FR,fr,en-US,en → fr_FR.UTF-8 |
| GB | Europe/London | en-GB,en → en_GB.UTF-8 |
| JP | Asia/Tokyo | ja-JP,ja,en-US,en → ja_JP.UTF-8 |
| LT | Europe/Vilnius | lt-LT,lt,en-US,en → lt_LT.UTF-8 |
| LV | Europe/Riga | lv-LV,lv,en-US,en → lv_LV.UTF-8 |
| NL | Europe/Amsterdam | nl-NL,nl,en-US,en → nl_NL.UTF-8 |
| PL | Europe/Warsaw | pl-PL,pl,en-US,en → pl_PL.UTF-8 |
| RU | Europe/Moscow, Asia/Yekaterinburg, Asia/Novosibirsk, Asia/Vladivostok | ru-RU,ru,en-US,en → ru_RU.UTF-8 |
| SE | Europe/Stockholm | sv-SE,sv,en-US,en → sv_SE.UTF-8 |
| SG | Asia/Singapore | en-SG,en → en_SG.UTF-8 |
| TR | Europe/Istanbul | tr-TR,tr,en-US,en → tr_TR.UTF-8 |
| US | America/New_York, America/Chicago, America/Denver, America/Phoenix, America/Los_Angeles, America/Anchorage, Pacific/Honolulu | en-US,en → en_US.UTF-8 |

Зоны взяты из `zone.tab` (не `zone1970.tab`: там NL сведён к Europe/Brussels, SE — к Europe/Berlin, а браузер жителя показывает Europe/Amsterdam и Europe/Stockholm). Комментарий над таблицей: источник зон — tzdata `zone.tab`; языки — официальные языки страны по CLDR; набор стран ограничен и расширяется только вместе с проверкой ребра BI.R1→BI.R2.

Контракты, которые проверит роль 2:
- **E01 → BI.R2:** Каждая строка таблицы пригодна для выбора: страна — двухбуквенный код; зоны есть в tzdata `zone.tab` именно для этой страны; у каждого набора языков первый тег совпадает с POSIX-локалью; теги — BCP 47.
  - все 18 стран: AT CA CH DE EE FI FR GB JP LT LV NL PL RU SE SG TR US — ровно этот набор
  - для каждой (country, zone): строка `country<TAB>…<TAB>zone` есть в zone.tab и файл `/usr/share/zoneinfo/<zone>` существует
  - NL.zones == ["Europe/Amsterdam"]; SE.zones == ["Europe/Stockholm"] (не Brussels/Berlin из zone1970.tab)
  - для каждого набора: tags[0] == `ll-RR`, posix == `ll_RR.UTF-8` с теми же ll и RR
  - DE.language_sets[0].tags == [de-DE, de, en-US, en]

### BI.R2 — Выбрать окружение личности по стране, зоне и языку.

Файлы: `src/identity/presets.rs`. Зависит от: BI.R1.

`pub enum LanguageChoice { Local(usize), English, Custom(String) }`
`#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)] pub struct EnvironmentProfile { pub country: String, pub timezone: String, pub languages: Vec<String>, pub posix_locale: String, pub matches_country: bool }`
`impl EnvironmentProfile { pub fn accept_language(&self) -> String /* языки через запятую без пробелов */; pub fn primary(&self) -> &str /* languages[0] */ }`
`pub fn select(country: &str, zone: Option<&str>, language: &LanguageChoice) -> Result<EnvironmentProfile, PresetError>`

- `country` нормализуется в верхний регистр; не из `PRESETS` → `PresetError::UnknownCountry`.
- `zone`: `None` → `zones[0]`; не из `zones` пресета → `ZoneNotInCountry`.
- `Local(i)`: `language_sets[i]`, иначе `NoSuchLanguageSet`.
- `English`: `["en-US", "en"]`, posix `en_US.UTF-8`.
- `Custom(tag)`: тег BCP 47 вида `ll` или `ll-RR` (2–3 строчные латинские буквы, затем необязательно `-` и 2 заглавные). Иначе `BadLanguageTag`. languages = `[tag, ll]` (если tag уже `ll` — `[tag]`), posix = `ll_RR.UTF-8` (для `ll` без региона — `ll_LL` в верхнем регистре, например `ru` → `ru_RU.UTF-8`).
- `matches_country = true`, только если итоговый список tags совпадает с одним из `language_sets` пресета.
- `PresetError`: `UnknownCountry, ZoneNotInCountry, NoSuchLanguageSet, BadLanguageTag`. `Display` — фиксированные фразы через `t!`, без входных значений.

Контракты, которые проверит роль 2:
- **E02 → BI.M1:** `select` детерминирован: одно и то же (страна, зона, язык) даёт один и тот же `EnvironmentProfile`; ошибки — без входных значений в тексте.
  - select("DE", None, Local(0)) → {country DE, timezone Europe/Berlin, languages [de-DE, de, en-US, en], posix_locale de_DE.UTF-8, matches_country true}; accept_language() == "de-DE,de,en-US,en"
  - select("de", None, Local(0)) == select("DE", None, Local(0))
  - select("Germany", None, Local(0)) → Err(UnknownCountry)
  - select("US", Some("America/Chicago"), English) → timezone America/Chicago, languages [en-US, en], posix en_US.UTF-8, matches_country true
  - select("US", Some("Europe/Berlin"), English) → Err(ZoneNotInCountry)
  - select("CH", None, Local(1)) → languages [fr-CH, fr, en-US, en], posix fr_CH.UTF-8
  - select("CH", None, Local(2)) → Err(NoSuchLanguageSet)
  - select("DE", None, English) → languages [en-US, en], matches_country false
  - select("JP", None, Custom("ru-RU")) → languages [ru-RU, ru], posix ru_RU.UTF-8, matches_country false
  - select("JP", None, Custom("ru")) → languages [ru], posix ru_RU.UTF-8
  - select("JP", None, Custom("ru_RU")) и Custom("RU-ru") и Custom("") → Err(BadLanguageTag)
  - текст каждой ошибки не содержит входной строки ("Germany", "Europe/Berlin", "ru_RU")

### BI.R3 — Проверить зону по системной tzdata.

Файлы: `src/identity/tzdata.rs`. Зависит от: —.

`pub fn verify_zone(tzdir: &Path, country: &str, zone: &str) -> Result<(), ZoneError>`; `pub const SYSTEM_TZDIR: &str = "/usr/share/zoneinfo";`
- Имя зоны: только `[A-Za-z0-9_+-]` сегменты через `/`, без `..`, без ведущего `/`, длина ≤ 64. Иначе `BadZoneName`.
- Нет файла `tzdir/zone.tab` → `TzdataUnavailable`.
- Строки `zone.tab` (без `#`): столбец 1 — код страны, столбец 3 — зона. Пары (country, zone) нет → `ZoneNotInCountry`.
- Файл `tzdir/<zone>` отсутствует → `ZoneMissing`.
- Файлы читаются без следования симлинкам за пределы `tzdir` не требуется: имя уже проверено.

Контракты, которые проверит роль 2:
- **E03 → BI.M3:** `verify_zone` отвечает только по tzdata и не выходит за `tzdir`.
  - (/usr/share/zoneinfo, DE, Europe/Berlin) → Ok
  - (/usr/share/zoneinfo, DE, Europe/Amsterdam) → Err(ZoneNotInCountry)
  - фикстура: zone.tab со строкой `NL\t+5222+00454\tEurope/Nowhere`, файла зоны нет → Err(ZoneMissing)
  - фикстура без zone.tab → Err(TzdataUnavailable)
  - zone "../../etc/passwd", "/Europe/Berlin", "Europe/Ber lin", 65 символов → Err(BadZoneName)

### BI.E1 — Распознать браузер и его версию по `--version`.

Файлы: `src/identity/engine.rs`. Зависит от: —.

`#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)] pub enum Family { Chromium, Gecko }`
`#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)] pub enum Brand { Chrome, Chromium, Brave, Firefox, LibreWolf }`
`#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)] pub struct EngineInfo { pub family: Family, pub brand: Brand, pub major: u32, pub version: String }`
`pub fn parse_version(stdout: &str) -> Result<EngineInfo, EngineError>` и `pub fn detect(path: &Path) -> Result<EngineInfo, EngineError>`.

Разбор первой непустой строки:
- `Google Chrome X` → Chromium/Chrome;
- `Chromium X …` → Chromium/Chromium;
- `Brave Browser X` → Chromium/Brave (major — первое число X, это мажор Chromium);
- `Mozilla Firefox X` → Gecko/Firefox;
- `LibreWolf X` → Gecko/LibreWolf.

`version` — X целиком (до пробела). Иное → `UnknownEngine`.

`detect`:
- путь не абсолютный → `NotAbsolute`;
- запуск `path --version` с `env_clear()` и `PATH=/usr/bin:/bin`, stdin null, вывод ≤ 4 КиБ;
- ожидание не дольше 10 с, затем kill → `Timeout`;
- код ≠ 0 → `VersionFailed`;
- ошибки без вывода процесса.

Контракты, которые проверит роль 2:
- **E04 → BI.M1:** `EngineInfo` описывает настоящий браузер по его собственному `--version`; ничего не придумывается.
  - "Google Chrome 155.0.8059.39 \n" → Chromium, Chrome, 155, "155.0.8059.39"
  - "Chromium 155.0.7000.1 Arch Linux" → Chromium, Chromium, 155, "155.0.7000.1"
  - "Brave Browser 154.1.96.61" → Chromium, Brave, 154, "154.1.96.61"
  - "Mozilla Firefox 157.0" → Gecko, Firefox, 157, "157.0"
  - "LibreWolf 157.0-1" → Gecko, LibreWolf, 157, "157.0-1"
  - "Opera 100.0" и "" → Err(UnknownEngine)
  - detect("chrome") → Err(NotAbsolute)
  - скрипт `exit 3` → Err(VersionFailed); скрипт `sleep 30` → Err(Timeout) за ≤ 12 с
  - скрипт печатает `env`: в выводе нет переменных родителя кроме PATH
- **E04b → BI.E2:** Реестр сопоставляет строки с `EngineInfo` по brand и major, а не по строке версии.
  - EngineInfo{Chrome, 155, "155.0.8059.39"} и {Chrome, 155, "155.0.9999.1"} → одинаковый status
  - brand в JSON: ровно одно из Chrome, Chromium, Brave, Firefox, LibreWolf; неизвестный brand в JSON — ошибка разбора при сборке теста встраивания (тест читает data/identity-verified.json и проверяет каждую строку)

### BI.E2 — Вести реестр проверенных лабораторией версий.

Файлы: `data/identity-verified.json`, `src/identity/verified.rs`. Зависит от: BI.E1.

`data/identity-verified.json` встраивается через `include_str!`. Формат: `[{"brand":"Chrome","major":155,"strategy":"local","evidence":"docs/design/cm-network-manager/i15r-evidence/2026-10-08","note":""}]`.

Строки — только пары, прошедшие приёмку E20 через `cm identity` (evidence `bi-evidence/2026-10-10`):
- `Chrome 155 local`;
- `Brave 154 local`, note `"farbling: navigator.languages сокращается до одного языка"`;
- `LibreWolf 157 local`;
- `LibreWolf 157 crowd`, note `"RFP: часовой пояс Atlantic/Reykjavik, языки en-US"`.

`pub enum Strategy { Local, Crowd }` объявлен в `model.rs` (BI.M1); здесь используется.

`pub enum Verification { Verified { evidence: String, note: String }, Unverified }`
`pub fn status(engine: &EngineInfo, strategy: Strategy) -> Verification` — совпадение brand и major и strategy.

Строки добавляет роль 2 после лабораторного прогона, со ссылкой на evidence. Код при этом не меняется.

Контракты, которые проверит роль 2:
- **E05 → BI.M3:** Реестр отвечает `Verified` только для строк, за которыми стоит evidence того же механизма.
  - (Chrome 155, Local) → Verified, evidence "docs/design/cm-network-manager/bi-evidence/2026-10-10"
  - (Brave 154, Local) → Verified, note содержит "farbling"
  - (LibreWolf 157, Local) → Verified; (LibreWolf 157, Crowd) → Verified, note содержит "RFP"
  - (Chrome 156, Local), (Chromium 155, Local), (Firefox 157, Local), (LibreWolf 158, Crowd), (Chrome 155, Crowd) → Unverified
  - каждая строка JSON: каталог evidence существует в репозитории

### BI.M1 — Описать модель личности браузера.

Файлы: `src/identity/model.rs`, `src/identity/mod.rs`, `src/lib.rs`. Зависит от: BI.R2, BI.E1.

`pub mod identity;` в `src/lib.rs`. Подмодули: `presets, tzdata, engine, verified, model, store, validate, plan, launch, guard, axes, cli`.

`#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "lowercase")] pub enum Strategy { Local, Crowd }`
```rust
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserIdentityProfile {
    pub schema_version: u32,          // = 1
    pub id: String,                   // ^[a-z0-9][a-z0-9-]{0,31}$
    pub strategy: Strategy,
    pub browser: PathBuf,             // абсолютный путь
    pub engine: EngineInfo,           // снимок при создании или последнем запуске
    pub environment: Option<EnvironmentProfile>, // Some только для Local
    pub extra_args: Vec<String>,      // флаги пользователя; запреты — в BI.M3
    pub created_at: i64,
    pub generation: u32,              // растёт при каждом изменении личности
    pub history: Vec<HistoryEntry>,   // не больше HISTORY_LIMIT, старые вытесняются
    pub bypass_suspected: bool,
}
pub struct HistoryEntry { pub generation: u32, pub at: i64, pub change: String }
pub const HISTORY_LIMIT: usize = 5;   // гипотеза Q27, менять только решением пользователя
```
`pub fn validate_id(id: &str) -> Result<(), ModelError>`; `impl BrowserIdentityProfile { pub fn record(&mut self, at: i64, change: &str) }` — generation += 1, запись в history с вытеснением старых.

Неизвестная `schema_version` → `ModelError::UnsupportedSchema`. `Debug` не выводит `extra_args` целиком: только их число.

Контракты, которые проверит роль 2:
- **E06 → BI.M2:** Модель сериализуется без потерь, отвергает чужие поля и неизвестную схему, история ограничена.
  - validate_id: "work", "a", "a-1", 32 символа → Ok; "Work", "-a", "a/b", "a b", "", 33 символа → Err
  - JSON roundtrip профиля DE local == исходный
  - JSON с лишним полем `"x":1` → Err; schema_version 2 → Err(UnsupportedSchema)
  - 7 вызовов record → generation +7, history.len() == 5, history[0].generation — третья запись
  - format!("{:?}") не содержит значений extra_args
- **E08 → BI.M3:** Валидатор получает полный профиль; каждое правило таблицы BI.M3 срабатывает ровно на своём условии.
  - Local + DE + Chrome 155 (профиль и движок совпадают) → errors [], warnings []
  - Crowd + Chrome 155 + environment None → errors [StrategyEngineUnsupported]
  - Crowd + LibreWolf 157 + environment DE → errors [CrowdWithRegion], warnings []
  - Local + environment None → errors [LocalNeedsEnvironment]
  - Local + DE с timezone "Europe/Amsterdam" (подложено вручную) → errors [ZoneInvalid]
  - extra_args ["--user-agent=x"] → errors [ForbiddenArgument("--user-agent")]; ["--remote-debugging-pipe"], ["--headless=new"], ["--lang=ru"], ["--profile"] — по одному коду каждый
  - ["--time-zone-for-testing=Asia/Tokyo"], ["--profile-directory=Other"], ["--ProfileManager"], ["-P"] → по одному ForbiddenArgument с этим флагом
  - extra_args ["--ozone-platform=wayland"] и ["-private-window"] → errors []
  - профиль Chrome 155, движок Chrome 156 → warnings [EngineChanged, UnverifiedVersion]
  - Local + DE + English → warnings [LanguageNotRegional]
  - Local + DE + Brave 154 → warnings [BraveFarblesLanguages]
  - bypass_suspected = true → errors [BypassSuspected]

### BI.M2 — Хранить личности в каталоге CM пользователя.

Файлы: `src/identity/store.rs`. Зависит от: BI.M1.

`pub fn root() -> PathBuf`: `CM_IDENTITY_ROOT`, иначе `$XDG_DATA_HOME/cm/identities`, иначе `$HOME/.local/share/cm/identities`.

`pub struct IdentityStore { root: PathBuf }`, `open(root) -> Result<Self, StoreError>`:
- создаёт `root` 0700;
- отказ `Unsafe`, если root или любой каталог личности — симлинк либо принадлежит другому uid.

Раскладка: `<root>/<id>/identity.json` (0600), `<root>/<id>/profile/` (0700), `<root>/<id>/state.json` (BI.G1), `<root>/<id>/browser.log` (BI.L3), `<root>/<id>/run.lock` (BI.L3).

Методы:
- `create(&profile)` → `Exists`, если каталог есть;
- `load(id)` → `NotFound`;
- `save(&profile)` — атомарно: `identity.json.tmp`, fsync, rename;
- `list()` — id по алфавиту; каталоги без `identity.json` пропускаются;
- `remove(id, purge: bool)` — без purge удаляет только `identity.json` и `state.json`, профиль браузера остаётся;
- `dir(id)`, `profile_dir(id)`.

Оставшийся `.tmp` не читается и перезаписывается при следующем `save`.

Контракты, которые проверит роль 2:
- **E07 → BI.G1:** Хранилище даёт каталог личности, принадлежащий пользователю, с правами 0700/0600 и атомарной записью.
  - create(work) → <root>/work 0700, identity.json 0600, profile/ 0700
  - create(work) повторно → Err(Exists); load(none) → Err(NotFound)
  - оставленный identity.json.tmp с мусором: load(work) читает прежний identity.json
  - list() при каталогах b, a, c-без-json → [a, b]
  - remove(work, false) → identity.json и state.json удалены, profile/ на месте; remove(work, true) → каталога нет
  - root — симлинк → open → Err(Unsafe)

### BI.M3 — Проверить согласованность личности до запуска.

Файлы: `src/identity/validate.rs`. Зависит от: BI.M1, BI.E2, BI.R3.

`pub fn validate(p: &BrowserIdentityProfile, engine: &EngineInfo, tzdir: &Path, lab: bool) -> Report`
`pub struct Report { pub errors: Vec<Violation>, pub warnings: Vec<Violation> }` — запуск разрешён только при пустом `errors`.
`#[derive(Clone, Debug, PartialEq, Eq)] pub enum Violation` — коды и правила (порядок вывода — порядок правил):

| # | Код | Вид | Условие |
|---|---|---|---|
| 1 | `StrategyEngineUnsupported` | error | `Crowd` и `engine.family == Chromium` (crowd требует Gecko с RFP) |
| 2 | `CrowdWithRegion` | error | `Crowd` и `environment.is_some()` |
| 3 | `LocalNeedsEnvironment` | error | `Local` и `environment.is_none()` |
| 4 | `ZoneInvalid` | error | `Local` и `verify_zone(tzdir, country, timezone)` не Ok |
| 5 | `ForbiddenArgument(String)` | error | в `extra_args` аргумент, начинающийся с одного из: `--user-agent`, `--remote-debugging-port`, `--remote-debugging-pipe`, `--enable-automation`, `--headless`, `--lang`, `--accept-lang`, `--user-data-dir`, `--profile`, `-profile`, `--no-sandbox`, `--marionette`, `--remote-allow`, `--time-zone-for-testing` (перекрывает `TZ`), `--profile-directory`, `--ProfileManager`, `-ProfileManager` (другой профиль вне отпечатка охраны); точное совпадение — `-P` (префикс задел бы `-private-window`); в `String` — самый длинный совпавший флаг |
| 6 | `EngineChanged` | warning | brand или major `engine` отличаются от `p.engine` |
| 7 | `UnverifiedVersion` | warning | `verified::status(engine, strategy) == Unverified` |
| 8 | `LanguageNotRegional` | warning | `Local` и `!environment.matches_country` |
| 9 | `BraveFarblesLanguages` | warning | `Local` и brand `Brave` |
| 10 | `BypassSuspected` | error | `p.bypass_suspected` (охрана, BI.G2) |

`lab == true` не меняет правил; флаги лаборатории добавляет план (BI.L1/L2), не пользователь.

Контракты, которые проверит роль 2:
- **E09 → BI.L1:** План строится только для профиля без errors; план не добавляет ничего, что валидатор запрещает пользователю.
  - parent_env {DISPLAY=:0, WAYLAND_DISPLAY=wayland-1, XAUTHORITY=/h/.Xauthority, XDG_RUNTIME_DIR=/run/user/1000, DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus, HOME=/h, PATH=/usr/bin, LC_TIME=ru_RU.UTF-8, LANGUAGE=ru, TZ=Europe/Moscow, SECRET_TOKEN=x}
  - chromium_plan(DE local, /r/work/profile, env, None, false).args == ["--user-data-dir=/r/work/profile", "--lang=de-DE", "--accept-lang=de-DE,de,en-US,en", "--no-first-run", "--no-default-browser-check"]
  - env == {DBUS_SESSION_BUS_ADDRESS, DISPLAY, HOME=/h, LANG=de_DE.UTF-8, PATH=/usr/bin, TZ=Europe/Berlin, WAYLAND_DISPLAY, XAUTHORITY, XDG_RUNTIME_DIR} — ровно 9 ключей; нет LC_TIME, LANGUAGE, SECRET_TOKEN
  - parent_env без PATH → PATH=/usr/bin:/bin
  - url "http://127.0.0.1:18765/" — последний аргумент; extra_args ["--ozone-platform=wayland"] — сразу перед url
  - lab=true → после `--no-default-browser-check` идут ровно "--headless=new", "--no-sandbox", "--disable-popup-blocking"; без lab `--disable-popup-blocking` нет
  - в argv нет `--user-agent`, `--remote-debugging-*`, `--enable-automation`; files == []
- **E10 → BI.L2:** user.js целиком задаётся стратегией; UA-prefs не пишутся.
  - Local + DE: user.js == "// Managed by cm identity. Changes here are overwritten at launch.\nuser_pref(\"intl.accept_languages\", \"de-DE, de, en-US, en\");\nuser_pref(\"intl.locale.requested\", \"de-DE\");\nuser_pref(\"privacy.resistFingerprinting\", false);\n"
  - Crowd: user.js == "// Managed by cm identity. Changes here are overwritten at launch.\nuser_pref(\"intl.accept_languages\", \"en-US, en\");\nuser_pref(\"intl.locale.requested\", \"en-US\");\nuser_pref(\"privacy.resistFingerprinting\", true);\n"
  - Local env: TZ=Europe/Berlin, LANG=de_DE.UTF-8; Crowd env: LANG=en_US.UTF-8 и нет TZ
  - args == ["--profile", "/r/work/profile", "--no-remote"]; lab=true → + "--headless"
  - lab=true: user.js == строка Local DE + "user_pref(\"dom.disable_open_during_load\", false);\n"; без lab этой строки нет
  - ни одна строка user.js не содержит "useragent"
- **E17b → BI.C1:** Оси строятся из `Report` валидатора без повторной проверки и без доступа к сети.
  - Report{errors [], warnings [UnverifiedVersion]} → APP Partial
  - Report{errors [ForbiddenArgument("--user-agent")], warnings []} → APP Blocked «личность не согласована»

### BI.L1 — Построить план запуска Chromium-браузера.

Файлы: `src/identity/plan.rs`. Зависит от: BI.M3.

`pub struct FileWrite { pub path: PathBuf, pub contents: String }`
`pub struct LaunchPlan { pub program: PathBuf, pub args: Vec<String>, pub env: BTreeMap<String, String>, pub files: Vec<FileWrite> }`
`pub fn chromium_plan(p: &BrowserIdentityProfile, profile_dir: &Path, parent_env: &BTreeMap<String, String>, url: Option<&str>, lab: bool) -> LaunchPlan` — чистая функция, только для `Local`.

**args** строго в таком порядке:
1. `--user-data-dir=<profile_dir>`
2. `--lang=<primary>`
3. `--accept-lang=<accept_language()>`
4. `--no-first-run`
5. `--no-default-browser-check`
6. при `lab`: `--headless=new`, `--no-sandbox`, `--disable-popup-blocking` (страница лаборатории открывает вкладку скриптом, без жеста пользователя)
7. `extra_args` по порядку
8. `url`, если есть

**env** — только:
- из `parent_env`, если есть: `DISPLAY`, `WAYLAND_DISPLAY`, `XAUTHORITY`, `XDG_RUNTIME_DIR`, `DBUS_SESSION_BUS_ADDRESS`, `HOME`, `PATH` (по умолчанию `/usr/bin:/bin`), `XDG_CONFIG_HOME`, `XDG_CACHE_HOME`, `PULSE_SERVER`;
- плюс `TZ=<timezone>` и `LANG=<posix_locale>`.

Никаких `LC_*`, `LANGUAGE` и прочих переменных родителя. `files` пуст.

Контракты, которые проверит роль 2:
- **E15 → BI.L3:** Исполнитель передаёт браузеру ровно план: те же argv и env, ничего от родителя.
  - launch(work DE local, None, false) → Ok(0); записанный argv == plan.args; записанный env == plan.env (родитель с SECRET_TOKEN=x — его нет)
  - браузер запущен в новой сессии: getsid(pid) == pid (скрипт пишет `ps -o sid= -p $$`)
  - browser.log создан с правами 0600
  - после выхода state.json содержит last_launch и last_exit

### BI.L2 — Построить план запуска Gecko-браузера и его user.js.

Файлы: `src/identity/plan.rs`. Зависит от: BI.M3.

`pub fn gecko_plan(p, profile_dir, parent_env, url, lab) -> LaunchPlan`.

**args:** `--profile`, `<profile_dir>`, `--no-remote`, при `lab` — `--headless`, затем `extra_args`, затем `url`.

**env:** тот же allowlist, что в BI.L1. Дополнительно:
- `Local` — `TZ=<timezone>`, `LANG=<posix_locale>`;
- `Crowd` — `LANG=en_US.UTF-8`, без `TZ`.

**files:** ровно один `FileWrite` — `<profile_dir>/user.js`, первая строка `// Managed by cm identity. Changes here are overwritten at launch.`, затем:
- `Local`:
  - `user_pref("intl.accept_languages", "<языки через ", ">");`
  - `user_pref("intl.locale.requested", "<primary>");`
  - `user_pref("privacy.resistFingerprinting", false);`
- `Crowd`:
  - `user_pref("intl.accept_languages", "en-US, en");`
  - `user_pref("intl.locale.requested", "en-US");`
  - `user_pref("privacy.resistFingerprinting", true);`

При `lab` в конец добавляется `user_pref("dom.disable_open_during_load", false);` — по той же причине, что `--disable-popup-blocking` у Chromium. Без `lab` блокировщик окон не трогается.

Строки завершаются `\n`. UA-prefs (`general.useragent.override`) не пишутся никогда.

Контракты, которые проверит роль 2:
- **E16 → BI.L3:** Для Gecko исполнитель пишет user.js до запуска и не трогает другие файлы профиля.
  - после launch profile/user.js == строка из E10 (Local DE); права 0600
  - заранее положенный profile/prefs.js не изменён

### BI.G1 — Снимать отпечаток файлов профиля и вести маркеры запуска.

Файлы: `src/identity/guard.rs`. Зависит от: BI.M2.

Ключевые файлы:
- Chromium — `Default/Preferences`, `Local State`;
- Gecko — `prefs.js`, `times.json`.

`pub struct FileMark { pub path: String /* относительный */, pub present: bool, pub size: u64, pub mtime_ns: i128, pub sha256: String }`
`pub fn fingerprint(profile_dir: &Path, family: Family) -> Vec<FileMark>` — отсутствующий файл: `present=false`, остальные поля нули и пустая строка.
`#[derive(Serialize, Deserialize)] pub struct GuardState { pub last_launch: Option<LaunchMark>, pub last_exit: Option<ExitMark> }`
`pub struct LaunchMark { pub generation: u32, pub pid: u32, pub started_at: i64 }`
`pub struct ExitMark { pub at: i64, pub files: Vec<FileMark> }`

Функции:
- `load_state(dir)` — нет файла → `GuardState::default()`;
- `record_launch(dir, generation, pid, at)`;
- `record_exit(dir, profile_dir, family, at)`.

Запись атомарная, 0600, файл `<dir>/state.json`.

Контракты, которые проверит роль 2:
- **E11 → BI.G2:** Отпечаток меняется при любой правке ключевого файла и не меняется без правок; маркеры переживают перезапуск процесса CM.
  - пустой профиль Chromium → 2 записи present=false
  - запись `Default/Preferences` = "{}" → present=true, size 2, sha256 44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a
  - два fingerprint без изменений → равны; изменение одного байта → не равны
  - record_launch затем load_state → last_launch == записанное; state.json 0600

### BI.G2 — Обнаружить запуск профиля мимо CM перед новым запуском.

Файлы: `src/identity/guard.rs`. Зависит от: BI.G1.

`pub enum GuardOutcome { FirstLaunch, Clean, Warned }`, `pub enum GuardError { BypassSuspected, BrowserRunning }`
`pub fn check_before_launch(store: &IdentityStore, p: &mut BrowserIdentityProfile, family: Family, policy: BypassPolicy, is_alive: &dyn Fn(u32) -> bool) -> Result<GuardOutcome, GuardError>`; `pub enum BypassPolicy { Block, Warn }` — из `cm.conf` ключ `identity_bypass` (`block` по умолчанию, `warn`).

Порядок проверок:
1. **Блокировка профиля чужим процессом.**
   - Chromium: `profile/SingletonLock` — симлинк вида `<host>-<pid>`.
   - Gecko: `profile/lock` — симлинк вида `<ip>:+<pid>`.
   - Если pid разобран и `is_alive(pid)` → `BrowserRunning`. Мёртвый pid — устаревшая блокировка, её не трогать.
2. **Уже подозрение:** `p.bypass_suspected` и `Block` → `BypassSuspected`.
3. **Сравнение отпечатка.** Нет `last_exit`:
   - `FirstLaunch`, если `last_launch` тоже нет;
   - иначе предыдущий запуск не завершился через CM → то же, что расхождение.
4. **Расхождение.** Текущий `fingerprint` ≠ `last_exit.files`:
   - при `Block` выставить `p.bypass_suspected = true`, `p.record(now, "bypass-suspected")`, сохранить, вернуть `BypassSuspected`;
   - при `Warn` — `p.record(now, "bypass-warned")`, сохранить, `Warned`.
5. Иначе `Clean`.

Контракты, которые проверит роль 2:
- **E12 → BI.L3:** До запуска охрана отвечает однозначно; при обходе состояние сохраняется до подтверждения.
  - новая личность без state.json → Ok(FirstLaunch)
  - record_exit, файлы не менялись → Ok(Clean)
  - record_exit, затем Default/Preferences изменён, policy Block → Err(BypassSuspected); bypass_suspected сохранён в identity.json; history последняя запись "bypass-suspected"
  - повторный вызов без изменений → снова Err(BypassSuspected)
  - то же при policy Warn → Ok(Warned), bypass_suspected false, history "bypass-warned"
  - SingletonLock → "host-4242", is_alive(4242) == true → Err(BrowserRunning); is_alive false → проверка продолжается
  - Gecko: `lock` → "127.0.0.1:+4242", is_alive true → Err(BrowserRunning)
  - last_launch есть, last_exit нет (CM упал), policy Block → Err(BypassSuspected)
- **E13 → BI.C1:** Состояние охраны отражается в оси APP.
  - bypass_suspected → APP Blocked, reason "профиль открывали мимо CM: нужно cm identity confirm"
- **E13b → BI.G3:** Подтверждению нужен сохранённый признак обхода и тот же формат отпечатка.
  - после Err(BypassSuspected) identity.json содержит "bypass_suspected": true — confirm читает его из хранилища, а не из памяти процесса

### BI.G3 — Подтвердить личность после обнаруженного обхода.

Файлы: `src/identity/guard.rs`. Зависит от: BI.G2.

`pub fn confirm(store: &IdentityStore, p: &mut BrowserIdentityProfile, now: i64)`:
- `bypass_suspected = false`;
- `p.record(now, "bypass-confirmed")`;
- `GuardState.last_exit` заменяется текущим отпечатком (иначе следующий запуск снова увидит расхождение);
- сохранить.

Повторный `confirm` без подозрения — ничего не меняет и не пишет history.

Контракты, которые проверит роль 2:
- **E14 → BI.C2:** Подтверждение снимает блокировку один раз и обновляет эталон отпечатка.
  - после BypassSuspected: confirm → bypass_suspected false, history "bypass-confirmed", следующий check_before_launch → Ok(Clean)
  - confirm без подозрения → generation и history не меняются

### BI.L3 — Запустить браузер по плану и сопровождать его до выхода.

Файлы: `src/identity/launch.rs`. Зависит от: BI.L1, BI.L2, BI.G2.

`pub fn launch(store: &IdentityStore, id: &str, url: Option<&str>, lab: bool) -> Result<i32 /* код выхода браузера */, LaunchError>`.

Шаги:
1. Отказ `RunAsRoot`, если euid == 0 и не `test_mode()`.
2. `run.lock` через `flock(LOCK_EX|LOCK_NB)` → `AlreadyRunning`.
3. `load` и `engine::detect(browser)`. Если снимок отличается — обновить `p.engine` и сохранить. `p.record(now, "engine <brand> <major>")` — только при смене brand или major: патч-релиз не тратит поколение и запись истории.
4. `validate` (errors → `Invalid(Vec<Violation>)`).
5. `guard::check_before_launch` (ошибка → `Guard(GuardError)`).
6. План по семейству (Chromium → `chromium_plan`, Gecko → `gecko_plan`); записать `files` 0600 атомарно.
7. Запуск:
   - `Command::new(program)`, `args`, `env_clear()` + `env`;
   - stdin null, stdout и stderr → `browser.log` (0600, перезапись);
   - `pre_exec(setsid)` с комментарием `// SAFETY:`.
8. `record_launch(generation, pid, now)`; ожидать завершения опросом `try_wait`. На время ожидания установлены обработчики `SIGINT`, `SIGTERM`, `SIGHUP`: при сигнале CM один раз шлёт `SIGTERM` сессии браузера (`kill(-pid)`) и продолжает ждать. Иначе Ctrl+C или закрытый терминал оставили бы браузер без CM, и следующий запуск принял бы это за обход.
9. `record_exit(...)`. Вернуть код выхода (сигнал → 128 + номер).

Ошибки `LaunchError: RunAsRoot, AlreadyRunning, Store(StoreError), Engine(EngineError), Invalid(Vec<Violation>), Guard(GuardError), Spawn` — `Display` через `t!`, без argv и env.

Контракты, которые проверит роль 2:
- **E17 → BI.C2:** Отказы исполнителя различимы по типу и не запускают браузер.
  - второй launch той же личности, пока первый ждёт (скрипт `sleep 2`) → Err(AlreadyRunning), второй скрипт не запускался
  - профиль с extra_args ["--user-agent=x"] → Err(Invalid([ForbiddenArgument("--user-agent")])), файл argv не создан
  - фикстура меняет --version на 156 → профиль сохранён с engine 156 и history "engine Chrome 156"
  - фикстура меняет --version на 155.0.9999.1 → engine.version обновлён, generation и history.len() прежние
  - `cm identity launch` получает SIGINT, пока браузер-фикстура спит 60 с → cm завершается сам за ≤ 10 с, процесса браузера нет, state.json содержит last_exit, следующий launch → код 0 (не BypassSuspected)
  - euid 0 без test_mode → Err(RunAsRoot) (проверка, если тест запущен от root; иначе тест печатает NOT_APPLICABLE)

### BI.C1 — Вычислить оси REGION/APP/STATE/NET для личности.

Файлы: `src/identity/axes.rs`. Зависит от: BI.M3, BI.G2.

`pub enum AxisValue { Verified, Partial, Unknown, Blocked }`, `pub struct Axis { pub value: AxisValue, pub reason: &'static str }`
`pub struct Axes { pub net: Axis, pub region: Axis, pub state: Axis, pub app: Axis }`
`pub fn axes(p: &BrowserIdentityProfile, report: &Report) -> Axes` — правила (`reason` — ключи `t!`):

**NET** — всегда `Unknown`, «отдельного туннеля для личности пока нет (I08)».

**REGION:**
- `Crowd` → `Partial`, «crowd: часовой пояс UTC и язык en-US намеренно»;
- `Local` и в `report` есть `LanguageNotRegional` → `Partial`, «язык не из пресета страны»;
- иначе `Partial`, «страна задана вручную, выход туннеля не проверен».

`Verified` для REGION не выдаётся до I14.T01.

**STATE:**
- `Blocked`, если есть `ZoneInvalid` — «часовой пояс не найден в tzdata»;
- иначе `Verified`, «профиль в каталоге CM».

**APP:**
- `Blocked`, если есть `BypassSuspected` — «профиль открывали мимо CM: нужно cm identity confirm»;
- иначе `Blocked`, если есть другие errors — «личность не согласована»;
- иначе `Partial`, если есть `UnverifiedVersion` или `EngineChanged` — «версия браузера не проверена лабораторией»;
- иначе `Verified`, «версия проверена лабораторией».

Контракты, которые проверит роль 2:
- **E18 → BI.C2:** Оси не выдают Verified там, где нет проверки.
  - Local DE Chrome 155 без предупреждений → NET Unknown, REGION Partial «страна задана вручную, выход туннеля не проверен», STATE Verified, APP Verified
  - Local DE English → REGION Partial «язык не из пресета страны»
  - Crowd LibreWolf 157 → REGION Partial «crowd: часовой пояс UTC и язык en-US намеренно», APP Verified; LibreWolf 158 → APP Partial (UnverifiedVersion)
  - ZoneInvalid → STATE Blocked, APP Blocked
  - ни при каких входах REGION и NET не равны Verified

### BI.C2 — Дать пользователю команды cm identity.

Файлы: `src/identity/cli.rs`, `src/main.rs`, `src/i18n_table.rs`. Зависит от: BI.L3, BI.G3, BI.C1.

`src/main.rs`: как для `source` — `if args.first() == "identity" { exit(cm::identity::cli::dispatch(&args[1..])) }`. Без sudo: личность принадлежит пользователю.

**Команды и коды выхода:**
- `cm identity list` — строки `<id>  <strategy>  <brand> <major>  <country|->`;
- `cm identity create ID --browser PATH|auto --strategy local|crowd [--country CC] [--zone ZONE] [--lang local[:N]|en|TAG] [-- EXTRA...]`:
  - `auto` — первый существующий из `/usr/bin/google-chrome-stable, /usr/bin/chromium, /usr/bin/brave, /usr/bin/librewolf, /usr/bin/firefox`;
  - для `local` `--country` обязателен;
  - печатает итоговое окружение и предупреждения;
- `cm identity show ID` — поля личности, окружение, оси, предупреждения; без `extra_args` целиком (только число);
- `cm identity check ID` — `validate` и оси, ничего не запускает;
- `cm identity launch ID [URL] [--lab]` — `--lab` принимается только при `CM_IDENTITY_LAB=1` в окружении (иначе ошибка использования);
- `cm identity confirm ID`;
- `cm identity remove ID [--purge]`.

Коды выхода: 0 — успех; 2 — использование или валидация; 3 — охрана (`BypassSuspected`, `BrowserRunning`, `AlreadyRunning`); 4 — запуск браузера не удался или `RunAsRoot`; 5 — хранилище.

**Ошибки** печатаются как `cm identity: <фраза> [<код>]`. Код — имя варианта `Violation`, `GuardError` или `LaunchError`.

**Строки:** все пользовательские через `t!`, переводы на 6 языков в `src/i18n_table.rs` (дописать в конец, существующие строки не трогать). Справка `cm identity` — константа с переводом.

Контракты, которые проверит роль 2:
- **E19 → BI.C3:** Команды стабильны по выводу и кодам выхода; строки переведены на 6 языков.
  - create work --browser <fixture chrome> --strategy local --country DE → код 0; identity.json: environment DE, engine Chrome 155
  - create work повторно → код 5; create x --strategy local без --country → код 2
  - create c --browser <fixture chrome> --strategy crowd → код 2, вывод содержит [StrategyEngineUnsupported]
  - check work → код 0, вывод содержит NET, REGION, STATE, APP
  - launch work при изменённом profile/Default/Preferences после прошлого выхода → код 3, [BypassSuspected]; confirm work → 0; launch work → 0
  - launch work --lab без CM_IDENTITY_LAB=1 → код 2
  - create … -- --user-agent=x → код 2, [ForbiddenArgument]
  - для каждого из 6 языков (`CM_CONF` с `lang = auto` и `LC_ALL` = ru_RU.UTF-8, en_US.UTF-8, de_DE.UTF-8, it_IT.UTF-8, zh_CN.UTF-8, ar_EG.UTF-8): справка `cm identity` не содержит кириллицы, кроме ru

### BI.C3 — Добавить личность в справку и состояние cm.

Файлы: `src/main.rs`, `src/i18n_table.rs`. Зависит от: BI.C2.

- В общую справку `cm` — строка `cm identity …  личности браузера (стратегии local и crowd)` на 6 языках.
- `cm identity` без аргументов печатает справку и выходит с кодом 2.
- Больше ничего в существующих экранах не меняется: TUI-страница личностей — I17.

Контракты, которые проверит роль 2:
- **E20 → BI.Z:** Лабораторная приёмка: через `cm identity` браузер ведёт себя ровно как в протоколе I15R-LAB, механизм env-local.
  - Chrome 155, `create lab --strategy local --country DE`: первый запрос `Accept-Language` начинается с "de-DE,de;q=0.9"; UA без изменений (содержит "Chrome/155", нет "CMLab"); окно, Worker, вкладка: tz "Europe/Berlin", offset −60 (январь), `Intl` locale "de", languages [de-DE, de, en-US, en], webdriver false
  - Brave 154, то же: tz Europe/Berlin, `Intl` de, languages [de-DE], webdriver false; `Accept-Language` начинается с "de-DE" (q Brave меняет сам, от запуска к запуску — не проверяется)
  - во всех четырёх личностях новая вкладка (`window.open`) даёт те же значения, что окно
  - LibreWolf 157 crowd: UA содержит "Firefox/157.0"; tz "Atlantic/Reykjavik" или "UTC"; languages [en-US, en]; webdriver false
  - LibreWolf 157 local DE: UA без изменений; tz Europe/Berlin; `Intl` locale "de"; Accept-Language начинается с "de-DE"
  - голый запуск браузера с `--user-data-dir=<profile>` (Chrome) или `--profile` (LibreWolf) мимо CM, затем `cm identity launch lab --lab` → код 3 [BypassSuspected]
  - по результатам: строки в data/identity-verified.json для каждой проверенной пары (brand, major, strategy) со ссылкой на bi-evidence

## Сдача

Отчёт:
- по каждой вершине: сделано или открыто с причиной;
- `git diff --stat` по существующим файлам;
- EXIT и счётчики gate;
- список мест, где спецификация допускала толкование, и выбранное толкование.
