pub mod bridge_login;
#[cfg(any(test, feature = "stub"))]
pub mod bridge_stub;
pub mod cloud;
pub mod codec;
pub mod config;
pub mod connect;
mod error;
pub mod features;
pub mod import;
pub mod matrix;
pub mod media;
pub mod models;
pub mod msc4483;
pub mod platform;
pub mod prefs;
pub mod search;
pub mod storage;
pub mod store;
#[cfg(any(test, feature = "stub"))]
pub mod testing;

pub use error::Error;
