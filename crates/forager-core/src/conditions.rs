//! Whether now is a good time for background updates: not over a metered
//! connection (NetworkManager) and not on battery (UPower). Both are read
//! from the system bus; when a service is missing the answer is `None` and
//! nothing is held back.

use zbus::blocking::{Connection, Proxy};

/// What `update` was asked to avoid.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Avoid {
    pub metered: bool,
    pub battery: bool,
}

/// Why updates should wait, if one of the conditions in `avoid` holds now.
pub fn blocked(avoid: Avoid) -> Option<&'static str> {
    if avoid.metered && metered() == Some(true) {
        Some("the connection is metered")
    } else if avoid.battery && on_battery() == Some(true) {
        Some("running on battery")
    } else {
        None
    }
}

/// Whether NetworkManager considers the primary connection metered.
pub fn metered() -> Option<bool> {
    let value: u32 = property(
        "org.freedesktop.NetworkManager",
        "/org/freedesktop/NetworkManager",
        "org.freedesktop.NetworkManager",
        "Metered",
    )?;
    Some(is_metered(value))
}

/// `NMMetered`: 1 yes, 2 no, 3 guessed yes, 4 guessed no, 0 unknown.
/// A guess counts: NetworkManager guesses yes for phone hotspots.
fn is_metered(value: u32) -> bool {
    matches!(value, 1 | 3)
}

/// Whether UPower says the system runs on battery.
pub fn on_battery() -> Option<bool> {
    property(
        "org.freedesktop.UPower",
        "/org/freedesktop/UPower",
        "org.freedesktop.UPower",
        "OnBattery",
    )
}

fn property<T>(service: &str, path: &str, interface: &str, name: &str) -> Option<T>
where
    T: TryFrom<zbus::zvariant::OwnedValue>,
    T::Error: Into<zbus::Error>,
{
    let conn = Connection::system().ok()?;
    let proxy = Proxy::new(&conn, service, path, interface).ok()?;
    proxy.get_property(name).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guessed_metered_counts_as_metered() {
        assert!(is_metered(1));
        assert!(is_metered(3));
        assert!(!is_metered(0));
        assert!(!is_metered(2));
        assert!(!is_metered(4));
    }

    #[test]
    fn nothing_avoided_is_never_blocked() {
        assert_eq!(blocked(Avoid::default()), None);
    }
}
