//! Fixtures for the tests of all crates: empty temporary folders and demo stores.
use crate::config::CentralConfig;
use crate::store::{BeeperStore, SharedStore, StoreLock};
use std::path::PathBuf;

/// An empty folder for the test `name`. The path holds a hash of this checkout, so that tests in several checkouts
/// do not delete the folders of each other, and the next run of the test deletes the folder of the last run.
pub fn temp_dir(name: &str) -> PathBuf {
    let checkout = env!("CARGO_MANIFEST_DIR").bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b.into()));
    let dir = std::env::temp_dir().join(format!("beeper-rs-test-{checkout:x}")).join(name);
    if let Err(e) = std::fs::remove_dir_all(&dir) {
        assert_eq!(e.kind(), std::io::ErrorKind::NotFound, "delete the test folder {dir:?}: {e}");
    }
    std::fs::create_dir_all(&dir).expect("create the test folder");
    dir
}

/// The config of a demo store in `temp_dir(name)`.
pub fn demo_config(name: &str) -> CentralConfig {
    CentralConfig { demo: true, ..CentralConfig::new(temp_dir(name), "127.0.0.1".into(), 23390, None) }
}

/// A demo store with the seed chats in `temp_dir(name)`.
pub fn demo_store(name: &str) -> SharedStore {
    StoreLock::shared(BeeperStore::from_config(demo_config(name)))
}
