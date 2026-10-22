#![allow(clippy::expect_used)]

use super::*;

mod trust_mode {
    use std::cell::{Cell, RefCell};
    use std::time::Duration;

    use prost_types::Timestamp;
    use restichub_proto::signing::JobJournal as _;

    use super::*;
    use crate::journal::{FileJournal, JOURNAL_FILE};
    use crate::service::{ServiceState, StartType};

    /// A service double that records the calls in order. Its state is fixed; a restart writes
    /// the connection status like a service that connected, when `connects` is set.
    struct Fake {
        state: ServiceState,
        dir: PathBuf,
        connects: bool,
        restart_fails: bool,
        events: RefCell<Vec<String>>,
    }

    impl Fake {
        fn new(dir: &Path, state: ServiceState) -> Self {
            Self {
                state,
                dir: dir.to_path_buf(),
                connects: true,
                restart_fails: false,
                events: RefCell::new(Vec::new()),
            }
        }

        fn events(&self) -> Vec<String> {
            self.events.borrow().clone()
        }
    }

    impl ServiceManager for Fake {
        fn install_and_start(&self, _restic: &Path) -> Result<()> {
            self.events.borrow_mut().push("start".to_owned());
            Ok(())
        }

        fn state(&self) -> Result<ServiceState> {
            Ok(self.state)
        }

        fn stop(&self) -> Result<()> {
            Ok(())
        }

        fn restart(&self) -> Result<()> {
            self.events.borrow_mut().push("restart".to_owned());
            if self.restart_fails {
                bail!("cannot stop the service");
            }
            if self.connects {
                crate::connection::record_connected(&self.dir, "https://cp.example:8443");
            }
            Ok(())
        }
    }

    const RUNNING: ServiceState = ServiceState {
        installed: true,
        running: true,
        start_type: StartType::Automatic,
    };

    const STOPPED: ServiceState = ServiceState {
        installed: true,
        running: false,
        start_type: StartType::Automatic,
    };

    fn quick() -> WaitOptions {
        WaitOptions {
            timeout: Duration::from_millis(100),
            poll: Duration::from_millis(10),
        }
    }

    fn config_dir(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "restichub-agent-trustmode-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let dir = base.join("config");
        std::fs::create_dir_all(&dir).expect("config dir");
        dir
    }

    /// Writes the mode file without the trust check, which needs root or Administrators.
    fn write_mode(dir: &Path, enabled: bool) -> Result<()> {
        let value = if enabled { "enabled" } else { "disabled" };
        std::fs::write(
            crate::scripts::mode_file_for(dir),
            format!("server_scripts = {value}\n"),
        )?;
        Ok(())
    }

    fn refuse_mode(_dir: &Path, _enabled: bool) -> Result<()> {
        bail!("the agent would not trust the file")
    }

    fn mode_text(dir: &Path) -> String {
        std::fs::read_to_string(crate::scripts::mode_file_for(dir)).unwrap_or_default()
    }

    fn cleanup(dir: &Path) {
        let _ = std::fs::remove_dir_all(dir.parent().expect("parent"));
    }

    fn apply(fake: &Fake, dir: &Path, enable: bool, no_restart: bool) -> Result<()> {
        apply_trust_mode(fake, dir, enable, no_restart, quick(), &write_mode)
    }

    fn accept_job(dir: &Path, finished: bool) {
        let mut journal = FileJournal::load(dir.join(JOURNAL_FILE)).expect("journal");
        let expires = Timestamp {
            seconds: OffsetDateTime::now_utc().unix_timestamp() + 3600,
            nanos: 0,
        };
        journal.record_accepted("job-1", expires);
        if finished {
            journal.record_completed(
                "job-1",
                restichub_proto::agent::v1::JobResult {
                    job_id: "job-1".to_owned(),
                    ..Default::default()
                },
            );
        }
        journal.persist().expect("persist");
    }

    /// Starts the service on the journal: it marks every unfinished entry as an orphan of the
    /// process that ended.
    fn service_started(dir: &Path) {
        crate::jobs::Jobs::open(
            PathBuf::from("restic"),
            String::new(),
            dir.join("cache"),
            dir,
        )
        .expect("the service opens the journal");
    }

