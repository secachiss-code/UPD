//! I17-D.T01.a: fixtures cover host mode, app sharing, condition, and every axis.
use serde_json::Value;

#[test]
fn state_fixtures_cover_every_axis() {
    let text = include_str!("fixtures/i17/states.json");
    let states: Vec<Value> = serde_json::from_str(text).unwrap();
    let mut hosts = Vec::new();
    let mut apps = Vec::new();
    let mut conditions = Vec::new();
    let mut axes = std::collections::BTreeMap::<String, Vec<String>>::new();
    for state in &states {
        hosts.push(state["host"].as_str().unwrap().to_owned());
        apps.push(state["apps"].as_str().unwrap().to_owned());
        conditions.push(state["condition"].as_str().unwrap().to_owned());
        let object = state["axes"].as_object().unwrap();
        assert_eq!(object.len(), 4, "{}", state["name"]);
        for (axis, value) in object {
            axes.entry(axis.clone())
                .or_default()
                .push(value.as_str().unwrap().to_owned());
        }
    }
    for host in ["off", "proxy", "tunnel"] {
        assert!(hosts.iter().any(|item| item == host), "{host}");
    }
    for mode in ["separate", "shared"] {
        assert!(apps.iter().any(|item| item == mode), "{mode}");
    }
    for condition in ["blocked", "degraded", "unknown"] {
        assert!(
            conditions.iter().any(|item| item == condition),
            "{condition}"
        );
    }
    for axis in ["net", "region", "state", "app"] {
        let values = axes.get(axis).unwrap();
        for value in ["unknown", "partial", "verified", "blocked", "error"] {
            assert!(values.iter().any(|item| item == value), "{axis}:{value}");
        }
    }
}
