//! The service setup after `register` and in `install-service`, and the `status` report
//! (FR-IN-07, FR-IN-08, FR-AG-01).
//!
//! The logic runs against the [`ServiceManager`] trait, so a test replaces systemd and the Windows
//! service control manager.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use time::OffsetDateTime;

use crate::connection::{ConnectionStatus, Wait, wait_for_connection};
use crate::identity::StoredIdentity;
use crate::service::{ServiceManager, ServiceState};

/// The command that `install-service` and `status` name when the host has no identity.
const REGISTER_HINT: &str =
    "restichub-agent register --url <control plane URL> --token <enrollment token>";

/// How long the setup waits for the first connection.
#[derive(Debug, Clone, Copy)]
pub struct WaitOptions {
    pub timeout: Duration,
    pub poll: Duration,
}

impl Default for WaitOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(60),
            poll: Duration::from_millis(500),
        }
    }
}

/// What the setup found after it started the service.
#[derive(Debug)]
pub struct SetupReport {
    pub agent_id: String,
    pub tenant_id: String,
    pub cp_url: String,
    pub service: ServiceState,
    pub wait: Wait,
}

impl SetupReport {
    #[must_use]
    pub fn connected(&self) -> bool {
        self.service.running && matches!(self.wait, Wait::Connected(_))
    }
}

/// Loads the identity and refuses a host without a usable one.
///
/// # Errors
///
/// Returns an error that names the register command when the identity is absent or damaged.
pub fn require_identity(dir: &Path) -> Result<StoredIdentity> {
    let identity = StoredIdentity::load(dir)
        .and_then(|identity| identity.certificate_expiry().map(|_| identity))
        .with_context(|| {
            format!(
                "no valid agent identity in {}; register first: {REGISTER_HINT}",
                dir.display()
            )
        })?;
    Ok(identity)
}

/// Installs and starts the service, then waits for the agent to report a connection.
///
/// # Errors
///
/// Returns an error when the identity is unusable or the service cannot be installed or started.
/// A service that runs but does not connect is a report, not an error.
pub fn install_and_wait(
    manager: &dyn ServiceManager,
    dir: &Path,
    restic: &Path,
    wait: WaitOptions,
) -> Result<SetupReport> {
    let identity = require_identity(dir)?;
    let since = OffsetDateTime::now_utc();
    manager.install_and_start(restic)?;
    let waited = wait_for_connection(dir, since, wait.timeout, wait.poll);
    let service = manager.state()?;
    Ok(SetupReport {
        agent_id: identity.agent_id,
        tenant_id: identity.tenant_id,
        cp_url: identity.cp_url,
        service,
        wait: waited,
    })
}

/// The final summary of `register` and `install-service`.
#[must_use]
pub fn render_report(report: &SetupReport, hostname: Option<&str>) -> String {
    let who = match hostname {
        Some(name) => format!("{name} (agent id {})", report.agent_id),
        None => format!("agent id {}", report.agent_id),
    };
    let mut lines = vec![
        format!("registered as {who}"),
        format!("organization:  {}", report.tenant_id),
    ];
    if !report.service.installed {
        lines.push("service:       not installed".to_owned());
        lines.push("next:          restichub-agent install-service".to_owned());
    } else if !report.service.running {
        lines.push("service:       installed, not running".to_owned());
        lines.push(stopped_hint());
    } else {
        lines.push("service:       installed and running".to_owned());
        match &report.wait {
            Wait::Connected(_) => {
                lines.push(format!("connection:    connected to {}", report.cp_url));
            }
            Wait::TimedOut(status) => {
                lines.push(format!(
                    "connection:    not connected to {} after the wait",
                    report.cp_url
                ));
                if let Some(error) = status
                    .as_ref()
                    .and_then(|status| status.last_error.as_ref())
                {
                    lines.push(format!("last error:    {error}"));
                }
                lines.push(
                    "next:          check the clock of this host, the firewall to the control plane (port 8443) and the URL, then run: restichub-agent status"
                        .to_owned(),
                );
            }
        }
    }
    lines.join("\n")
}

