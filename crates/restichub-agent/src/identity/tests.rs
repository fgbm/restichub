#![allow(clippy::expect_used)]

use super::*;

pub(crate) fn sample() -> StoredIdentity {
    StoredIdentity {
        agent_id: "0192f1e0-0000-7000-8000-000000000001".to_owned(),
        tenant_id: "0192f1e0-0000-7000-8000-000000000002".to_owned(),
        cp_url: "https://cp.example:8443".to_owned(),
        grpc_endpoint: Some("https://agents.example:8443".to_owned()),
        key_pem: "-----BEGIN PRIVATE KEY-----\nMIIB\n-----END PRIVATE KEY-----\n".to_owned(),
        certificate_der: vec![1, 2, 3],
        ca_chain_der: vec![vec![4, 5], vec![6, 7]],
        signing_keys: StoredSigningKeys {
            keys: vec![crate::enroll::EnrollmentKeyBody {
                role: "current".to_owned(),
                key_id: "key-1".to_owned(),
                public_key: "AAAA".to_owned(),
            }],
            signature: "AAAA".to_owned(),
        },
    }
}

#[cfg(unix)]
fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "restichub-agent-identity-{name}-{}",
        std::process::id()
    ));
    remove(&dir);
    dir
}

#[cfg(not(unix))]
fn temp_dir(name: &str) -> std::path::PathBuf {
    // The parent of the configuration directory must be SYSTEM and Administrators only, so the
    // test builds a protected base and puts the identity inside it.
    let base = std::env::temp_dir().join(format!(
        "restichub-agent-identity-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    crate::paths::create_protected_dir(&base).expect("cannot create the protected base");
    base.join("agent")
}

fn remove(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_file(lock_file(dir));
}

#[test]
fn the_identity_round_trips() {
    let dir = temp_dir("round-trip");
    let identity = sample();
    identity.save(&dir).expect("cannot save the identity");
    assert!(StoredIdentity::exists(&dir));
    let loaded = StoredIdentity::load(&dir).expect("cannot load the identity");
    assert_eq!(loaded.agent_id, identity.agent_id);
    assert_eq!(loaded.tenant_id, identity.tenant_id);
    assert_eq!(loaded.cp_url, identity.cp_url);
    assert_eq!(loaded.grpc_endpoint, identity.grpc_endpoint);
    assert_eq!(loaded.key_pem, identity.key_pem);
    assert_eq!(loaded.certificate_der, identity.certificate_der);
    assert_eq!(loaded.ca_chain_der, identity.ca_chain_der);
    assert_eq!(loaded.signing_keys, identity.signing_keys);
    remove(&dir);
}

#[test]
fn a_directory_without_an_identity_is_reported() {
    let dir = temp_dir("absent");
    assert!(!StoredIdentity::exists(&dir));
}

#[test]
fn fr_in_07_a_save_keeps_the_files_that_are_not_identity_files() {
    let dir = temp_dir("keeps-other");
    crate::paths::ensure_config_dir(&dir).expect("cannot create the directory");
    std::fs::write(dir.join("restic.json"), b"{}").expect("write");
    sample().save(&dir).expect("cannot save the identity");
    assert!(dir.join("restic.json").is_file());
    assert!(StoredIdentity::exists(&dir));
    remove(&dir);
}

#[test]
fn fr_in_07_a_save_drops_the_state_of_the_old_identity() {
    let dir = temp_dir("drops-old-state");
    crate::paths::ensure_config_dir(&dir).expect("cannot create the directory");
    std::fs::write(dir.join("signing-key-updates.json"), b"{}").expect("write");
    std::fs::write(dir.join("status.json"), b"{}").expect("write");
    sample().save(&dir).expect("cannot save the identity");
    assert!(!dir.join("signing-key-updates.json").exists());
    assert!(!dir.join("status.json").exists());
    remove(&dir);
}

#[test]
fn a_second_save_replaces_the_identity() {
    let dir = temp_dir("atomic-replace");
    sample().save(&dir).expect("the first save must succeed");
    let mut new = sample();
    new.agent_id = "new-agent".to_owned();
    new.save(&dir).expect("the second save must succeed");
    let loaded = StoredIdentity::load(&dir).expect("cannot load the identity");
    assert_eq!(loaded.agent_id, "new-agent");
    assert!(dir.exists());
    assert!(!sibling(&dir, ".staging").exists());
    assert!(!sibling(&dir, ".old").exists());
    remove(&dir);
}

#[test]
fn a_failed_write_keeps_the_old_identity() {
    let dir = temp_dir("atomic-failure");
    let old = sample();
    old.save(&dir).expect("the first save must succeed");
    // A file where the staging directory belongs makes the staging fail.
    let staging = sibling(&dir, ".staging");
    std::fs::write(&staging, b"occupied").expect("cannot occupy the staging path");
    let mut new = sample();
    new.agent_id = "new-agent".to_owned();
    assert!(new.save(&dir).is_err());
    let loaded = StoredIdentity::load(&dir).expect("the old identity must survive");
    assert_eq!(loaded.agent_id, old.agent_id);
    remove(&dir);
    let _ = std::fs::remove_file(&staging);
}

#[test]
fn a_failed_swap_is_recovered_from_the_backup() {
    let dir = temp_dir("recover");
    let identity = sample();
    identity.save(&dir).expect("the first save must succeed");
    // A swap that a crash interrupted leaves the identity in the backup and no main directory.
    let backup = sibling(&dir, ".old");
    std::fs::rename(&dir, &backup).expect("cannot move the identity to the backup");
    assert!(!dir.exists());
    let loaded = StoredIdentity::load(&dir).expect("the backup must recover");
    assert_eq!(loaded.agent_id, identity.agent_id);
    assert!(dir.exists());
    remove(&dir);
}

fn install_from_sample(dir: &Path, rotated: &Rotated) -> Result<()> {
    let sample = sample();
    StoredIdentity::install_rotation(dir, &sample.agent_id, &sample.certificate_der, rotated)
}

fn rotated() -> Rotated {
    Rotated {
        key_pem: "-----BEGIN PRIVATE KEY-----\nNEW\n-----END PRIVATE KEY-----\n".to_owned(),
        certificate_der: vec![9, 9, 9],
        ca_chain_der: vec![vec![8, 8], vec![6, 7]],
    }
}

#[test]
fn fr_id_03_a_rotation_replaces_the_files_and_keeps_the_rest_of_the_state() {
    let dir = temp_dir("rotation");
    sample().save(&dir).expect("the save must succeed");
    std::fs::write(dir.join("journal.db"), b"state").expect("cannot write other state");
    install_from_sample(&dir, &rotated()).expect("the rotation must install");
    let loaded = StoredIdentity::load(&dir).expect("the identity must load");
    assert_eq!(loaded.certificate_der, vec![9, 9, 9]);
    assert!(loaded.key_pem.contains("NEW"));
    assert_eq!(loaded.ca_chain_der, vec![vec![8, 8], vec![6, 7]]);
    assert_eq!(loaded.agent_id, sample().agent_id);
    assert_eq!(loaded.signing_keys, sample().signing_keys);
    assert_eq!(
        std::fs::read(dir.join("journal.db")).expect("other state"),
        b"state"
    );
    assert!(!dir.join(ROTATION_MARKER).exists());
    assert!(!staged(&dir, KEY_FILE).exists());
    remove(&dir);
}

#[test]
fn fr_id_03_a_rotation_that_stopped_after_the_marker_is_finished_at_the_next_load() {
    let dir = temp_dir("rotation-crash");
    sample().save(&dir).expect("the save must succeed");
    // A crash after the marker and the first move leaves the new key beside the old
    // certificate and the staged rest.
    let new = rotated();
    paths::create_protected_file(&staged(&dir, KEY_FILE), new.key_pem.as_bytes())
        .expect("stage the key");
    paths::create_protected_file(
        &staged(&dir, CERT_FILE),
        pem::encode("CERTIFICATE", &new.certificate_der).as_bytes(),
    )
    .expect("stage the certificate");
    let chain: String = new
        .ca_chain_der
        .iter()
        .map(|der| pem::encode("CERTIFICATE", der))
        .collect();
    paths::create_protected_file(&staged(&dir, CA_CHAIN_FILE), chain.as_bytes())
        .expect("stage the chain");
    paths::create_protected_file(&dir.join(ROTATION_MARKER), b"1").expect("the marker");
    std::fs::rename(staged(&dir, KEY_FILE), dir.join(KEY_FILE)).expect("move the key");

    let loaded = StoredIdentity::load(&dir).expect("the identity must load");
    assert_eq!(loaded.certificate_der, vec![9, 9, 9]);
    assert!(loaded.key_pem.contains("NEW"));
    assert!(!dir.join(ROTATION_MARKER).exists());
    remove(&dir);
}

#[test]
fn fr_id_03_staged_files_without_the_marker_are_dropped_and_the_old_identity_stays() {
    let dir = temp_dir("rotation-partial");
    sample().save(&dir).expect("the save must succeed");
    paths::create_protected_file(&staged(&dir, KEY_FILE), b"half").expect("stage the key");
    let loaded = StoredIdentity::load(&dir).expect("the identity must load");
    assert_eq!(loaded.certificate_der, sample().certificate_der);
    assert!(loaded.key_pem.contains("MIIB"));
    assert!(!staged(&dir, KEY_FILE).exists());
    remove(&dir);
}

#[test]
fn fr_id_03_a_rotation_of_a_replaced_identity_is_refused_and_keeps_the_new_one() {
    let dir = temp_dir("rotation-replaced");
    sample().save(&dir).expect("the save must succeed");
    // The rotation started from the sample identity; a register then stored another one.
    let mut new = sample();
    new.key_pem = "new registration key".to_owned();
    new.certificate_der = vec![7, 7, 7];
    new.save(&dir).expect("the second save must succeed");

    let error = install_from_sample(&dir, &rotated()).expect_err("the rotation must be refused");
    assert_eq!(
        error.downcast_ref::<IdentityReplaced>(),
        Some(&IdentityReplaced::Certificate),
        "{error:#}"
    );
    let loaded = StoredIdentity::load(&dir).expect("the identity must load");
    assert_eq!(loaded.key_pem, new.key_pem);
    assert_eq!(loaded.certificate_der, new.certificate_der);
    assert_eq!(loaded.ca_chain_der, new.ca_chain_der);
    for name in [KEY_FILE, CERT_FILE, CA_CHAIN_FILE] {
        assert!(!staged(&dir, name).exists(), "{name} is staged");
    }
    assert!(!dir.join(ROTATION_MARKER).exists());

    let mut other = sample();
    other.agent_id = "other-agent".to_owned();
    other.save(&dir).expect("the third save must succeed");
    let error = install_from_sample(&dir, &rotated()).expect_err("the rotation must be refused");
    assert_eq!(
        error.downcast_ref::<IdentityReplaced>(),
        Some(&IdentityReplaced::Agent),
        "{error:#}"
    );
    let loaded = StoredIdentity::load(&dir).expect("the identity must load");
    assert_eq!(loaded.agent_id, "other-agent");
    assert_eq!(loaded.key_pem, sample().key_pem);
    assert_eq!(loaded.certificate_der, sample().certificate_der);
    remove(&dir);
}

#[test]
fn fr_id_03_a_new_rotation_finishes_a_half_moved_staged_set_first() {
    let dir = temp_dir("rotation-half-moved");
    sample().save(&dir).expect("the save must succeed");
    let earlier = Rotated {
        key_pem: "earlier key".to_owned(),
        certificate_der: vec![1, 2, 3],
        ca_chain_der: vec![vec![4, 5]],
    };
    // The earlier run staged a complete set, moved the key and stopped.
    paths::create_protected_file(&staged(&dir, KEY_FILE), earlier.key_pem.as_bytes())
        .expect("stage the key");
    paths::create_protected_file(
        &staged(&dir, CERT_FILE),
        pem::encode("CERTIFICATE", &earlier.certificate_der).as_bytes(),
    )
    .expect("stage the certificate");
    paths::create_protected_file(
        &staged(&dir, CA_CHAIN_FILE),
        pem::encode("CERTIFICATE", &earlier.ca_chain_der[0]).as_bytes(),
    )
    .expect("stage the chain");
    paths::create_protected_file(&dir.join(ROTATION_MARKER), b"1").expect("the marker");
    std::fs::rename(staged(&dir, KEY_FILE), dir.join(KEY_FILE)).expect("move the key");

    let next = Rotated {
        key_pem: "next key".to_owned(),
        certificate_der: vec![9],
        ca_chain_der: vec![vec![8]],
    };
    install_from_sample(&dir, &next).expect("the rotation must install");

    let key = std::fs::read_to_string(dir.join(KEY_FILE)).expect("the key");
    let cert = std::fs::read_to_string(dir.join(CERT_FILE)).expect("the certificate");
    assert_eq!(key, "next key");
    assert_eq!(cert, pem::encode("CERTIFICATE", &next.certificate_der));
    assert!(!dir.join(ROTATION_MARKER).exists());
    assert!(!staged(&dir, CERT_FILE).exists());
    remove(&dir);
}

/// Another process loads while the rotation has staged only the new key. Without the lock
/// the load drops the staged key, and the rotation installs the new certificate with the old
/// key.
#[test]
fn fr_id_03_a_load_waits_for_a_rotation_that_stages_its_files() {
    let dir = temp_dir("rotation-lock");
    sample().save(&dir).expect("the save must succeed");
    let next = rotated();
    let held = lock(&dir).expect("the rotation takes the lock");
    paths::create_protected_file(&staged(&dir, KEY_FILE), next.key_pem.as_bytes())
        .expect("stage the key");

    let (tx, rx) = std::sync::mpsc::channel();
    let reader_dir = dir.clone();
    let reader = std::thread::spawn(move || {
        let _ = tx.send(StoredIdentity::load(&reader_dir));
    });
    assert!(
        matches!(
            rx.recv_timeout(std::time::Duration::from_millis(500)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ),
        "the load must wait for the rotation"
    );
    assert!(staged(&dir, KEY_FILE).exists());

    paths::create_protected_file(
        &staged(&dir, CERT_FILE),
        pem::encode("CERTIFICATE", &next.certificate_der).as_bytes(),
    )
    .expect("stage the certificate");
    let chain: String = next
        .ca_chain_der
        .iter()
        .map(|der| pem::encode("CERTIFICATE", der))
        .collect();
    paths::create_protected_file(&staged(&dir, CA_CHAIN_FILE), chain.as_bytes())
        .expect("stage the chain");
    paths::create_protected_file(&dir.join(ROTATION_MARKER), b"1").expect("the marker");
    finish_rotation(&dir).expect("the rotation must finish");
    drop(held);

    let loaded = rx
        .recv()
        .expect("the reader must answer")
        .expect("the identity must load");
    reader.join().expect("the reader must not panic");
    assert_eq!(loaded.key_pem, next.key_pem);
    assert_eq!(loaded.certificate_der, next.certificate_der);
    let stored = StoredIdentity::load(&dir).expect("the identity must load");
    assert_eq!(stored.key_pem, next.key_pem);
    assert_eq!(stored.certificate_der, next.certificate_der);
    remove(&dir);
}

fn assert_waits<T>(rx: &std::sync::mpsc::Receiver<T>, what: &str) {
    assert!(
        matches!(
            rx.recv_timeout(Duration::from_millis(500)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ),
        "{what} must wait for the lock"
    );
}

#[test]
fn fr_id_03_a_rotation_install_waits_for_the_lock() {
    let dir = temp_dir("rotation-install-lock");
    sample().save(&dir).expect("the save must succeed");
    let held = lock(&dir).expect("cannot take the lock");

    let (tx, rx) = std::sync::mpsc::channel();
    let rotation_dir = dir.clone();
    let rotation = std::thread::spawn(move || {
        let _ = tx.send(install_from_sample(&rotation_dir, &rotated()));
    });
    assert_waits(&rx, "the rotation");
    for name in [KEY_FILE, CERT_FILE, CA_CHAIN_FILE] {
        assert!(
            !staged(&dir, name).exists(),
            "{name} is staged under the lock"
        );
    }
    assert!(!dir.join(ROTATION_MARKER).exists());
    drop(held);

    rx.recv()
        .expect("the rotation must answer")
        .expect("the rotation must install");
    rotation.join().expect("the rotation must not panic");
    let loaded = StoredIdentity::load(&dir).expect("the identity must load");
    assert_eq!(loaded.certificate_der, rotated().certificate_der);
    assert_eq!(loaded.key_pem, rotated().key_pem);
    remove(&dir);
}

#[test]
fn fr_ag_04_a_save_waits_for_a_rotation_in_progress() {
    let dir = temp_dir("save-lock");
    sample().save(&dir).expect("the save must succeed");
    let held = lock(&dir).expect("the rotation takes the lock");
    paths::create_protected_file(&staged(&dir, KEY_FILE), b"rotated key").expect("stage the key");

    let (tx, rx) = std::sync::mpsc::channel();
    let save_dir = dir.clone();
    let save = std::thread::spawn(move || {
        let mut new = sample();
        new.agent_id = "new-agent".to_owned();
        let _ = tx.send(new.save(&save_dir));
    });
    assert_waits(&rx, "the save");
    let metadata: Metadata =
        serde_json::from_slice(&std::fs::read(dir.join(METADATA_FILE)).expect("read"))
            .expect("the metadata");
    assert_eq!(metadata.agent_id, sample().agent_id);
    assert!(staged(&dir, KEY_FILE).exists(), "the directory is swapped");
    drop(held);

    rx.recv()
        .expect("the save must answer")
        .expect("the save must succeed");
    save.join().expect("the save must not panic");
    let loaded = StoredIdentity::load(&dir).expect("the identity must load");
    assert_eq!(loaded.agent_id, "new-agent");
    assert_eq!(loaded.key_pem, sample().key_pem);
    remove(&dir);
}

#[test]
fn fr_id_03_a_lock_that_another_process_keeps_fails_and_names_the_file() {
    let dir = temp_dir("lock-timeout");
    sample().save(&dir).expect("the save must succeed");
    let held = lock(&dir).expect("cannot take the lock");
    let error =
        lock_within(&dir, Duration::from_millis(200)).expect_err("a held lock must time out");
    let text = format!("{error:#}");
    assert!(
        text.contains(&lock_file(&dir).display().to_string()),
        "{text}"
    );
    assert!(
        error
            .chain()
            .find_map(|cause| cause.downcast_ref::<std::io::Error>())
            .is_some_and(|io| io.kind() == std::io::ErrorKind::TimedOut),
        "{text}"
    );
    drop(held);
    remove(&dir);
}

#[test]
fn fr_ag_04_a_save_after_an_interrupted_swap_waits_for_the_lock() {
    let dir = temp_dir("save-backup-lock");
    sample().save(&dir).expect("the save must succeed");
    let backup = sibling(&dir, ".old");
    std::fs::rename(&dir, &backup).expect("cannot move the identity to the backup");
    let held = lock(&dir).expect("cannot take the lock");

    let (tx, rx) = std::sync::mpsc::channel();
    let save_dir = dir.clone();
    let save = std::thread::spawn(move || {
        let mut new = sample();
        new.agent_id = "new-agent".to_owned();
        let _ = tx.send(new.save(&save_dir));
    });
    assert_waits(&rx, "the save");
    assert!(!dir.exists(), "the save must not swap under the lock");
    assert!(backup.join(METADATA_FILE).exists());
    drop(held);

    rx.recv()
        .expect("the save must answer")
        .expect("the save must succeed");
    save.join().expect("the save must not panic");
    let loaded = StoredIdentity::load(&dir).expect("the identity must load");
    assert_eq!(loaded.agent_id, "new-agent");
    remove(&dir);
}

#[cfg(unix)]
#[test]
fn fr_id_03_a_lock_taken_during_repeated_saves_never_finds_the_directory_missing() {
    let dir = temp_dir("lock-during-saves");
    sample().save(&dir).expect("the save must succeed");
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let saver_dir = dir.clone();
    let saver_stop = std::sync::Arc::clone(&stop);
    let saver = std::thread::spawn(move || {
        while !saver_stop.load(std::sync::atomic::Ordering::Relaxed) {
            sample().save(&saver_dir).expect("the save must succeed");
        }
    });
    for _ in 0..200 {
        lock_within(&dir, Duration::from_secs(10)).expect("the lock must be taken");
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    saver.join().expect("the saver must not panic");
    remove(&dir);
}

/// The holder keeps swapping in a new directory with a held lock, so the waiter never holds
/// a lock that is still in place.
#[cfg(unix)]
#[test]
fn fr_id_03_a_waiter_that_keeps_losing_the_swapped_lock_stops_at_its_deadline() {
    let dir = temp_dir("lock-chase");
    sample().save(&dir).expect("the save must succeed");
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let swapper_dir = dir.clone();
    let swapper_stop = std::sync::Arc::clone(&stop);
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let swapper = std::thread::spawn(move || {
        // A save moves the directory to the backup name, and a waiter relies on it to tell a swap
        // from a missing directory.
        let old = sibling(&swapper_dir, ".old");
        let mut held = lock(&swapper_dir).expect("cannot take the lock");
        let _ = ready_tx.send(());
        while !swapper_stop.load(std::sync::atomic::Ordering::Relaxed) {
            let _ = std::fs::remove_dir_all(&old);
            std::fs::rename(&swapper_dir, &old).expect("cannot move the directory");
            paths::ensure_config_dir(&swapper_dir).expect("cannot create the directory");
            held = lock(&swapper_dir).expect("cannot take the new lock");
            std::thread::sleep(Duration::from_millis(5));
        }
        drop(held);
        let _ = std::fs::remove_dir_all(&old);
    });
    ready_rx.recv().expect("the swapper must start");

    let started = Instant::now();
    let error = lock_within(&dir, Duration::from_millis(300))
        .expect_err("a lock that keeps moving must time out");
    let elapsed = started.elapsed();
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    swapper.join().expect("the swapper must not panic");
    assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
    assert!(
        error
            .chain()
            .find_map(|cause| cause.downcast_ref::<std::io::Error>())
            .is_some_and(|io| io.kind() == std::io::ErrorKind::TimedOut),
        "{error:#}"
    );
    remove(&dir);
}

#[cfg(unix)]
#[test]
fn fr_id_03_a_waiter_takes_the_lock_of_the_directory_that_a_save_swapped_in() {
    let dir = temp_dir("lock-swap");
    sample().save(&dir).expect("the save must succeed");
    let held = lock(&dir).expect("cannot take the lock");

    let (tx, rx) = std::sync::mpsc::channel();
    let waiter_dir = dir.clone();
    let waiter = std::thread::spawn(move || {
        let _ = tx.send(lock(&waiter_dir).map(drop));
    });
    assert_waits(&rx, "the waiter");
    let old = sibling(&dir, ".swapped");
    let _ = std::fs::remove_dir_all(&old);
    std::fs::rename(&dir, &old).expect("cannot move the directory");
    paths::ensure_config_dir(&dir).expect("cannot create the new directory");
    let new = lock(&dir).expect("the new directory has a lock of its own");
    drop(held);
    assert_waits(&rx, "a waiter on the old lock file");
    drop(new);

    rx.recv()
        .expect("the waiter must answer")
        .expect("the waiter must take the new lock");
    waiter.join().expect("the waiter must not panic");
    let _ = std::fs::remove_dir_all(&old);
    remove(&dir);
}

#[test]
fn the_generated_key_and_request_are_valid() {
    let (generated, csr_der) = generate_key_and_csr().expect("cannot generate the key");
    assert!(generated.key_pem.contains("PRIVATE KEY"));
    assert!(!generated.public_key.is_empty());
    assert!(!csr_der.is_empty());
}
