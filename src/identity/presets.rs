//! Пресеты страны и выбор окружения личности.

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LanguageSet {
    pub tags: &'static [&'static str],
    pub posix: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Preset {
    pub country: &'static str,
    pub zones: &'static [&'static str],
    pub language_sets: &'static [LanguageSet],
}

// Источник зон — tzdata zone.tab; языки — официальные языки страны по CLDR;
// набор стран ограничен и расширяется только вместе с проверкой ребра BI.R1→BI.R2.
pub static PRESETS: &[Preset] = &[
    Preset {
        country: "AT",
        zones: &["Europe/Vienna"],
        language_sets: &[LanguageSet {
            tags: &["de-AT", "de", "en-US", "en"],
            posix: "de_AT.UTF-8",
        }],
    },
    Preset {
        country: "CA",
        zones: &[
            "America/Toronto",
            "America/Vancouver",
            "America/Edmonton",
            "America/Winnipeg",
            "America/Halifax",
        ],
        language_sets: &[
            LanguageSet {
                tags: &["en-CA", "en"],
                posix: "en_CA.UTF-8",
            },
            LanguageSet {
                tags: &["fr-CA", "fr", "en-CA", "en"],
                posix: "fr_CA.UTF-8",
            },
        ],
    },
    Preset {
        country: "CH",
        zones: &["Europe/Zurich"],
        language_sets: &[
            LanguageSet {
                tags: &["de-CH", "de", "en-US", "en"],
                posix: "de_CH.UTF-8",
            },
            LanguageSet {
                tags: &["fr-CH", "fr", "en-US", "en"],
                posix: "fr_CH.UTF-8",
            },
        ],
    },
    Preset {
        country: "DE",
        zones: &["Europe/Berlin"],
        language_sets: &[LanguageSet {
            tags: &["de-DE", "de", "en-US", "en"],
            posix: "de_DE.UTF-8",
        }],
    },
    Preset {
        country: "EE",
        zones: &["Europe/Tallinn"],
        language_sets: &[LanguageSet {
            tags: &["et-EE", "et", "en-US", "en"],
            posix: "et_EE.UTF-8",
        }],
    },
    Preset {
        country: "FI",
        zones: &["Europe/Helsinki"],
        language_sets: &[LanguageSet {
            tags: &["fi-FI", "fi", "en-US", "en"],
            posix: "fi_FI.UTF-8",
        }],
    },
    Preset {
        country: "FR",
        zones: &["Europe/Paris"],
        language_sets: &[LanguageSet {
            tags: &["fr-FR", "fr", "en-US", "en"],
            posix: "fr_FR.UTF-8",
        }],
    },
    Preset {
        country: "GB",
        zones: &["Europe/London"],
        language_sets: &[LanguageSet {
            tags: &["en-GB", "en"],
            posix: "en_GB.UTF-8",
        }],
    },
    Preset {
        country: "JP",
        zones: &["Asia/Tokyo"],
        language_sets: &[LanguageSet {
            tags: &["ja-JP", "ja", "en-US", "en"],
            posix: "ja_JP.UTF-8",
        }],
    },
    Preset {
        country: "LT",
        zones: &["Europe/Vilnius"],
        language_sets: &[LanguageSet {
            tags: &["lt-LT", "lt", "en-US", "en"],
            posix: "lt_LT.UTF-8",
        }],
    },
    Preset {
        country: "LV",
        zones: &["Europe/Riga"],
        language_sets: &[LanguageSet {
            tags: &["lv-LV", "lv", "en-US", "en"],
            posix: "lv_LV.UTF-8",
        }],
    },
    Preset {
        country: "NL",
        zones: &["Europe/Amsterdam"],
        language_sets: &[LanguageSet {
            tags: &["nl-NL", "nl", "en-US", "en"],
            posix: "nl_NL.UTF-8",
        }],
    },
    Preset {
        country: "PL",
        zones: &["Europe/Warsaw"],
        language_sets: &[LanguageSet {
            tags: &["pl-PL", "pl", "en-US", "en"],
            posix: "pl_PL.UTF-8",
        }],
    },
    Preset {
        country: "RU",
        zones: &[
            "Europe/Moscow",
            "Asia/Yekaterinburg",
            "Asia/Novosibirsk",
            "Asia/Vladivostok",
        ],
        language_sets: &[LanguageSet {
            tags: &["ru-RU", "ru", "en-US", "en"],
            posix: "ru_RU.UTF-8",
        }],
    },
    Preset {
        country: "SE",
        zones: &["Europe/Stockholm"],
        language_sets: &[LanguageSet {
            tags: &["sv-SE", "sv", "en-US", "en"],
            posix: "sv_SE.UTF-8",
        }],
    },
    Preset {
        country: "SG",
        zones: &["Asia/Singapore"],
        language_sets: &[LanguageSet {
            tags: &["en-SG", "en"],
            posix: "en_SG.UTF-8",
        }],
    },
    Preset {
        country: "TR",
        zones: &["Europe/Istanbul"],
        language_sets: &[LanguageSet {
            tags: &["tr-TR", "tr", "en-US", "en"],
            posix: "tr_TR.UTF-8",
        }],
    },
    Preset {
        country: "US",
        zones: &[
            "America/New_York",
            "America/Chicago",
            "America/Denver",
            "America/Phoenix",
            "America/Los_Angeles",
            "America/Anchorage",
            "Pacific/Honolulu",
        ],
        language_sets: &[LanguageSet {
            tags: &["en-US", "en"],
            posix: "en_US.UTF-8",
        }],
    },
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LanguageChoice {
    Local(usize),
    English,
    Custom(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentProfile {
    pub country: String,
    pub timezone: String,
    pub languages: Vec<String>,
    pub posix_locale: String,
    pub matches_country: bool,
}

impl EnvironmentProfile {
    /// Языки через запятую без пробелов.
    pub fn accept_language(&self) -> String {
        self.languages.join(",")
    }

    pub fn primary(&self) -> &str {
        self.languages.first().map(String::as_str).unwrap_or("")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresetError {
    UnknownCountry,
    ZoneNotInCountry,
    NoSuchLanguageSet,
    BadLanguageTag,
}

impl PresetError {
    pub fn code(self) -> &'static str {
        match self {
            Self::UnknownCountry => "UnknownCountry",
            Self::ZoneNotInCountry => "ZoneNotInCountry",
            Self::NoSuchLanguageSet => "NoSuchLanguageSet",
            Self::BadLanguageTag => "BadLanguageTag",
        }
    }
}

impl fmt::Display for PresetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnknownCountry => t!("неизвестная страна"),
            Self::ZoneNotInCountry => t!("часовой пояс не относится к стране"),
            Self::NoSuchLanguageSet => t!("нет такого набора языков"),
            Self::BadLanguageTag => t!("неверный языковой тег"),
        })
    }
}

