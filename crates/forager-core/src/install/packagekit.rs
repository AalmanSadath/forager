//! Installs and removes system packages through the PackageKit daemon over
//! D-Bus. PackageKit asks polkit for permission itself, works the same on
//! every distribution it has a backend for, and reports progress.

use std::collections::HashMap;

use zbus::MatchRule;
use zbus::blocking::fdo::DBusProxy;
use zbus::blocking::{Connection, MessageIterator, Proxy};
use zbus::message::Type;
use zbus::zvariant::{DynamicType, OwnedObjectPath, OwnedValue};

use crate::error::{Error, Result};

const SERVICE: &str = "org.freedesktop.PackageKit";
const PATH: &str = "/org/freedesktop/PackageKit";
const TRANSACTION: &str = "org.freedesktop.PackageKit.Transaction";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

/// `PK_FILTER_ENUM_INSTALLED` as a filter bit.
const FILTER_INSTALLED: u64 = 1 << 2;
/// `PK_TRANSACTION_FLAG_ENUM_ALLOW_REINSTALL` as a flag bit, so a missing
/// install of the same version, or a format switch, can be repaired.
const FLAG_ALLOW_REINSTALL: u64 = 1 << 4;
const EXIT_SUCCESS: u32 = 1;
/// `PK_ERROR_ENUM_NOT_AUTHORIZED`: polkit refused, e.g. with no agent to ask.
pub const ERROR_NOT_AUTHORIZED: u32 = 48;

/// Whether the PackageKit service is running or can be started.
pub fn available() -> bool {
    let Ok(conn) = Connection::system() else {
        return false;
    };
    let Ok(dbus) = DBusProxy::new(&conn) else {
        return false;
    };
    let Ok(name) = SERVICE.try_into() else {
        return false;
    };
    dbus.name_has_owner(name).unwrap_or(false)
        || dbus
            .list_activatable_names()
            .is_ok_and(|names| names.iter().any(|n| n.as_str() == SERVICE))
}

/// Installs a local package file. `progress` gets percentages as they come.
pub fn install_file(path: &std::path::Path, progress: &mut dyn FnMut(u32)) -> Result<()> {
    let path = path.to_string_lossy().into_owned();
    Transaction::begin()?.run("InstallFiles", &(FLAG_ALLOW_REINSTALL, vec![path]), progress)?;
    Ok(())
}

/// Removes an installed package by name.
pub fn remove(name: &str, progress: &mut dyn FnMut(u32)) -> Result<()> {
    let ids = Transaction::begin()?.run("Resolve", &(FILTER_INSTALLED, vec![name]), &mut |_| {})?;
    if ids.is_empty() {
        return Err(Error::PackageKit {
            code: 0,
            details: format!("{name} is not installed"),
        });
    }
    // Do not take dependent packages along, and leave dependencies in place.
    Transaction::begin()?.run("RemovePackages", &(0u64, ids, false, false), progress)?;
    Ok(())
}

struct Transaction {
    proxy: Proxy<'static>,
    signals: MessageIterator,
}

impl Transaction {
    fn begin() -> Result<Transaction> {
        let conn = Connection::system()?;
        let daemon = Proxy::new(&conn, SERVICE, PATH, SERVICE)?;
        let path: OwnedObjectPath = daemon.call("CreateTransaction", &())?;
        // Subscribe before starting the transaction so no signal is missed.
        let rule = MatchRule::builder()
            .msg_type(Type::Signal)
            .sender(SERVICE)?
            .path(path.clone())?
            .build();
        let signals = MessageIterator::for_match_rule(rule, &conn, Some(256))?;
        let proxy = Proxy::new_owned(conn, SERVICE.to_string(), path, TRANSACTION.to_string())?;
        // Lets polkit ask the user for a password.
        proxy.call_method("SetHints", &(vec!["interactive=true"],))?;
        Ok(Transaction { proxy, signals })
    }

    /// Calls `method` and waits for the transaction to finish, returning the
    /// package ids it reported.
    fn run<B>(mut self, method: &str, args: &B, progress: &mut dyn FnMut(u32)) -> Result<Vec<String>>
    where
        B: serde::Serialize + DynamicType,
    {
        self.proxy.call_method(method, args)?;
        let mut packages = Vec::new();
        let mut error = None;
        for message in &mut self.signals {
            let message = message?;
            let header = message.header();
            let (Some(interface), Some(member)) = (header.interface(), header.member()) else {
                continue;
            };
            let body = message.body();
            match (interface.as_str(), member.as_str()) {
                (TRANSACTION, "Package") => {
                    let (_info, id, _summary): (u32, String, String) = body.deserialize()?;
                    packages.push(id);
                }
                (TRANSACTION, "ErrorCode") => {
                    let (code, details): (u32, String) = body.deserialize()?;
                    error = Some(Error::PackageKit { code, details });
                }
                (TRANSACTION, "Finished") => {
                    let (exit, _runtime): (u32, u32) = body.deserialize()?;
                    return match (exit, error) {
                        (EXIT_SUCCESS, _) => Ok(packages),
                        (_, Some(error)) => Err(error),
                        (exit, None) => Err(Error::PackageKit {
                            code: 0,
                            details: format!("transaction ended with exit code {exit}"),
                        }),
                    };
                }
                (PROPERTIES, "PropertiesChanged") => {
                    let (_iface, changed, _invalidated): (String, HashMap<String, OwnedValue>, Vec<String>) =
                        body.deserialize()?;
                    // 101 means "unknown".
                    if let Some(percent) = changed.get("Percentage").and_then(|v| u32::try_from(v).ok())
                        && percent <= 100
                    {
                        progress(percent);
                    }
                }
                _ => {}
            }
        }
        Err(Error::PackageKit {
            code: 0,
            details: "PackageKit closed the connection".into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Talks to the real daemon; read-only. Skips when there is no system bus
    /// or PackageKit, as in CI containers.
    #[test]
    fn resolves_an_installed_package() {
        if !available() {
            return;
        }
        let Ok(transaction) = Transaction::begin() else {
            return;
        };
        let shell = if std::path::Path::new("/usr/bin/bash").exists() {
            "bash"
        } else {
            "coreutils"
        };
        let ids = transaction
            .run("Resolve", &(FILTER_INSTALLED, vec![shell]), &mut |_| {})
            .unwrap();
        assert!(ids.iter().any(|id| id.starts_with(&format!("{shell};"))), "{ids:?}");
    }
}
