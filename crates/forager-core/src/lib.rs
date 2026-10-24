//! Core library for Forager: tracks forge releases and installs them through
//! the system package manager.

pub mod envfile;
pub mod error;
pub mod history;
pub mod http;
pub mod install;
pub mod matcher;
pub mod model;
pub mod platform;
pub mod signature;
pub mod source;
pub mod store;
pub mod timer;
pub mod update;
pub mod verify;
pub mod version;

pub use error::{Error, Result};
