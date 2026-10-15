//! Lexical rules for the target of a restore (FR-RS-03, ADR-0035).
//!
//! The control plane uses them to refuse early, and the agent uses them as the first step of its own
//! authoritative check. They never touch a file system: the agent resolves symbolic links before it
//! calls [`is_within`].

/// Which separators and case rules a path follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathFlavor {
    Posix,
    Windows,
}

impl PathFlavor {
    /// The flavor of an agent, from the `os` that its `AgentHello` reports.
    #[must_use]
    pub fn from_os(os: &str) -> Self {
        if os.eq_ignore_ascii_case("windows") {
            Self::Windows
        } else {
            Self::Posix
        }
    }

    /// The flavor of the operating system that runs this code.
    #[must_use]
    pub const fn host() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else {
            Self::Posix
        }
    }
}

/// The components of an absolute path, or `None` when the path is not absolute in the flavor, holds
/// a `.` or `..` component, or uses a form that this check does not model.
///
/// A POSIX path keeps the case of each component. A Windows path starts with the drive and lower-cases
/// every component, because the file system compares names without case. It refuses a UNC path, a
/// component with a colon (an alternate data stream) and a component that ends in a dot or a space,
/// because Windows drops that tail and so names another file.
#[must_use]
pub fn components(flavor: PathFlavor, path: &str) -> Option<Vec<String>> {
    match flavor {
        PathFlavor::Posix => posix_components(path),
        PathFlavor::Windows => windows_components(path),
    }
}

fn posix_components(path: &str) -> Option<Vec<String>> {
    let rest = path.strip_prefix('/')?;
    let mut parts = Vec::new();
    for part in rest.split('/').filter(|part| !part.is_empty()) {
        if part == "." || part == ".." {
            return None;
        }
        parts.push(part.to_owned());
    }
    Some(parts)
}

fn windows_components(path: &str) -> Option<Vec<String>> {
    // `canonicalize` returns the verbatim form, so the check reads it like the plain drive path.
    let path = path.strip_prefix(r"\\?\").unwrap_or(path);
    let bytes = path.as_bytes();
    let drive_form = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/');
    if !drive_form {
        return None;
    }
    let mut parts = vec![format!("{}:", path[..1].to_lowercase())];
    for part in path[3..].split(['\\', '/']).filter(|part| !part.is_empty()) {
        if part == "." || part == ".." || part.contains(':') {
            return None;
        }
        if part.ends_with('.') || part.ends_with(' ') {
            return None;
        }
        parts.push(part.to_lowercase());
    }
    Some(parts)
}

/// True when `path` is `root` or lies below it. A path that is not absolute in the flavor gives
/// false.
#[must_use]
pub fn is_within(flavor: PathFlavor, path: &str, root: &str) -> bool {
    match (components(flavor, path), components(flavor, root)) {
        (Some(path), Some(root)) => path.starts_with(&root),
        _ => false,
    }
}

/// True when `path` and `other` are the same path, or one lies below the other.
#[must_use]
pub fn overlaps(flavor: PathFlavor, path: &str, other: &str) -> bool {
    is_within(flavor, path, other) || is_within(flavor, other, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_rs_03_a_posix_path_is_inside_its_root_by_component() {
        let posix = PathFlavor::Posix;
        assert!(is_within(
            posix,
            "/var/lib/restichub-restore",
            "/var/lib/restichub-restore"
        ));
        assert!(is_within(
            posix,
            "/var/lib/restichub-restore/a/b",
            "/var/lib/restichub-restore"
        ));
        assert!(is_within(
            posix,
            "/var/lib/restichub-restore//a/",
            "/var/lib/restichub-restore/"
        ));
        assert!(!is_within(
            posix,
            "/var/lib/restichub-restore2",
            "/var/lib/restichub-restore"
        ));
        assert!(!is_within(posix, "/var/lib", "/var/lib/restichub-restore"));
        assert!(is_within(posix, "/anything", "/"));
        assert!(!is_within(posix, "relative/path", "/"));
        assert!(!is_within(posix, "/a/../etc", "/a"));
        assert!(!is_within(posix, "C:\\restore", "/"));
    }

    #[test]
    fn fr_rs_03_a_windows_path_compares_without_case_and_refuses_odd_forms() {
        let windows = PathFlavor::Windows;
        let root = r"C:\ProgramData\ResticHub\Restore";
        assert!(is_within(
            windows,
            r"c:\programdata\resticHUB\restore\x\y",
            root
        ));
        assert!(is_within(windows, "C:/ProgramData/ResticHub/Restore", root));
        assert!(is_within(
            windows,
            r"\\?\C:\ProgramData\ResticHub\Restore\x",
            root
        ));
        assert!(!is_within(
            windows,
            r"C:\ProgramData\ResticHub\Restore2",
            root
        ));
        assert!(!is_within(
            windows,
            r"D:\ProgramData\ResticHub\Restore",
            root
        ));
        assert!(!is_within(
            windows,
            r"C:\ProgramData\ResticHub\Restore\..\Agent",
            root
        ));
        assert!(!is_within(
            windows,
            r"C:\ProgramData\ResticHub\Restore\x:stream",
            root
        ));
        assert!(!is_within(
            windows,
            r"C:\ProgramData\ResticHub\Restore\x.",
            root
        ));
        assert!(!is_within(windows, r"\\server\share\Restore", root));
        assert!(!is_within(windows, "/ProgramData/ResticHub/Restore", root));
        assert!(is_within(windows, r"C:\", "C:/"));
    }

    #[test]
    fn fr_rs_03_overlap_covers_both_directions() {
        let posix = PathFlavor::Posix;
        assert!(overlaps(posix, "/etc", "/etc/restichub-agent"));
        assert!(overlaps(
            posix,
            "/etc/restichub-agent/x",
            "/etc/restichub-agent"
        ));
        assert!(overlaps(posix, "/", "/etc/restichub-agent"));
        assert!(!overlaps(posix, "/etc/other", "/etc/restichub-agent"));
    }

    #[test]
    fn fr_rs_03_the_flavor_follows_the_reported_os() {
        assert_eq!(PathFlavor::from_os("windows"), PathFlavor::Windows);
        assert_eq!(PathFlavor::from_os("Windows"), PathFlavor::Windows);
        assert_eq!(PathFlavor::from_os("linux"), PathFlavor::Posix);
        assert_eq!(PathFlavor::from_os(""), PathFlavor::Posix);
    }
}