fn stopped_hint() -> String {
    if cfg!(windows) {
        "next:          read the Application log in Event Viewer, fix the cause and run: restichub-agent install-service".to_owned()
    } else {
        "next:          run `journalctl -u restichub-agent`, fix the cause and run: restichub-agent install-service".to_owned()
    }
}

/// The text of `register --no-service` after a successful enrollment.
#[must_use]
pub fn no_service_hint() -> &'static str {
    "the service is not installed; run next: restichub-agent install-service"
}

/// Everything `status` shows (FR-AG-01).
#[derive(Debug)]
pub struct StatusReport {
    pub identity: Result<StatusIdentity, String>,
    pub restic: Option<(String, String)>,
    pub service: ServiceState,
    pub connection: Option<ConnectionStatus>,
}

/// The identity facts of the status output. They hold no secret (FR-AG-04).
#[derive(Debug)]
pub struct StatusIdentity {
    pub agent_id: String,
    pub tenant_id: String,
    pub cp_url: String,
    pub certificate_expiry: OffsetDateTime,
}

impl StatusReport {
    /// True when the service runs and its stream is open.
    #[must_use]
    pub fn connected(&self) -> bool {
        self.identity.is_ok()
            && self.service.running
            && self
                .connection
                .as_ref()
                .is_some_and(|connection| connection.connected)
    }
}

/// Collects the status from the identity, restic, the service manager and the status file.
///
/// A part that cannot be read becomes a line of the report and never stops the others.
#[must_use]
pub fn collect_status(
    manager: &dyn ServiceManager,
    dir: &Path,
    restic: Option<(String, String)>,
) -> StatusReport {
    let identity = require_identity(dir)
        .and_then(|identity| {
            Ok(StatusIdentity {
                certificate_expiry: identity.certificate_expiry()?,
                agent_id: identity.agent_id,
                tenant_id: identity.tenant_id,
                cp_url: identity.cp_url,
            })
        })
        .map_err(|error| format!("{error:#}"));
    StatusReport {
        identity,
        restic,
        service: manager.state().unwrap_or(ServiceState::NOT_INSTALLED),
        connection: ConnectionStatus::read(dir).ok().flatten(),
    }
}

#[must_use]
pub fn render_status(report: &StatusReport) -> String {
    let mut lines = Vec::new();
    match &report.identity {
        Ok(identity) => {
            lines.push(format!("agent id:            {}", identity.agent_id));
            lines.push(format!("organization:        {}", identity.tenant_id));
            lines.push(format!(
                "certificate expires: {}",
                identity.certificate_expiry
            ));
        }
        Err(error) => lines.push(format!("identity:            {error}")),
    }
    match &report.restic {
        Some((version, path)) => lines.push(format!("restic:              {version} ({path})")),
        None => lines.push("restic:              not installed".to_owned()),
    }
    let service = &report.service;
    lines.push(format!(
        "service:             {}",
        if !service.installed {
            "not installed".to_owned()
        } else if service.running {
            format!("installed, running, start type {}", service.start_type)
        } else {
            format!("installed, stopped, start type {}", service.start_type)
        }
    ));
    let connection = report.connection.as_ref();
    let cp_url = connection
        .map(|connection| connection.cp_url.as_str())
        .filter(|url| !url.is_empty())
        .or_else(|| {
            report
                .identity
                .as_ref()
                .ok()
                .map(|identity| identity.cp_url.as_str())
        });
    lines.push(format!(
        "control plane:       {}",
        cp_url.unwrap_or("(unknown)")
    ));
    lines.push(format!(
        "connection:          {}",
        if report.connected() {
            "connected"
        } else {
            "not connected"
        }
    ));
    if let Some(at) = connection.and_then(ConnectionStatus::connected_time) {
        lines.push(format!("last connected:      {at}"));
    }
    if let Some(error) = connection.and_then(|connection| connection.last_error.as_ref()) {
        let at = connection
            .and_then(ConnectionStatus::error_time)
            .map(|at| format!(" ({at})"))
            .unwrap_or_default();
        lines.push(format!("last error:          {error}{at}"));
    }
    lines.join("\n")
}

