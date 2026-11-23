/// A struct literal whose fields after `default:` take `Default::default()`, as in
/// `with_defaults!(Draft { id, open: true }, default: query, results)`: a struct that cannot derive `Default` names
/// each default field once.
#[macro_export]
macro_rules! with_defaults {
    ($($ty:ident)::+ { $($set:tt)* }, default: $($rest:ident),* $(,)?) => {
        $($ty)::+ { $($rest: Default::default(),)* $($set)* }
    };
}

pub mod bridge_login;
#[cfg(any(test, feature = "stub"))]
pub mod bridge_stub;
pub mod browser_login;
pub mod chat_index;
pub mod cloud;
pub mod codec;
pub mod config;
pub mod connect;
mod download;
pub mod drafts;
mod error;
pub mod features;
mod floe;
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
