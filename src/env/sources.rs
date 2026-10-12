//! Страна берётся только из ответа источника и только как две буквы.

use crate::common::test_mode;

use super::private::valid_name;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GeoFormat {
    Trace,
    Plain,
    Json(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeoSource {
    pub id: String,
    pub url: String,
    pub format: GeoFormat,
}

pub const MAX_GEO_BODY: usize = 4096;

pub fn default_sources() -> Vec<GeoSource> {
    vec![
        GeoSource {
            id: "cloudflare".to_owned(),
            url: "https://www.cloudflare.com/cdn-cgi/trace".to_owned(),
            format: GeoFormat::Trace,
        },
        GeoSource {
            id: "ipinfo".to_owned(),
            url: "https://ipinfo.io/country".to_owned(),
            format: GeoFormat::Plain,
        },
        GeoSource {
            id: "countryis".to_owned(),
            url: "https://api.country.is/".to_owned(),
            format: GeoFormat::Json("country".to_owned()),
        },
    ]
}

pub fn parse(format: &GeoFormat, body: &[u8]) -> Option<String> {
    if body.len() > MAX_GEO_BODY {
        return None;
    }
    let text = std::str::from_utf8(body).ok()?;
    let raw = match format {
        GeoFormat::Trace => text.lines().find_map(|line| line.strip_prefix("loc="))?,
        GeoFormat::Plain => text.trim(),
        GeoFormat::Json(field) => {
            let value: serde_json::Value = serde_json::from_str(text).ok()?;
            return accept(value.get(field)?.as_str()?);
        }
    };
    accept(raw)
}

fn accept(raw: &str) -> Option<String> {
    let country = raw.trim().to_ascii_uppercase();
    if country.len() == 2 && country.bytes().all(|byte| byte.is_ascii_uppercase()) {
        Some(country)
    } else {
        None
    }
}

pub fn parse_source_arg(text: &str) -> Option<GeoSource> {
    let (id, rest) = text.split_once('=')?;
    let (format, url) = rest.split_once('=')?;
    if !valid_name(id) || !allowed_url(url) {
        return None;
    }
    let format = if format == "trace" {
        GeoFormat::Trace
    } else if format == "plain" {
        GeoFormat::Plain
    } else if let Some(field) = format.strip_prefix("json:")
        && !field.is_empty()
        && !field.contains('=')
    {
        GeoFormat::Json(field.to_owned())
    } else {
        return None;
    };
    Some(GeoSource {
        id: id.to_owned(),
        url: url.to_owned(),
        format,
    })
}

fn allowed_url(url: &str) -> bool {
    url.starts_with("https://") || (test_mode() && url.starts_with("http://"))
}
