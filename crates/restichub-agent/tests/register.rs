//! The `register` command against an in-process control plane (AC-IN-02, FR-IN-07).

#![allow(clippy::expect_used)]

mod common;

use std::path::{Path, PathBuf};

use restichub_agent::cli::RegisterOptions;
use restichub_agent::commands;
use restichub_agent::identity::StoredIdentity;
use restichub_agent::restic::Restic;
use x509_parser::prelude::{FromDer as _, GeneralName, X509Certificate};

/// A restic double for the enrollment tests. The hash check has its own unit tests (AC-AG-11).
struct FakeRestic;

impl Restic for FakeRestic {
    fn verify(&self) -> anyhow::Result<()> {
        Ok(())
    }

    fn version(&self) -> anyhow::Result<String> {
        Ok("0.19.1".to_owned())
    }
}

fn options(fixture: &common::Fixture, force: bool) -> RegisterOptions {
    RegisterOptions {
        url: Some(fixture.server.url.clone()),
        token: Some(fixture.token.clone()),
        ca_file: Some(fixture.server.ca_file()),
        force,
        ..RegisterOptions::default()
    }
}

fn stored_files(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .expect("cannot read the configuration directory")
        .map(|entry| entry.expect("cannot read an entry").path())
        .collect()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

#[tokio::test(flavor = "multi_thread")]
async fn ac_in_02_register_enrolls_the_host_and_stores_no_token() {
    let Some(fixture) = common::fixture("agent-register").await else {
        return;
    };
    let dir = common::temp_config_dir("register-e2e");

    commands::register_async(&options(&fixture, false), &dir, &FakeRestic)
        .await
        .expect("the agent must register");

    let identity = StoredIdentity::load(&dir).expect("the identity must load");
    assert_eq!(identity.tenant_id, fixture.tenant.to_string());
    assert!(!identity.agent_id.is_empty());
    assert_eq!(
        identity.grpc_endpoint.as_deref(),
        Some(common::AGENT_ENDPOINT)
    );
    assert!(!identity.signing_key_update.is_empty());
    assert!(identity.certificate_expiry().is_ok());

    // The certificate carries the SAN URI of the identity (ADR-0026).
    let (_, certificate) =
        X509Certificate::from_der(&identity.certificate_der).expect("the certificate must parse");
    let expected = format!(
        "restichub://tenant/{}/agent/{}",
        identity.tenant_id, identity.agent_id
    );
    let san = certificate
        .subject_alternative_name()
        .expect("the SAN extension")
        .expect("the SAN is present");
    assert!(
        san.value
            .general_names
            .iter()
            .any(|name| matches!(name, GeneralName::URI(uri) if *uri == expected.as_str())),
        "the SAN URI is missing: {:?}",
        san.value.general_names
    );

    // The one-time token is nowhere in the stored files (ADR-0032).
    for path in stored_files(&dir) {
        let bytes = std::fs::read(&path).expect("cannot read a stored file");
        assert!(
            !contains(&bytes, fixture.token.as_bytes()),
            "the token is in {}",
            path.display()
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
    fixture.db.cleanup().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn ac_in_02_a_second_registration_is_refused_without_force() {
    let Some(fixture) = common::fixture("agent-overwrite").await else {
        return;
    };
    let dir = common::temp_config_dir("register-overwrite");

    commands::register_async(&options(&fixture, false), &dir, &FakeRestic)
        .await
        .expect("the first register must succeed");
    let error = commands::register_async(&options(&fixture, false), &dir, &FakeRestic)
        .await
        .expect_err("the second register must be refused");
    assert!(
        error.to_string().contains("already exists"),
        "unexpected error: {error}"
    );

    let _ = std::fs::remove_dir_all(&dir);
    fixture.db.cleanup().await;
}

#[cfg(unix)]
#[test]
fn the_key_file_is_owner_only() {
    let dir = common::temp_config_dir("register-key-mode");
    let identity = sample_identity();
    identity.save(&dir).expect("cannot save the identity");
    use std::os::unix::fs::PermissionsExt as _;
    let mode = std::fs::metadata(dir.join(restichub_agent::identity::KEY_FILE))
        .expect("the key file")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_status_without_an_identity_is_refused() {
    let dir = common::temp_config_dir("register-status-empty");
    assert!(commands::status(&dir).is_err());
}

fn sample_identity() -> StoredIdentity {
    StoredIdentity {
        agent_id: "0192f1e0-0000-7000-8000-000000000001".to_owned(),
        tenant_id: "0192f1e0-0000-7000-8000-000000000002".to_owned(),
        cp_url: "https://cp.example:8443".to_owned(),
        grpc_endpoint: None,
        key_pem: "-----BEGIN PRIVATE KEY-----\nMIIB\n-----END PRIVATE KEY-----\n".to_owned(),
        certificate_der: vec![1, 2, 3],
        ca_chain_der: vec![vec![4, 5]],
        signing_key_update: vec![6, 7, 8],
    }
}
