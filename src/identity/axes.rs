//! Оси REGION, APP, STATE и NET для личности.

use super::model::{BrowserIdentityProfile, Strategy};
use super::validate::{Report, Violation};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AxisValue {
    Verified,
    Partial,
    Unknown,
    Blocked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Axis {
    pub value: AxisValue,
    pub reason: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Axes {
    pub net: Axis,
    pub region: Axis,
    pub state: Axis,
    pub app: Axis,
}

pub fn axes(profile: &BrowserIdentityProfile, report: &Report) -> Axes {
    let net = axis(
        AxisValue::Unknown,
        "отдельного туннеля для личности пока нет (I08)",
    );
    let region = if profile.strategy == Strategy::Crowd {
        axis(
            AxisValue::Partial,
            "crowd: часовой пояс UTC и язык en-US намеренно",
        )
    } else if has(report, |item| {
        matches!(item, Violation::LanguageNotRegional)
    }) {
        axis(AxisValue::Partial, "язык не из пресета страны")
    } else {
        axis(
            AxisValue::Partial,
            "страна задана вручную, выход туннеля не проверен",
        )
    };
    let state = if report
        .errors
        .iter()
        .any(|item| matches!(item, Violation::ZoneInvalid))
    {
        axis(AxisValue::Blocked, "часовой пояс не найден в tzdata")
    } else {
        axis(AxisValue::Verified, "профиль в каталоге CM")
    };
    let app = if report
        .errors
        .iter()
        .any(|item| matches!(item, Violation::BypassSuspected))
    {
        axis(
            AxisValue::Blocked,
            "профиль открывали мимо CM: нужно cm identity confirm",
        )
    } else if !report.errors.is_empty() {
        axis(AxisValue::Blocked, "личность не согласована")
    } else if report.warnings.iter().any(|item| {
        matches!(
            item,
            Violation::UnverifiedVersion | Violation::EngineChanged
        )
    }) {
        axis(
            AxisValue::Partial,
            "версия браузера не проверена лабораторией",
        )
    } else {
        axis(AxisValue::Verified, "версия проверена лабораторией")
    };
    Axes {
        net,
        region,
        state,
        app,
    }
}

fn axis(value: AxisValue, reason: &'static str) -> Axis {
    Axis { value, reason }
}

fn has(report: &Report, pred: impl Fn(&Violation) -> bool) -> bool {
    report.errors.iter().chain(report.warnings.iter()).any(pred)
}