    #[test]
    fn fr_hk_08_trust_enable_restarts_an_idle_service_and_waits_for_the_connection() {
        let dir = config_dir("restart");
        accept_job(&dir, true);
        let fake = Fake::new(&dir, RUNNING);
        apply(&fake, &dir, true, false).expect("apply");
        assert!(mode_text(&dir).contains("server_scripts = enabled"));
        assert_eq!(fake.events(), ["restart"]);
        cleanup(&dir);
    }

    #[test]
    fn fr_hk_08_trust_change_does_not_restart_a_service_with_a_job_in_progress() {
        let dir = config_dir("busy");
        accept_job(&dir, false);
        let fake = Fake::new(&dir, RUNNING);
        apply(&fake, &dir, false, false).expect("apply");
        assert!(mode_text(&dir).contains("server_scripts = disabled"));
        assert!(fake.events().is_empty());
        cleanup(&dir);
    }

    #[test]
    fn fr_hk_08_a_journal_that_cannot_be_read_does_not_prove_an_idle_service() {
        let dir = config_dir("bad-journal");
        std::fs::write(dir.join(JOURNAL_FILE), b"not json").expect("write");
        let fake = Fake::new(&dir, RUNNING);
        apply(&fake, &dir, true, false).expect("apply");
        assert!(fake.events().is_empty());
        cleanup(&dir);
    }

    /// FR-AG-10, FR-IN-08: a command that would stop the service refuses while a job runs, because
    /// the stop cuts restic short and it can keep its repository lock.
    #[test]
    fn fr_ag_10_a_command_that_stops_the_service_is_refused_while_a_job_runs() {
        let dir = config_dir("busy-stop");
        let fake = Fake::new(&dir, RUNNING);
        refuse_running_job(&fake, &dir).expect("an idle service is no refusal");

        accept_job(&dir, false);
        let error = refuse_running_job(&fake, &dir).expect_err("a running job must refuse");
        let text = format!("{error:#}");
        assert!(text.starts_with("cannot stop the service: "), "{text}");
        assert!(
            text.contains("the job journal shows a job in progress"),
            "{text}"
        );
        assert!(text.contains("then run the command again"), "{text}");
        assert!(fake.events().is_empty(), "a refusal touches no service");

        let stopped = Fake::new(&dir, STOPPED);
        refuse_running_job(&stopped, &dir).expect("a stopped service runs no job");

        accept_job(&dir, true);
        refuse_running_job(&fake, &dir).expect("a finished job is no refusal");

        std::fs::write(dir.join(JOURNAL_FILE), b"not json").expect("write");
        let error = refuse_running_job(&fake, &dir).expect_err("a bad journal must refuse");
        assert!(
            format!("{error:#}").contains("the job journal cannot be read"),
            "{error:#}"
        );
        cleanup(&dir);
    }

    /// FR-IN-08: an entry that the previous process left without a result runs no job, so it does
    /// not refuse the command; a job of the live service does.
    #[test]
    fn fr_in_08_an_entry_of_the_previous_process_does_not_refuse_a_command() {
        let dir = config_dir("stale-job");
        let fake = Fake::new(&dir, RUNNING);
        accept_job(&dir, false);
        service_started(&dir);
        refuse_running_job(&fake, &dir).expect("no job of the live service runs");

        let mut journal = FileJournal::load(dir.join(JOURNAL_FILE)).expect("journal");
        let expires = Timestamp {
            seconds: OffsetDateTime::now_utc().unix_timestamp() + 3600,
            nanos: 0,
        };
        journal.record_accepted("job-2", expires);
        journal.persist().expect("persist");
        let error = refuse_running_job(&fake, &dir).expect_err("a job of the live service refuses");
        let text = format!("{error:#}");
        assert!(
            text.contains("the job journal shows a job in progress"),
            "{text}"
        );
        assert!(text.contains("wait for the job to end"), "{text}");
        assert!(!text.contains("by hand"), "{text}");
        cleanup(&dir);
    }

