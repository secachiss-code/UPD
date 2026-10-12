//! R01, R02: geo parse and probe result file.

#[path = "ab_support/mod.rs"]
mod ab_support;

use std::fs;
use std::os::unix::fs::PermissionsExt;

use cm::env::collect::{CollectError, MAX_RESULT_BYTES, read_result};
use cm::env::geo::Observation;
use cm::env::sources::{self, GeoFormat, default_sources, parse, parse_source_arg};

use ab_support::with_cm_conf;

#[test]
fn r01_r02_region_sources() {
    with_cm_conf(|| {
        let trace = include_str!("fixtures/ab/geo/trace.txt");
        assert_eq!(
            parse(&GeoFormat::Trace, trace.as_bytes()),
            Some("DE".into())
        );
        assert_eq!(parse(&GeoFormat::Plain, b"de\n"), Some("DE".into()));
        assert!(parse(&GeoFormat::Plain, b"Germany").is_none());
        assert_eq!(
            parse(
                &GeoFormat::Json("country".into()),
                br#"{"ip":"203.0.113.7","country":"NL"}"#
            ),
            Some("NL".into())
        );
        assert!(parse(&GeoFormat::Json("country".into()), br#"{"country":1}"#).is_none());
        assert!(parse(&GeoFormat::Plain, &[0xff; 8]).is_none());
        assert!(parse(&GeoFormat::Plain, vec![b'a'; 4097].as_slice()).is_none());
        for src in default_sources() {
            assert!(src.url.starts_with("https://"));
        }
        assert!(parse_source_arg("a=plain=https://x.test/c").is_some());
        assert!(parse_source_arg("a=xml=https://x").is_none());
        assert!(parse_source_arg("A=plain=https://x").is_none());
        let obs = vec![
            Observation {
                source: "a".into(),
                country: "DE".into(),
                at_unix_ms: 1,
            },
            Observation {
                source: "b".into(),
                country: "DE".into(),
                at_unix_ms: 2,
            },
        ];
        let dir = cm::common::contract_fixtures::TempDirGuard::new("cm-ab-r02").unwrap();
        let path = dir.path().join("out.json");
        fs::write(&path, serde_json::to_vec(&obs).unwrap()).unwrap();
        assert_eq!(read_result(&path).unwrap().len(), 2);
        assert!(read_result(std::path::Path::new("/not-json")).is_err());
        fs::write(&path, vec![b' '; MAX_RESULT_BYTES + 1]).unwrap();
        assert_eq!(read_result(&path), Err(CollectError::Malformed));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    });
}
