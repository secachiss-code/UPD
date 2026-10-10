//! C01, C02, C03: frame schema, strict decoder, polkit actions per operation class.

mod pack_support;

use std::collections::BTreeSet;
use std::io::BufReader;

use cm::controller::actions::{ACTION_APP, ACTION_NET, ACTION_WORKER, action_for};
use cm::controller::codec::{decode, encode, read_frame, request_digest};
use cm::controller::protocol::{
    CONTROL_VERSION, ControlError, MAX_FRAME_BYTES, Op, OpClass, Reply, ReplyData, Request,
};
use pack_support::{Xorshift, mutate};
use serde_json::json;

const SAMPLE: &str = r#"{"v":1,"id":"a1b2c3d4-0001","op":{"type":"worker_start","instance":"browser","generation":7}}"#;

fn op_frame(op: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"v": 1, "id": "a1b2c3d4-0001", "op": op})).unwrap()
}

fn all_ops() -> Vec<Op> {
    let instance = || "browser".to_owned();
    vec![
        Op::InstancePrepare {
            instance: instance(),
        },
        Op::WorkerStart {
            instance: instance(),
            generation: 1,
        },
        Op::WorkerReload {
            instance: instance(),
            generation: 1,
            next_generation: 2,
        },
        Op::WorkerStop {
            instance: instance(),
            generation: 1,
        },
        Op::WorkerStatus {
            instance: instance(),
        },
        Op::NetApply {
            instance: instance(),
            generation: 1,
        },
        Op::NetRevert {
            instance: instance(),
            generation: 1,
        },
        Op::AppLaunch {
            instance: instance(),
            generation: 1,
            program: "/usr/bin/true".to_owned(),
            args: Vec::new(),
            env: Default::default(),
        },
        Op::Reconcile,
    ]
}

#[test]
fn c01_frame_roundtrip_and_classes() {
    let request = decode(SAMPLE.as_bytes()).unwrap();
    assert_eq!(
        request,
        Request {
            v: 1,
            id: "a1b2c3d4-0001".to_owned(),
            op: Op::WorkerStart {
                instance: "browser".to_owned(),
                generation: 7
            },
        }
    );
    assert_eq!(serde_json::to_string(&request).unwrap(), SAMPLE);
    for op in all_ops() {
        let class = match &op {
            Op::WorkerStatus { .. } => OpClass::Status,
            Op::NetApply { .. } | Op::NetRevert { .. } => OpClass::Net,
            Op::AppLaunch { .. } => OpClass::App,
            _ => OpClass::Worker,
        };
        assert_eq!(op.class(), class, "{op:?}");
        assert_eq!(op.instance().is_none(), op == Op::Reconcile, "{op:?}");
    }
}

#[test]
fn c01_frame_schema_has_no_uid_path_or_config() {
    for op in all_ops() {
        let text = serde_json::to_string(&op).unwrap();
        for field in ["\"uid\"", "\"path\"", "\"config\"", "\"root\"", "\"base\""] {
            assert!(!text.contains(field), "{text}");
        }
    }
}