    #[test]
    fn fr_hk_08_trust_enable_fails_when_the_agent_does_not_reconnect() {
        let dir = config_dir("no-reconnect");
        let mut fake = Fake::new(&dir, RUNNING);
        fake.connects = false;
        let error = format!(
            "{:#}",
            apply(&fake, &dir, true, false).expect_err("no connection")
        );
        assert!(error.contains("did not connect again"), "{error}");
        assert!(error.contains("the mode is written"), "{error}");
        cleanup(&dir);
    }

    #[test]
    fn fr_hk_08_a_failed_restart_says_that_the_mode_is_written() {
        let dir = config_dir("restart-fails");
        let mut fake = Fake::new(&dir, RUNNING);
        fake.restart_fails = true;
        let error = format!(
            "{:#}",
            apply(&fake, &dir, false, false).expect_err("restart fails")
        );
        assert!(error.contains("the mode is written"), "{error}");
        assert!(error.contains(RESTART_COMMAND), "{error}");
        assert!(mode_text(&dir).contains("server_scripts = disabled"));
        cleanup(&dir);
    }

    #[test]
    fn fr_hk_08_trust_change_does_not_restart_a_stopped_or_absent_service() {
        let dir = config_dir("stopped");
        for state in [STOPPED, ServiceState::NOT_INSTALLED] {
            let fake = Fake::new(&dir, state);
            apply(&fake, &dir, false, false).expect("apply");
            assert!(fake.events().is_empty());
        }
        assert!(mode_text(&dir).contains("server_scripts = disabled"));
        cleanup(&dir);
    }

    #[test]
    fn fr_hk_08_no_restart_leaves_the_service_alone() {
        let dir = config_dir("no-restart");
        let fake = Fake::new(&dir, RUNNING);
        apply(&fake, &dir, true, true).expect("apply");
        assert!(fake.events().is_empty());
        cleanup(&dir);
    }

    #[tokio::test]
    async fn fr_hk_08_register_writes_the_trust_mode_before_the_enrollment() {
        let dir = config_dir("register");
        let enroll = async {
            assert!(mode_text(&dir).contains("server_scripts = enabled"));
            Ok(())
        };
        enroll_with_trust_mode(&dir, false, &write_mode, enroll)
            .await
            .expect("register");
        assert!(mode_text(&dir).contains("server_scripts = enabled"));
        cleanup(&dir);
    }

    #[tokio::test]
    async fn fr_hk_08_a_mode_that_cannot_be_written_does_not_spend_the_token() {
        let dir = config_dir("register-refused");
        let enrolled = Cell::new(false);
        let enroll = async {
            enrolled.set(true);
            Ok(())
        };
        let error = enroll_with_trust_mode(&dir, false, &refuse_mode, enroll)
            .await
            .expect_err("the mode cannot be written");
        assert!(
            format!("{error:#}").contains("token is not used"),
            "{error:#}"
        );
        assert!(!enrolled.get(), "the enrollment ran");
        cleanup(&dir);
    }

    #[tokio::test]
    async fn fr_hk_08_a_failed_enrollment_puts_the_earlier_mode_back() {
        let dir = config_dir("register-fails");
        let failed = || async { Err(anyhow::anyhow!("the control plane refused the token")) };
        enroll_with_trust_mode(&dir, false, &write_mode, failed())
            .await
            .expect_err("the enrollment fails");
        assert!(!crate::scripts::mode_file_for(&dir).exists());
        write_mode(&dir, false).expect("write");
        enroll_with_trust_mode(&dir, false, &write_mode, failed())
            .await
            .expect_err("the enrollment fails");
        assert!(mode_text(&dir).contains("server_scripts = disabled"));
        cleanup(&dir);
    }
}

