//! Core library for Forager: tracks forge releases and installs them through
//! the system package manager.

pub mod error;
pub mod install;
pub mod matcher;
pub mod model;
pub mod platform;
pub mod source;
pub mod store;
pub mod verify;
pub mod version;

pub use error::{Error, Result};
