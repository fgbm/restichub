//! The stable identifier of the computer (FR-ID-01).
//!
//! The enrollment sends it so the control plane can warn about a second record of one computer.
//! The value is a hint, never a credential, and it never reaches a log: the type hides it from
//! `Debug`.

use std::fmt;

/// The operating system identifier of this computer. The text is not printed by `Debug`.
#[derive(Clone, PartialEq, Eq)]
pub struct MachineId(String);

impl MachineId {
    /// Wraps a raw value. An empty or all-zero value is no identifier.
    pub fn new(raw: &str) -> Option<Self> {
        let value = raw.trim();
        if value.is_empty() || value.chars().all(|c| c == '0' || c == '-') {
            return None;
        }
        Some(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Reads the identifier of this computer, or `None` when the system has none.
    pub fn read() -> Option<Self> {
        read_platform()
    }
}

impl fmt::Debug for MachineId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MachineId(..)")
    }
}

#[cfg(target_os = "linux")]
fn read_platform() -> Option<MachineId> {
    ["/etc/machine-id", "/var/lib/dbus/machine-id"]
        .iter()
        .find_map(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| MachineId::new(&text))
}

#[cfg(windows)]
fn read_platform() -> Option<MachineId> {
    crate::winsec::machine_guid().and_then(|text| MachineId::new(&text))
}

#[cfg(not(any(target_os = "linux", windows)))]
fn read_platform() -> Option<MachineId> {
    None
}

#[cfg(test)]
mod tests {
    use super::MachineId;

    #[test]
    fn ac_id_01_an_empty_identifier_is_none() {
        assert!(MachineId::new("  \n").is_none());
        assert!(MachineId::new("00000000-0000-0000-0000-000000000000").is_none());
    }

    #[test]
    fn ac_id_01_debug_hides_the_identifier() {
        let id = MachineId::new("0123456789abcdef").expect("a value");
        assert_eq!(id.as_str(), "0123456789abcdef");
        assert!(!format!("{id:?}").contains("0123"));
    }
}