/// Prints the status and fails when the agent is not connected.
///
/// # Errors
///
/// Returns an error when the agent is not connected, so the exit code is not zero.
pub fn print_status(report: &StatusReport) -> Result<()> {
    println!("{}", render_status(report));
    if report.connected() {
        Ok(())
    } else {
        bail!("the agent is not connected")
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use std::cell::RefCell;
    use std::path::PathBuf;

    use super::*;
    use crate::service::StartType;

    struct Fake {
        state: ServiceState,
        installs: RefCell<Vec<PathBuf>>,
        connect_dir: Option<PathBuf>,
        fail: bool,
    }

    impl Fake {
        fn new(state: ServiceState) -> Self {
            Self {
                state,
                installs: RefCell::new(Vec::new()),
                connect_dir: None,
                fail: false,
            }
        }
    }

    impl ServiceManager for Fake {
        fn install_and_start(&self, restic: &Path) -> Result<()> {
            if self.fail {
                bail!("cannot start the service");
            }
            self.installs.borrow_mut().push(restic.to_path_buf());
            if let Some(dir) = &self.connect_dir {
                crate::connection::record_connected(dir, "https://cp.example:8443");
            }
            Ok(())
        }

        fn state(&self) -> Result<ServiceState> {
            Ok(self.state)
        }

        fn stop(&self) -> Result<()> {
            Ok(())
        }
    }

    const RUNNING: ServiceState = ServiceState {
        installed: true,
        running: true,
        start_type: StartType::Automatic,
    };

    fn quick() -> WaitOptions {
        WaitOptions {
            timeout: Duration::from_millis(80),
            poll: Duration::from_millis(10),
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "restichub-agent-setup-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("cannot create the directory");
        dir
    }

    /// An identity with a real self-signed certificate, so the expiry parses.
    fn save_identity(dir: &Path) {
        let key = rcgen::KeyPair::generate().expect("key");
        let params = rcgen::CertificateParams::new(vec!["agent.example".to_owned()])
            .expect("certificate parameters");
        let certificate = params.self_signed(&key).expect("certificate");
        let identity = StoredIdentity {
            agent_id: "0192f1e0-0000-7000-8000-000000000001".to_owned(),
            tenant_id: "0192f1e0-0000-7000-8000-000000000002".to_owned(),
            cp_url: "https://cp.example:8443".to_owned(),
            grpc_endpoint: None,
            key_pem: key.serialize_pem(),
            certificate_der: certificate.der().to_vec(),
            ca_chain_der: vec![certificate.der().to_vec()],
            signing_keys: crate::identity::StoredSigningKeys {
                keys: Vec::new(),
                signature: "AAAA".to_owned(),
            },
        };
        identity.save(dir).expect("cannot save the identity");
    }

    #[test]
    fn fr_in_08_install_service_without_an_identity_refuses_and_installs_nothing() {
        let dir = temp_dir("no-identity");
        let fake = Fake::new(ServiceState::NOT_INSTALLED);
        let error = install_and_wait(&fake, &dir, Path::new("/restic"), quick())
            .expect_err("a host without an identity must be refused");
        let text = format!("{error:#}");
        assert!(
            text.contains("register first: restichub-agent register"),
            "{text}"
        );
        assert!(fake.installs.borrow().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fr_in_07_a_connected_service_gives_a_connected_report() {
        let dir = temp_dir("connected");
        save_identity(&dir);
        let mut fake = Fake::new(RUNNING);
        fake.connect_dir = Some(dir.clone());
        let report = install_and_wait(&fake, &dir, Path::new("/restic"), quick()).expect("setup");
        assert!(report.connected());
        assert_eq!(
            fake.installs.borrow().as_slice(),
            [PathBuf::from("/restic")]
        );
        let text = render_report(&report, Some("host-1"));
        assert!(
            text.contains("registered as host-1 (agent id 0192f1e0"),
            "{text}"
        );
        assert!(text.contains("installed and running"), "{text}");
        assert!(
            text.contains("connected to https://cp.example:8443"),
            "{text}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fr_in_07_a_running_service_that_does_not_connect_names_the_next_step() {
        let dir = temp_dir("not-connected");
        save_identity(&dir);
        let fake = Fake::new(RUNNING);
        let report = install_and_wait(&fake, &dir, Path::new("/restic"), quick()).expect("setup");
        assert!(!report.connected());
        let text = render_report(&report, None);
        assert!(text.contains("not connected"), "{text}");
        assert!(text.contains("clock"), "{text}");
        assert!(text.contains("restichub-agent status"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fr_in_07_a_stopped_service_names_the_log() {
        let dir = temp_dir("stopped");
        save_identity(&dir);
        let fake = Fake::new(ServiceState {
            running: false,
            ..RUNNING
        });
        let report = install_and_wait(&fake, &dir, Path::new("/restic"), quick()).expect("setup");
        assert!(!report.connected());
        let text = render_report(&report, None);
        assert!(text.contains("installed, not running"), "{text}");
        assert!(
            text.contains("journalctl") || text.contains("Event Viewer"),
            "{text}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fr_in_08_a_failed_start_is_an_error() {
        let dir = temp_dir("start-fails");
        save_identity(&dir);
        let mut fake = Fake::new(RUNNING);
        fake.fail = true;
        assert!(install_and_wait(&fake, &dir, Path::new("/restic"), quick()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fr_ag_01_status_exits_non_zero_unless_connected() {
        let dir = temp_dir("status-exit");
        save_identity(&dir);
        let stopped = Fake::new(ServiceState {
            running: false,
            ..RUNNING
        });
        crate::connection::record_connected(&dir, "https://cp.example:8443");
        // The file says connected, but a stopped service cannot hold a stream.
        let report = collect_status(&stopped, &dir, None);
        assert!(!report.connected());
        assert!(print_status(&report).is_err());

        let running = Fake::new(RUNNING);
        let report = collect_status(&running, &dir, Some(("0.19.1".to_owned(), "/r".to_owned())));
        assert!(report.connected());
        assert!(print_status(&report).is_ok());

        crate::connection::record_failure(&dir, "https://cp.example:8443", "tls handshake failed");
        let report = collect_status(&running, &dir, None);
        assert!(!report.connected());
        let text = render_status(&report);
        assert!(
            text.contains("last error:          tls handshake failed"),
            "{text}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fr_ag_01_status_shows_the_facts_and_no_secret() {
        let dir = temp_dir("status-text");
        save_identity(&dir);
        crate::connection::record_connected(&dir, "https://cp.example:8443");
        let fake = Fake::new(RUNNING);
        let report = collect_status(
            &fake,
            &dir,
            Some(("0.19.1".to_owned(), "/opt/r".to_owned())),
        );
        let text = render_status(&report);
        for expected in [
            "agent id:            0192f1e0-0000-7000-8000-000000000001",
            "organization:        0192f1e0-0000-7000-8000-000000000002",
            "certificate expires:",
            "restic:              0.19.1 (/opt/r)",
            "installed, running, start type automatic",
            "control plane:       https://cp.example:8443",
            "connection:          connected",
            "last connected:",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in\n{text}");
        }
        assert!(!text.contains("PRIVATE KEY"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fr_ag_01_status_without_an_identity_says_what_to_run() {
        let dir = temp_dir("status-empty");
        let fake = Fake::new(ServiceState::NOT_INSTALLED);
        let report = collect_status(&fake, &dir, None);
        assert!(!report.connected());
        let text = render_status(&report);
        assert!(text.contains("register first"), "{text}");
        assert!(text.contains("not installed"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
