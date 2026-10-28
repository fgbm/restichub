//! The environment of the control plane process for the About screen (ADR-0055).
//!
//! The facts come from the running process. A fact that the system does not expose stays absent.
//! No value that identifies the host leaves this module: no host name, no address and no path.

use std::path::Path;
use std::sync::OnceLock;

use restichub_db::{Database, DbError};
use serde::Serialize;
use utoipa::ToSchema;

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
    /// True when the process runs in Docker, Podman or Kubernetes.
    pub container: bool,
    /// The server version of PostgreSQL, such as `16.4`, without the builder suffix.
    pub database: String,
}

/// Reads the environment facts and the PostgreSQL version for the About screen.
pub async fn read(db: &Database) -> Result<EnvironmentView, DbError> {
    let system = system();
    Ok(EnvironmentView {
        os: system.os.to_owned(),
        os_version: system.os_version.clone(),
        kernel: system.kernel.clone(),
        arch: system.arch.to_owned(),
        container: system.container,
        database: db.server_version().await?,
    })
}

struct System {
    os: &'static str,
    os_version: Option<String>,
    kernel: Option<String>,
    arch: &'static str,
    container: bool,
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
    pretty.or_else(|| match (name, version) {
        (Some(name), Some(version)) => Some(format!("{name} {version}")),
        (name, _) => name,
    })
}

fn kernel_release() -> Option<String> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let value = std::fs::read_to_string("/proc/sys/kernel/osrelease").ok()?;
    let value = value.trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn in_container() -> bool {
    if !cfg!(target_os = "linux") {
        return false;
    }
    // Docker and Podman write a marker file. Under cgroup v1, and on some Kubernetes nodes, the
    // runtime name appears in the cgroup path of the process instead.
    if Path::new("/.dockerenv").exists() || Path::new("/run/.containerenv").exists() {
        return true;
    }
    let cgroup = std::fs::read_to_string("/proc/self/cgroup").unwrap_or_default();
    [
        "docker",
        "kubepods",
        "containerd",
        "podman",
        "libpod",
        "lxc",
    ]
    .iter()
    .any(|runtime| cgroup.contains(runtime))
}
