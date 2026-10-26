//! Core library for Forager: tracks forge releases and installs them through
//! the system package manager.

pub mod appimage;
pub mod cleanup;
pub mod conditions;
pub mod desktop;
pub mod doctor;
pub mod envfile;
pub mod error;
pub mod history;
pub mod host;
pub mod http;
pub mod icons;
pub mod install;
pub mod matcher;
pub mod model;
pub mod notice;
pub mod platform;
pub mod share;
pub mod signature;
pub mod source;
pub mod store;
pub mod timer;
pub mod update;
pub mod verify;
pub mod version;
pub mod zsync;

pub use error::{Error, Result};