pub fn select(
    country: &str,
    zone: Option<&str>,
    language: &LanguageChoice,
) -> Result<EnvironmentProfile, PresetError> {
    let country = country.to_ascii_uppercase();
    let preset = PRESETS
        .iter()
        .find(|item| item.country == country)
        .ok_or(PresetError::UnknownCountry)?;
    let timezone = match zone {
        None => preset.zones.first().copied().unwrap_or("").to_string(),
        Some(name) if preset.zones.contains(&name) => name.to_string(),
        Some(_) => return Err(PresetError::ZoneNotInCountry),
    };
    let (languages, posix_locale) = match language {
        LanguageChoice::Local(index) => {
            let set = preset
                .language_sets
                .get(*index)
                .ok_or(PresetError::NoSuchLanguageSet)?;
            (
                set.tags.iter().map(|tag| (*tag).to_string()).collect(),
                set.posix.to_string(),
            )
        }
        LanguageChoice::English => (
            vec!["en-US".to_string(), "en".to_string()],
            "en_US.UTF-8".to_string(),
        ),
        LanguageChoice::Custom(tag) => parse_custom_tag(tag)?,
    };
    let matches_country = preset
        .language_sets
        .iter()
        .any(|set| same_tags(set.tags, &languages));
    Ok(EnvironmentProfile {
        country,
        timezone,
        languages,
        posix_locale,
        matches_country,
    })
}

fn same_tags(tags: &[&str], languages: &[String]) -> bool {
    tags.len() == languages.len()
        && tags
            .iter()
            .zip(languages)
            .all(|(tag, language)| *tag == language)
}

/// `ll` или `ll-RR`: 2–3 строчные латинские буквы и необязательный регион из 2 заглавных.
fn parse_custom_tag(tag: &str) -> Result<(Vec<String>, String), PresetError> {
    let bytes = tag.as_bytes();
    let lang_len = bytes
        .iter()
        .take_while(|byte| byte.is_ascii_lowercase())
        .count();
    if !(2..=3).contains(&lang_len) {
        return Err(PresetError::BadLanguageTag);
    }
    let lang = &tag[..lang_len];
    let rest = &tag[lang_len..];
    let region = if rest.is_empty() {
        None
    } else {
        let region = rest.strip_prefix('-').ok_or(PresetError::BadLanguageTag)?;
        if region.len() != 2 || !region.bytes().all(|byte| byte.is_ascii_uppercase()) {
            return Err(PresetError::BadLanguageTag);
        }
        Some(region)
    };
    let languages = if region.is_some() {
        vec![tag.to_string(), lang.to_string()]
    } else {
        vec![lang.to_string()]
    };
    let region_owned = match region {
        Some(region) => region.to_string(),
        None => lang.to_uppercase(),
    };
    Ok((languages, format!("{lang}_{region_owned}.UTF-8")))
}
