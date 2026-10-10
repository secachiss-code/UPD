//! BI.M2: каталог личностей — права 0700/0600, атомарная запись, список, удаление, симлинки (E07).

#[path = "bi_support/mod.rs"]
mod support;

use cm::common::contract_fixtures::TempDirGuard;
use cm::identity::model::{BrowserIdentityProfile, Strategy};
use cm::identity::store::{IdentityStore, StoreError, root};
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use support::{Checks, chrome155, de_env, env_guard, profile, set_env};

fn mode(path: &Path) -> u32 {
    fs::metadata(path).expect("metadata").permissions().mode() & 0o777
}

fn de_profile(id: &str) -> BrowserIdentityProfile {
    profile(
        id,
        Strategy::Local,
        Path::new("/usr/bin/google-chrome-stable"),
        chrome155(),
        Some(de_env()),
    )
}

#[test]
fn e07_store_permissions_atomic_write_and_removal() {
    let mut c = Checks::new("E07");
    let guard = TempDirGuard::new("cm-bi-e07").expect("tmp");
    let base: PathBuf = guard.path().to_path_buf();

    {
        let _env = env_guard();
        set_env(
            "CM_IDENTITY_ROOT",
            base.join("identities").to_str().expect("utf-8"),
        );
        c.eq(
            "root() читает CM_IDENTITY_ROOT",
            base.join("identities"),
            root(),
        );
    }

    let store_root = base.join("identities");
    let store = IdentityStore::open(store_root.clone()).expect("open");
    let work = de_profile("work");
    c.eq("create(work)", Ok(()), store.create(&work));
    let dir = store_root.join("work");
    c.eq("каталог личности 0700", 0o700, mode(&dir));
    c.eq(
        "identity.json 0600",
        0o600,
        mode(&dir.join("identity.json")),
    );
    c.eq("profile/ 0700", 0o700, mode(&dir.join("profile")));
    c.eq(
        "create(work) повторно",
        Err(StoreError::Exists),
        store.create(&work),
    );
    c.eq("load(none)", Err(StoreError::NotFound), store.load("none"));

    fs::write(
        dir.join("identity.json.tmp"),
        "мусор, оставшийся после сбоя",
    )
    .unwrap();
    c.eq(
        "load читает прежний identity.json",
        Ok(work.clone()),
        store.load("work"),
    );

    let list_root = base.join("listing");
    let listing = IdentityStore::open(list_root.clone()).expect("open listing");
    listing.create(&de_profile("b")).expect("create b");
    listing.create(&de_profile("a")).expect("create a");
    fs::create_dir(list_root.join("c")).expect("каталог без identity.json");
    c.eq(
        "list() по алфавиту, без каталога без json",
        Ok(vec!["a".to_string(), "b".to_string()]),
        listing.list(),
    );

    fs::write(dir.join("state.json"), "{}").unwrap();
    c.eq("remove(work, false)", Ok(()), store.remove("work", false));
    c.holds("identity.json удалён", !dir.join("identity.json").exists());
    c.holds("state.json удалён", !dir.join("state.json").exists());
    c.holds("profile/ на месте", dir.join("profile").is_dir());
    c.eq("remove(work, true)", Ok(()), store.remove("work", true));
    c.holds("каталога личности нет после purge", !dir.exists());

    let real = base.join("real-root");
    fs::create_dir(&real).expect("real root");
    let link = base.join("link-root");
    symlink(&real, &link).expect("symlink");
    c.eq(
        "root — симлинк → Unsafe",
        Err(StoreError::Unsafe),
        IdentityStore::open(link).map(|_| ()),
    );
    c.finish();
}
