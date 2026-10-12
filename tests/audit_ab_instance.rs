//! T01, T02: process scan, holders, elsewhere.

#[path = "ab_support/mod.rs"]
mod ab_support;

use std::fs;
use std::os::unix::fs::symlink;

use cm::app::instance::{elsewhere, holders, netns_id, scan};
use cm::common::contract_fixtures::TempDirGuard;

use ab_support::with_cm_conf;

#[test]
fn t01_t02_instance_scan() {
    with_cm_conf(|| {
        assert_eq!(netns_id("net:[4026531840]"), Some(4026531840));
        assert_eq!(netns_id("net:[x]"), None);
        assert_eq!(netns_id("mnt:[1]"), None);
        let dir = TempDirGuard::new("cm-ab-t01").unwrap();
        let root = dir.path().join("proc");
        fs::create_dir_all(&root).unwrap();
        let bin = root.join("bin-app");
        fs::write(&bin, b"").unwrap();
        for (pid, ns, exe) in [
            (10u32, Some(4026531840u64), true),
            (11, Some(4026532000), true),
            (12, Some(4026532000), true),
            (13, None, false),
        ] {
            let pdir = root.join(pid.to_string());
            fs::create_dir_all(&pdir).unwrap();
            if exe {
                symlink(&bin, pdir.join("exe")).unwrap();
            }
            if let Some(id) = ns {
                fs::create_dir_all(pdir.join("ns")).unwrap();
                symlink(format!("net:[{id}]"), pdir.join("ns/net")).unwrap();
            }
        }
        fs::create_dir_all(root.join("net")).unwrap();
        fs::write(root.join("self"), b"").unwrap();
        let procs = scan(&root);
        assert_eq!(procs.len(), 4);
        assert!(procs[3].netns.is_none());
        assert_eq!(holders(&procs, 4026532000, 1000), vec![11, 12]);
        assert!(holders(&procs, 4026533000, 1000).is_empty());
        assert!(holders(&procs, 4026532000, 1001).is_empty());
        assert_eq!(
            elsewhere(&procs, 1000, std::path::Path::new(&bin), 4026532000),
            vec![10]
        );
        assert_eq!(
            elsewhere(&procs, 1000, std::path::Path::new(&bin), 4026531840),
            vec![11, 12]
        );
        assert!(
            elsewhere(&procs, 1000, std::path::Path::new(&bin), 4026532000).is_empty() == false
        ); // other exe
        assert!(elsewhere(&procs, 1000, std::path::Path::new("/no/such"), 4026532000).is_empty());
    });
}