#[test]
fn c01_error_codes_and_replies() {
    let errors = [
        ControlError::BadFrame,
        ControlError::TooLarge,
        ControlError::UnsupportedVersion,
        ControlError::BadId,
        ControlError::BadInstance,
        ControlError::BadArgument,
        ControlError::Denied,
        ControlError::PeerChanged,
        ControlError::GenerationMismatch,
        ControlError::Busy,
        ControlError::Quota,
        ControlError::NotRunning,
        ControlError::Conflict,
        ControlError::InvalidConfig,
        ControlError::Timeout,
        ControlError::Unsupported,
        ControlError::Crashed,
        ControlError::Failed,
    ];
    let codes: BTreeSet<&str> = errors.iter().map(|error| error.code()).collect();
    assert_eq!(codes.len(), 18);
    assert!(
        codes
            .iter()
            .all(|code| code.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'))
    );
    assert_eq!(ControlError::BadFrame.code(), "bad_frame");
    assert_eq!(
        ControlError::GenerationMismatch.code(),
        "generation_mismatch"
    );
    assert_eq!(ControlError::PeerChanged.code(), "peer_changed");
    let ok = Reply::ok("k", Some(ReplyData::Started { generation: 7 }));
    assert_eq!(
        serde_json::to_string(&ok).unwrap(),
        r#"{"v":1,"id":"k","ok":true,"code":"ok","data":{"type":"started","generation":7}}"#
    );
    let error = Reply::error("k", ControlError::Denied);
    assert_eq!(
        serde_json::to_string(&error).unwrap(),
        r#"{"v":1,"id":"k","ok":false,"code":"denied","data":null}"#
    );
    assert_eq!(CONTROL_VERSION, 1);
}

#[test]
fn c02_size_limits() {
    let big = vec![b'x'; MAX_FRAME_BYTES + 1];
    assert_eq!(decode(&big), Err(ControlError::TooLarge));
    let overhead = serde_json::to_vec(&json!({"v":1,"id":"","op":{"type":"reconcile"}}))
        .unwrap()
        .len();
    let exact = serde_json::to_vec(
        &json!({"v":1,"id":"a".repeat(MAX_FRAME_BYTES - overhead),"op":{"type":"reconcile"}}),
    )
    .unwrap();
    assert_eq!(exact.len(), MAX_FRAME_BYTES);
    assert_eq!(decode(&exact), Err(ControlError::BadId));
}

#[test]
fn c02_rejects_everything_but_the_exact_frame() {
    for bad in [&b"not json"[..], b"{}", b"[]", &[0xFF, 0xFF]] {
        assert_eq!(decode(bad), Err(ControlError::BadFrame));
    }
    let start = json!({"type":"worker_start","instance":"browser","generation":7});
    let extra_top =
        serde_json::to_vec(&json!({"v":1,"id":"a1b2c3d4-0001","uid":0,"op":start})).unwrap();
    assert_eq!(decode(&extra_top), Err(ControlError::BadFrame));
    for (key, value) in [
        ("uid", json!(0)),
        ("path", json!("/x")),
        ("index", json!(5)),
    ] {
        let mut op = start.clone();
        op[key] = value;
        assert_eq!(decode(&op_frame(op)), Err(ControlError::BadFrame), "{key}");
    }
    let mut net = json!({"type":"net_apply","instance":"browser","generation":1});
    net["index"] = json!(5);
    assert_eq!(decode(&op_frame(net)), Err(ControlError::BadFrame));
    assert_eq!(
        decode(&op_frame(
            json!({"type":"worker_kill","instance":"browser"})
        )),
        Err(ControlError::BadFrame)
    );
    for version in [0, 2] {
        let frame =
            serde_json::to_vec(&json!({"v":version,"id":"a1b2c3d4-0001","op":start})).unwrap();
        assert_eq!(decode(&frame), Err(ControlError::UnsupportedVersion));
    }
    let long_id = "a".repeat(65);
    for id in ["short", "UPPER-CASE-1", "a b c d e f g h", long_id.as_str()] {
        let frame = serde_json::to_vec(&json!({"v":1,"id":id,"op":start})).unwrap();
        assert_eq!(decode(&frame), Err(ControlError::BadId), "{id}");
    }
    let long_instance = "a".repeat(33);
    for instance in [
        "",
        "-x",
        "A",
        "a/b",
        "../x",
        long_instance.as_str(),
        "x;rm -rf",
        "$(id)",
        "..",
    ] {
        let op = json!({"type":"worker_start","instance":instance,"generation":1});
        assert_eq!(
            decode(&op_frame(op)),
            Err(ControlError::BadInstance),
            "{instance}"
        );
    }
}

#[test]
fn c02_argument_rules() {
    let reload = |next: u64| {
        decode(&op_frame(
            json!({"type":"worker_reload","instance":"browser","generation":5,"next_generation":next}),
        ))
    };
    assert_eq!(reload(5), Err(ControlError::BadArgument));
    assert_eq!(reload(4), Err(ControlError::BadArgument));
    assert!(reload(6).is_ok());
    let launch = |program: &str, args: Vec<String>, env: serde_json::Value| {
        decode(&op_frame(json!({
            "type":"app_launch","instance":"browser","generation":1,
            "program":program,"args":args,"env":env
        })))
    };
    for program in ["firefox", "/usr/../bin/sh", "/usr/bin/a\0b"] {
        assert_eq!(
            launch(program, Vec::new(), json!({})),
            Err(ControlError::BadArgument),
            "{program:?}"
        );
    }
    assert_eq!(
        launch("/bin/true", vec!["a".to_owned(); 65], json!({})),
        Err(ControlError::BadArgument)
    );
    assert!(launch("/bin/true", vec!["a".to_owned(); 64], json!({})).is_ok());
    assert_eq!(
        launch("/bin/true", vec!["a".repeat(4097)], json!({})),
        Err(ControlError::BadArgument)
    );
    assert!(
        launch(
            "/bin/true",
            Vec::new(),
            json!({"WAYLAND_DISPLAY":"wayland-7"})
        )
        .is_ok()
    );
    for (key, value) in [
        ("LD_PRELOAD", "/x.so"),
        ("PATH", "/x"),
        ("HOME", "/x"),
        ("TZ", "UTC"),
    ] {
        assert_eq!(
            launch("/bin/true", Vec::new(), json!({key: value})),
            Err(ControlError::BadArgument),
            "{key}"
        );
    }
    assert_eq!(
        launch(
            "/bin/true",
            Vec::new(),
            json!({"DISPLAY": "x".repeat(4097)})
        ),
        Err(ControlError::BadArgument)
    );
    // The frame without `env` is the same request as before the field existed.
    let plain = op_frame(
        json!({"type":"app_launch","instance":"browser","generation":1,"program":"/bin/true","args":[]}),
    );
    assert!(decode(&plain).is_ok());
}

#[test]
fn c02_read_frame_and_encode() {
    let mut stream = vec![b'x'; 70_000];
    stream.push(b'\n');
    stream.extend_from_slice(SAMPLE.as_bytes());
    stream.push(b'\n');
    let mut reader = BufReader::new(&stream[..]);
    assert_eq!(read_frame(&mut reader), Err(ControlError::TooLarge));
    assert_eq!(
        read_frame(&mut reader).unwrap().as_deref(),
        Some(SAMPLE.as_bytes())
    );
    assert_eq!(read_frame(&mut BufReader::new(&b""[..])), Ok(None));
    assert_eq!(
        read_frame(&mut BufReader::new(&b"abc"[..])),
        Err(ControlError::BadFrame)
    );
    let megabyte = vec![b'x'; 1 << 20];
    assert_eq!(
        read_frame(&mut BufReader::new(&megabyte[..])),
        Err(ControlError::TooLarge)
    );
    let encoded = encode(&Reply::ok("a\nb", None));
    assert_eq!(encoded.iter().filter(|byte| **byte == b'\n').count(), 1);
    assert_eq!(encoded.last(), Some(&b'\n'));
}

#[test]
fn c02_mutation_never_panics() {
    let mut rng = Xorshift(1);
    let allowed = [
        ControlError::BadFrame,
        ControlError::TooLarge,
        ControlError::UnsupportedVersion,
        ControlError::BadId,
        ControlError::BadInstance,
        ControlError::BadArgument,
    ];
    let samples: Vec<Vec<u8>> = all_ops()
        .into_iter()
        .map(|op| {
            serde_json::to_vec(&Request {
                v: 1,
                id: "a1b2c3d4-0001".to_owned(),
                op,
            })
            .unwrap()
        })
        .collect();
    let mut rejected = 0u32;
    for round in 0..100_000 {
        let mut bytes = samples[round % samples.len()].clone();
        for _ in 0..=(rng.next() % 3) {
            bytes = mutate(&bytes, &mut rng);
        }
        if let Err(error) = decode(&bytes) {
            assert!(allowed.contains(&error), "{error:?}");
            rejected += 1;
        }
    }
    assert!(rejected > 50_000, "{rejected}");
}

#[test]
fn c02_digest_depends_on_op_only() {
    let request = |id: &str, generation: u64| Request {
        v: 1,
        id: id.to_owned(),
        op: Op::WorkerStart {
            instance: "browser".to_owned(),
            generation,
        },
    };
    let digest = request_digest(&request("aaaaaaaa", 7));
    assert_eq!(digest.len(), 64);
    assert_eq!(digest, request_digest(&request("bbbbbbbb", 7)));
    assert_ne!(digest, request_digest(&request("aaaaaaaa", 8)));
}

#[test]
fn c03_actions_per_class_and_policy() {
    assert_eq!(action_for(OpClass::Status), "io.github.cm.status");
    assert_eq!(action_for(OpClass::Worker), "io.github.cm.worker");
    assert_eq!(action_for(OpClass::Net), "io.github.cm.net");
    assert_eq!(action_for(OpClass::App), "io.github.cm.app");
    assert_eq!(
        [ACTION_WORKER, ACTION_NET, ACTION_APP],
        [
            "io.github.cm.worker",
            "io.github.cm.net",
            "io.github.cm.app"
        ]
    );
    // The policy generator lives in the binary; its table is checked in the source.
    let main = include_str!("../src/main.rs");
    let flat: String = main.split_whitespace().collect::<Vec<_>>().join(" ");
    for (action, active) in [
        ("ACTION_WORKER", "yes"),
        ("ACTION_NET", "auth_admin_keep"),
        ("ACTION_APP", "yes"),
    ] {
        let needle = format!("cm::controller::actions::{action}, \"{active}\", msg(");
        let at = flat.find(&needle).unwrap_or_else(|| panic!("{needle}"));
        let entry = &flat[at..flat[at..].find("), ),").map_or(flat.len(), |end| at + end)];
        for language in ["\"ru\"", "\"de\"", "\"it\"", "\"zh\"", "\"ar\""] {
            assert!(entry.contains(language), "{action} {language}");
        }
    }
}

#[test]
fn c04_one_pkcheck_implementation() {
    let mut with_pkcheck = Vec::new();
    for dir in ["src/helper", "src/controller"] {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|ext| ext == "rs")
                && std::fs::read_to_string(&path)
                    .unwrap()
                    .contains("Command::new(\"pkcheck\")")
            {
                with_pkcheck.push(path.display().to_string());
            }
        }
    }
    assert_eq!(with_pkcheck, ["src/controller/actions.rs"]);
}