#[test]
fn fr_in_07_a_moved_restic_that_differs_from_its_record_fails() {
    let base = std::env::temp_dir().join(format!("restichub-agent-moved-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let layout = Layout::new(base.join("bin"), base.join("config"));
    std::fs::create_dir_all(layout.bin_dir()).expect("bin");
    std::fs::create_dir_all(base.join("config")).expect("config");
    std::fs::write(layout.binary_path(), b"restic").expect("restic");
    let wrong = "a".repeat(64);
    std::fs::write(
        layout.record_path(),
        format!(r#"{{"version":"0.19.1","sha256":"{wrong}"}}"#),
    )
    .expect("record");

    let error = verify_moved_restic(&layout).expect_err("mismatch");

    assert!(format!("{error:#}").contains("differs from its record after the move"));
    assert!(layout.binary_path().exists());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn fr_ag_01_a_missing_or_changed_restic_stops_the_service_for_an_administrator() {
    use sha2::Digest as _;
    let base =
        std::env::temp_dir().join(format!("restichub-agent-no-restic-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let layout = Layout::new(base.join("bin"), base.join("config"));
    std::fs::create_dir_all(layout.bin_dir()).expect("bin");
    std::fs::create_dir_all(base.join("config")).expect("config");
    let refused = |state: &str, cause: String| {
        let error = installed_restic(&layout, Ok(()), || {}).expect_err(state);
        assert!(needs_administrator(&error), "{state}: {error:#}");
        assert_eq!(
            format!("{error:#}"),
            format!(
                "restic is not installed or differs from its record: {cause}; the service stays stopped until an administrator runs `restichub-agent install-service`, which installs restic and starts the service; on a host without the restic download, add `--restic-archive <ARCHIVE>` with the restic release archive for this host"
            ),
            "{state}"
        );
    };
    let hash = |bytes: &[u8]| crate::restic_release::hex(&sha2::Sha256::digest(bytes));

    refused(
        "no record",
        format!(
            "restic is not installed: cannot read {}; run `restichub-agent restic update`: {}",
            layout.record_path().display(),
            std::io::Error::from_raw_os_error(2)
        ),
    );
    let record = |bytes: &[u8]| {
        std::fs::write(
            layout.record_path(),
            format!(r#"{{"version":"0.19.1","sha256":"{}"}}"#, hash(bytes)),
        )
        .expect("record");
    };
    record(b"restic");
    refused(
        "no binary",
        format!(
            "the restic binary {} is missing; run `restichub-agent restic update`",
            layout.binary_path().display()
        ),
    );
    std::fs::write(layout.binary_path(), b"changed").expect("restic");
    refused(
        "a changed binary",
        format!(
            "the restic binary does not match the recorded SHA-256: expected {}, got {}",
            hash(b"restic"),
            hash(b"changed")
        ),
    );
    record(b"changed");
    installed_restic(&layout, Ok(()), || {}).expect("the recorded binary passes");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn fr_ag_01_the_service_refuses_a_program_that_another_account_could_change() {
    // The temporary directory belongs to the test account, not to root or Administrators.
    let base = std::env::temp_dir().join(format!(
        "restichub-agent-open-program-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).expect("base");
    let exe = base.join("restichub-agent");
    let restic = base.join(crate::restic::RESTIC_FILE_NAME);
    std::fs::write(&exe, b"agent").expect("agent");
    std::fs::write(&restic, b"restic").expect("restic");

    let error = anyhow::Error::new(check_program(&exe, &restic).expect_err("an open program"));
    assert!(needs_administrator(&error), "{error:#}");
    let text = format!("{error:#}");
    assert!(
        text.starts_with("another account could change the agent program or restic: "),
        "{text}"
    );
    assert!(
        text.contains("runs `install-service` of the release package"),
        "{text}"
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn fr_ag_01_a_start_during_the_restic_swap_reads_restic_again() {
    use sha2::Digest as _;
    let base = std::env::temp_dir().join(format!(
        "restichub-agent-swap-window-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    let layout = Layout::new(base.join("bin"), base.join("config"));
    std::fs::create_dir_all(layout.bin_dir()).expect("bin");
    std::fs::create_dir_all(base.join("config")).expect("config");
    let record = |bytes: &[u8]| {
        let hash = crate::restic_release::hex(&sha2::Sha256::digest(bytes));
        std::fs::write(
            layout.record_path(),
            format!(r#"{{"version":"0.19.1","sha256":"{hash}"}}"#),
        )
        .expect("record");
    };
    record(b"old");
    std::fs::write(layout.binary_path(), b"new").expect("the renamed binary");

    installed_restic(&layout, Ok(()), || record(b"new"))
        .expect("the record that the update writes next passes");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn fr_ag_01_a_transient_failure_to_move_an_old_restic_restarts_the_service() {
    let base = std::env::temp_dir().join(format!(
        "restichub-agent-move-failed-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    let layout = Layout::new(base.join("bin"), base.join("config"));
    let failed_move = |kind: std::io::ErrorKind| {
        Err(anyhow::Error::new(std::io::Error::from(kind)).context("cannot copy the old restic"))
    };

    let locked = installed_restic(
        &layout,
        failed_move(std::io::ErrorKind::PermissionDenied),
        || {},
    )
    .expect_err("restic is not in place");
    assert!(!needs_administrator(&locked), "{locked:#}");
    assert!(
        format!("{locked:#}").starts_with("restic is not in place: "),
        "{locked:#}"
    );
    let gone = installed_restic(&layout, failed_move(std::io::ErrorKind::NotFound), || {})
        .expect_err("restic is not in place");
    assert!(needs_administrator(&gone), "{gone:#}");
    let _ = std::fs::remove_dir_all(&base);
}

fn planted_rotation(name: &str) -> (PathBuf, PathBuf) {
    let base = std::env::temp_dir().join(format!(
        "restichub-agent-install-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    let dir = base.join("ResticHub").join("Agent");
    std::fs::create_dir_all(&dir).expect("cannot create the directory");
    std::fs::write(dir.join("agent-key.pem"), b"key").expect("cannot write the key");
    std::fs::write(dir.join("agent-key.pem.next"), b"planted").expect("cannot write");
    std::fs::write(dir.join("rotation.next"), b"").expect("cannot write the marker");
    (base, dir)
}

#[cfg(unix)]
#[test]
fn fr_ag_04_install_service_prepares_the_directory_before_it_loads_the_identity() {
    use std::os::unix::fs::PermissionsExt as _;
    let (base, dir) = planted_rotation("order");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777))
        .expect("cannot set the mode");
    let error = prepare_install(&dir, &base.join("cache"), &StopCounter::new(false), false)
        .expect_err("no identity");
    assert!(
        format!("{error:#}").contains("no valid agent identity"),
        "{error:#}"
    );
    let mode = std::fs::metadata(&dir)
        .expect("cannot read the directory")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o700, "the directory was not prepared first");
    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(windows)]
#[test]
fn fr_ag_04_install_service_refuses_an_open_directory_before_it_loads_the_identity() {
    let (base, dir) = planted_rotation("order");
    crate::winsec::tests::grant(&dir, "(OI)(CI)(WD)").expect("cannot add the entry");
    let error = prepare_install(&dir, &dir.join("cache"), &StopCounter::new(false), false)
        .expect_err("an open directory");
    assert!(
        format!("{error:#}").contains("cannot prepare the directory"),
        "{error:#}"
    );
    assert_eq!(
        std::fs::read(dir.join("agent-key.pem")).expect("the key must stay"),
        b"key"
    );
    assert!(dir.join("agent-key.pem.next").exists());
    let _ = std::fs::remove_dir_all(&base);
}

/// A service double that counts the stops; a stop makes it not running.
struct StopCounter {
    state: std::cell::Cell<crate::service::ServiceState>,
    stops: std::cell::Cell<u32>,
}

impl StopCounter {
    fn new(running: bool) -> Self {
        Self {
            state: std::cell::Cell::new(crate::service::ServiceState {
                installed: true,
                running,
                start_type: crate::service::StartType::Automatic,
            }),
            stops: std::cell::Cell::new(0),
        }
    }
}

impl ServiceManager for StopCounter {
    fn install_and_start(&self, _restic: &Path) -> Result<()> {
        bail!("a refused identity must not install the service")
    }

    fn state(&self) -> Result<crate::service::ServiceState> {
        Ok(self.state.get())
    }

    fn stop(&self) -> Result<()> {
        self.stops.set(self.stops.get() + 1);
        let mut state = self.state.get();
        state.running = false;
        self.state.set(state);
        Ok(())
    }

    fn restart(&self) -> Result<()> {
        bail!("a refused identity must not restart the service")
    }
}

#[test]
fn fr_ag_04_install_service_refuses_an_identity_that_another_account_could_change_and_stops_the_service()
 {
    let chain = PathBuf::from("Agent").join(crate::identity::CA_CHAIN_FILE);
    let changeable = ChangeableIdentity {
        paths: vec![chain.clone()],
    };
    let manager = StopCounter::new(true);
    let error = format!("{:#}", refuse_changeable_identity(&changeable, &manager));
    assert_eq!(manager.stops.get(), 1, "the running service must stop");
    assert!(error.contains(&chain.display().to_string()), "{error}");
    assert!(error.contains("the service is not installed"), "{error}");
    assert!(
        error.contains("the running service is stopped and stays installed"),
        "{error}"
    );
    assert!(
        error.contains("register --force --url <URL> --token <TOKEN>"),
        "{error}"
    );

    let error = format!("{:#}", refuse_changeable_identity(&changeable, &manager));
    assert_eq!(manager.stops.get(), 1, "a stopped service is left alone");
    assert!(!error.contains("running service"), "{error}");
}

#[cfg(unix)]
#[test]
fn fr_ag_04_the_service_does_not_start_with_an_identity_that_another_account_could_change() {
    use std::os::unix::fs::PermissionsExt as _;
    let (base, dir) = planted_rotation("unsafe-run");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777))
        .expect("cannot set the mode");
    let error = run(&dir, None).expect_err("the service must refuse");
    assert!(needs_administrator(&error), "{error:#}");
    let text = format!("{error:#}");
    assert!(text.contains("install-service"), "{text}");
    assert!(text.contains("register --force"), "{text}");
    assert!(
        dir.join("agent-key.pem.next").exists(),
        "the refusal reads nothing and finishes no rotation"
    );
    let report = crate::setup::collect_status(&StopCounter::new(false), &dir, None);
    let identity = report.identity.expect_err("status must report the refusal");
    assert!(identity.contains("does not use the identity"), "{identity}");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn fr_ag_04_a_check_without_the_rights_asks_for_an_administrator() {
    let denied = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        .context("cannot open C:\\ProgramData\\ResticHub");
    let text = unsafe_identity(Path::new("Agent"), &denied).to_string();
    assert!(
        text.contains("run the command as an administrator"),
        "{text}"
    );
    assert!(!text.contains("register"), "{text}");

    let open = anyhow::anyhow!("Agent grants a write access to an account");
    let text = unsafe_identity(Path::new("Agent"), &open).to_string();
    assert!(text.contains("register --force"), "{text}");
}

#[test]
fn the_token_sources_are_mutually_exclusive() {
    let options = RegisterOptions {
        token: Some("re_x".to_owned()),
        token_file: Some(std::path::PathBuf::from("/tmp/token")),
        ..RegisterOptions::default()
    };
    assert!(resolve_token(&options).is_err());
}

#[test]
fn fr_ag_01_only_a_missing_or_damaged_identity_is_a_no_identity_error() {
    let missing = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::NotFound))
        .context("cannot read identity.json");
    let denied = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        .context("cannot read identity.json");
    let damaged = anyhow::anyhow!("bad identity.json");
    assert!(is_missing_or_damaged(&missing));
    assert!(is_missing_or_damaged(&damaged));
    assert!(!is_missing_or_damaged(&denied));
}

#[test]
fn an_empty_token_error_tells_what_to_do() {
    let options = RegisterOptions {
        token: Some("  \n".to_owned()),
        ..RegisterOptions::default()
    };
    let error = resolve_token(&options)
        .expect_err("an empty token")
        .to_string();
    assert!(error.contains("--token-file"), "{error}");
    assert!(error.contains("console"), "{error}");
}

#[test]
fn a_not_connected_error_names_the_next_command() {
    assert!(NOT_CONNECTED.contains("restichub-agent status"));
}

#[test]
fn a_missing_token_source_is_refused() {
    assert!(resolve_token(&RegisterOptions::default()).is_err());
}

#[test]
fn an_inline_token_is_trimmed() {
    let options = RegisterOptions {
        token: Some("  re_x\n".to_owned()),
        ..RegisterOptions::default()
    };
    assert_eq!(resolve_token(&options).expect("token"), "re_x");
}
