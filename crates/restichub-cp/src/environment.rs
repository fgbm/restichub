//! The environment of the control plane process for the About screen (ADR-0055).
//!
//! The facts come from the running process. A fact that the system does not expose stays absent.
//! No value that identifies the host leaves this module: no host name, no address and no path.

use std::path::Path;
use std::sync::OnceLock;

use restichub_db::Database;
use serde::Serialize;
use utoipa::ToSchema;

use crate::error::cause_chain;

/// The system of the server and the version of its PostgreSQL server.
#[derive(Debug, Serialize, ToSchema)]
pub struct EnvironmentView {
    /// The operating system family, such as `linux` or `windows`.
    pub os: String,
    /// The distribution on Linux, such as `Ubuntu 24.04.1 LTS`. Absent on another system.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os_version: Option<String>,
    /// The release string of the running Linux system, for example `6.8.0-45-generic`. Absent on
    /// another system.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kernel: Option<String>,
    /// The architecture of the process, such as `x86_64`.
    pub arch: String,
    /// True when the process runs in Docker, Podman or Kubernetes. Absent while the process cannot
    /// tell a container from a host.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container: Option<bool>,
    /// The server version of PostgreSQL, such as `16.4`, without the builder suffix. Absent while
    /// the control plane cannot query the database.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
}

/// Reads the environment facts and the PostgreSQL version for the About screen.
///
/// The screen reports a broken installation, so a database that answers no query leaves the
/// version of the server out and the route still answers.
pub async fn read(db: &Database) -> EnvironmentView {
    let system = system();
    let database = match db.server_version().await {
        Ok(version) => Some(version),
        Err(error) => {
            // The error text holds no connection string, so the log does not leak one (ADR-0055).
            tracing::warn!(
                cause = %cause_chain(&error),
                "the About answer holds no database version"
            );
            None
        }
    };
    EnvironmentView {
        os: system.os.to_owned(),
        os_version: system.os_version.clone(),
        kernel: system.kernel.clone(),
        arch: system.arch.to_owned(),
        container: system.container,
        database,
    }
}

struct System {
    os: &'static str,
    os_version: Option<String>,
    kernel: Option<String>,
    arch: &'static str,
    container: Option<bool>,
}

/// The facts do not change while the process runs, so the files are read one time.
fn system() -> &'static System {
    static SYSTEM: OnceLock<System> = OnceLock::new();
    SYSTEM.get_or_init(|| System {
        os: std::env::consts::OS,
        os_version: distribution(),
        kernel: kernel_release(),
        arch: std::env::consts::ARCH,
        container: in_container(),
    })
}

fn distribution() -> Option<String> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let text = std::fs::read_to_string("/etc/os-release").ok()?;
    let mut name = None;
    let mut version = None;
    let mut pretty = None;
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').to_owned();
        if value.is_empty() {
            continue;
        }
        match key {
            "PRETTY_NAME" => pretty = Some(value),
            "NAME" => name = Some(value),
            "VERSION" => version = Some(value),
            _ => {}
        }
    }
    let name = pretty.or_else(|| match (name, version) {
        (Some(name), Some(version)) => Some(format!("{name} {version}")),
        (name, _) => name,
    })?;
    distribution_name(&name)
}

/// The longest distribution name that the answer carries. A line of the file is not bounded.
const MAX_DISTRIBUTION_LEN: usize = 64;

/// The characters that a distribution name may hold.
fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '(' | ')' | '_' | '+' | '-' | '/')
}

/// True for a value that reads as a file path.
fn is_path_shape(value: &str) -> bool {
    value.starts_with('/') || value.contains("//")
}

/// Keeps a distribution name of the expected characters and length, and drops another value.
///
/// A name with another character stays out: the value goes to a support text, so a path or a byte
/// of a terminal escape must not arrive there. The slash stays in, because `Debian GNU/Linux 12
/// (bookworm)` holds one, and a value of the path shape is still dropped.
fn distribution_name(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > MAX_DISTRIBUTION_LEN {
        return None;
    }
    if is_path_shape(value) || !value.chars().all(is_name_char) {
        return None;
    }
    Some(value.to_owned())
}

fn kernel_release() -> Option<String> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let value = std::fs::read_to_string("/proc/sys/kernel/osrelease").ok()?;
    let value = value.trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn in_container() -> Option<bool> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let marker = Path::new("/.dockerenv").exists() || Path::new("/run/.containerenv").exists();
    let cgroup = std::fs::read_to_string("/proc/self/cgroup").unwrap_or_default();
    container_state(marker, &cgroup)
}

/// The container runtimes that name themselves in the cgroup path of the process.
const CONTAINER_RUNTIMES: [&str; 6] = [
    "docker",
    "kubepods",
    "containerd",
    "podman",
    "libpod",
    "lxc",
];

/// Reads the container state from the marker files and from the cgroup path of the process.
///
/// `None` when neither source names a container: a process in a private cgroup namespace shows
/// the path `0::/` on a host and in a container of cgroup v2, so `false` would be a guess.
fn container_state(marker: bool, cgroup: &str) -> Option<bool> {
    let runtime = CONTAINER_RUNTIMES
        .iter()
        .any(|runtime| cgroup.contains(runtime));
    (marker || runtime).then_some(true)
}

#[cfg(test)]
mod tests {
    use super::{container_state, distribution_name};

    #[test]
    fn adr_0055_a_cgroup_path_without_a_runtime_names_no_container() {
        // A process in a private cgroup namespace reports `0::/` on a host and in a container.
        assert_eq!(container_state(false, "0::/\n"), None);
        assert_eq!(container_state(false, "12:devices:/user.slice\n"), None);
        assert_eq!(container_state(false, "0::/docker/8f2c\n"), Some(true));
        assert_eq!(container_state(true, "0::/\n"), Some(true));
    }

    #[test]
    fn adr_0055_a_distribution_name_keeps_the_expected_characters() {
        assert_eq!(
            distribution_name("Ubuntu 24.04.1 LTS").as_deref(),
            Some("Ubuntu 24.04.1 LTS")
        );
        assert_eq!(
            distribution_name("Fedora Linux 40 (Workstation Edition)").as_deref(),
            Some("Fedora Linux 40 (Workstation Edition)")
        );
        assert_eq!(
            distribution_name("Debian GNU/Linux 12 (bookworm)").as_deref(),
            Some("Debian GNU/Linux 12 (bookworm)")
        );
        assert_eq!(distribution_name(""), None);
        assert_eq!(distribution_name("   "), None);
        assert_eq!(distribution_name(&"a".repeat(65)), None);
    }

    #[test]
    fn adr_0055_a_distribution_name_of_the_path_shape_stays_out() {
        assert_eq!(distribution_name("/srv/x"), None);
        assert_eq!(distribution_name("a//b"), None);
        assert_eq!(distribution_name("/etc/os-release"), None);
        // A terminal escape sequence must not reach the support text.
        assert_eq!(distribution_name("Arch \u{1b}[31mLinux"), None);
    }
}
