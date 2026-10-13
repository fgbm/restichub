//! The issued certificates of the installation certificate authority (ADR-0026).
//!
//! The tests cover the agent and server certificate profile, the serial number, the request policy,
//! the rotation and the path validation with `webpki` and `openssl`.

#![allow(clippy::expect_used)]

mod pki_support;

use std::path::PathBuf;
use std::process::Command;

use rcgen::{
    BasicConstraints, CertificateParams, IsCa, KeyPair, PKCS_ECDSA_P256_SHA256,
    PKCS_ECDSA_P384_SHA384, SanType,
};
use restichub_cp::pki::{CaError, CaService, CaStore, agent_san_uri};
use restichub_db::{CaRole, Database};
use rustls_pki_types::{CertificateDer, UnixTime};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;
use webpki::{EndEntityCert, KeyUsage};
use x509_parser::prelude::GeneralName;

use pki_support::{
    build_csr, certificate_serial, delete_issuing_cas, insert_issuing_ca, openssl_available,
    parse_cert, provider, replace_root_with_expired, test_db, write_pem,
};

async fn create_ca(db: &Database) -> CaService {
    let service = CaService::new(provider());
    service
        .ensure_initialized(db)
        .await
        .expect("cannot create the CA");
    service
}

async fn load(service: &CaService, db: &Database) -> CaStore {
    service
        .load(db)
        .await
        .expect("cannot load the CA")
        .expect("the CA is present")
}

fn verify_chain(leaf: &[u8], chain: &[Vec<u8>], usage: KeyUsage) {
    let leaf_der = CertificateDer::from(leaf.to_vec());
    let end = EndEntityCert::try_from(&leaf_der).expect("the leaf must parse");
    let root_der = CertificateDer::from(chain[1].clone());
    let anchors = [webpki::anchor_from_trusted_cert(&root_der).expect("the anchor must build")];
    let intermediate = [CertificateDer::from(chain[0].clone())];
    end.verify_for_usage(
        webpki::ALL_VERIFICATION_ALGS,
        &anchors,
        &intermediate,
        UnixTime::now(),
        usage,
        None,
        None,
    )
    .expect("the chain must validate");
}

#[tokio::test]
async fn an_issued_agent_certificate_chains_to_the_root_and_has_the_san_and_lifetime() {
    let Some(db) = test_db().await else {
        return;
    };
    let service = create_ca(db.database()).await;
    let store = load(&service, db.database()).await;

    let tenant = Uuid::now_v7();
    let agent = Uuid::now_v7();
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("cannot generate the key");
    let csr = build_csr(CertificateParams::default(), &key);
    let issued = service
        .issue_agent_certificate(&store, tenant, agent, &csr)
        .expect("cannot issue the agent certificate");

    let cert = parse_cert(&issued.cert_der);
    let expected_uri = agent_san_uri(tenant, agent);
    let san = cert
        .subject_alternative_name()
        .expect("cannot read the SAN")
        .expect("the SAN is present");
    assert!(
        san.value
            .general_names
            .iter()
            .any(|name| matches!(name, GeneralName::URI(uri) if *uri == expected_uri.as_str())),
        "the SAN URI is missing: {:?}",
        san.value.general_names
    );
    let cn = cert
        .subject()
        .iter_common_name()
        .next()
        .expect("the common name is present")
        .as_str()
        .expect("the common name is text");
    assert_eq!(cn, agent.to_string());

    let days = (cert.validity().not_after.timestamp() - cert.validity().not_before.timestamp())
        as f64
        / 86_400.0;
    assert!((days - 30.0).abs() < 0.1, "the lifetime is {days} days");

    let usage = cert
        .key_usage()
        .expect("cannot read the key usage")
        .expect("the key usage is present");
    assert!(usage.value.digital_signature());
    let extended = cert
        .extended_key_usage()
        .expect("cannot read the extended key usage")
        .expect("the extended key usage is present");
    assert!(extended.value.client_auth);
    assert!(!extended.value.server_auth);
    assert!(
        cert.basic_constraints()
            .expect("cannot read the basic constraints")
            .is_none_or(|extension| !extension.value.ca)
    );
    assert_eq!(certificate_serial(&issued.cert_der).len(), 16);

    assert_eq!(issued.chain_der.len(), 2);
    let issuing = parse_cert(&issued.chain_der[0]);
    let root = parse_cert(&issued.chain_der[1]);
    cert.verify_signature(Some(issuing.public_key()))
        .expect("the agent certificate must chain to the issuing CA");
    issuing
        .verify_signature(Some(root.public_key()))
        .expect("the issuing CA must chain to the root");
    root.verify_signature(None)
        .expect("the root must be self-signed");

    db.cleanup().await;
}

#[tokio::test]
async fn two_issues_from_the_same_csr_get_different_serials() {
    let Some(db) = test_db().await else {
        return;
    };
    let service = create_ca(db.database()).await;
    let store = load(&service, db.database()).await;

    let tenant = Uuid::now_v7();
    let agent = Uuid::now_v7();
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("cannot generate the key");
    let csr = build_csr(CertificateParams::default(), &key);
    let first = service
        .issue_agent_certificate(&store, tenant, agent, &csr)
        .expect("cannot issue the first certificate");
    let second = service
        .issue_agent_certificate(&store, tenant, agent, &csr)
        .expect("cannot issue the second certificate");

    let first_serial = certificate_serial(&first.cert_der);
    let second_serial = certificate_serial(&second.cert_der);
    assert!(
        first_serial.len() >= 8,
        "the serial must hold at least 64 bits"
    );
    assert_ne!(
        first_serial, second_serial,
        "two certificates from the same key must get different serials"
    );

    db.cleanup().await;
}

#[tokio::test]
async fn an_issued_server_certificate_has_server_auth() {
    let Some(db) = test_db().await else {
        return;
    };
    let service = create_ca(db.database()).await;
    let store = load(&service, db.database()).await;

    let sans = vec!["cp.example".to_owned(), "127.0.0.1".to_owned()];
    let issued = service
        .issue_server_certificate(&store, &sans)
        .expect("cannot issue the server certificate");
    assert!(!issued.key_der.is_empty());
    assert!(format!("{issued:?}").contains("<redacted>"));

    let cert = parse_cert(&issued.cert_der);
    let extended = cert
        .extended_key_usage()
        .expect("cannot read the extended key usage")
        .expect("the extended key usage is present");
    assert!(extended.value.server_auth);
    assert!(!extended.value.client_auth);
    let san = cert
        .subject_alternative_name()
        .expect("cannot read the SAN")
        .expect("the SAN is present");
    assert!(
        san.value
            .general_names
            .iter()
            .any(|name| matches!(name, GeneralName::DNSName(dns) if *dns == "cp.example"))
    );
    assert!(
        san.value
            .general_names
            .iter()
            .any(|name| matches!(name, GeneralName::IPAddress(_)))
    );

    db.cleanup().await;
}

#[tokio::test]
async fn rotation_keeps_both_issuing_cas_valid() {
    let Some(db) = test_db().await else {
        return;
    };
    let service = create_ca(db.database()).await;
    let store = load(&service, db.database()).await;
    let first = store
        .current_issuing()
        .expect("an issuing CA is present")
        .id;

    let rotated = service
        .rotate_issuing_ca(db.database(), &store)
        .await
        .expect("cannot rotate the issuing CA");
    assert_eq!(rotated.issuing().len(), 2);
    let now = OffsetDateTime::now_utc();
    let root = parse_cert(&rotated.root().cert_der);
    for ca in rotated.issuing() {
        assert!(ca.not_after > now, "an issuing CA is already expired");
        let cert = parse_cert(&ca.cert_der);
        cert.verify_signature(Some(root.public_key()))
            .expect("an issuing CA must chain to the root");
        assert!(
            cert.basic_constraints()
                .expect("cannot read the basic constraints")
                .expect("the basic constraints are present")
                .value
                .ca
        );
    }
    let current = rotated.current_issuing().expect("an issuing CA is present");
    assert_ne!(current.id, first, "the new issuing CA must be current");
    assert!(
        rotated.issuing().iter().any(|ca| ca.id == first),
        "the older issuing CA must stay"
    );

    db.cleanup().await;
}

#[tokio::test]
async fn two_concurrent_rotations_create_one_issuing_ca() {
    let Some(db) = test_db().await else {
        return;
    };
    let service = create_ca(db.database()).await;
    let store = load(&service, db.database()).await;

    // The advisory lock serializes the two rotations: the second returns the CA of the first.
    let (first, second) = tokio::join!(
        service.rotate_issuing_ca(db.database(), &store),
        service.rotate_issuing_ca(db.database(), &store)
    );
    assert!(first.is_ok(), "{first:?}");
    assert!(second.is_ok(), "{second:?}");

    let issuing = pki_support::ca_rows(db.database())
        .await
        .into_iter()
        .filter(|row| row.role == CaRole::Issuing)
        .count();
    assert_eq!(issuing, 2, "exactly one rotation must add an issuing CA");

    db.cleanup().await;
}

#[tokio::test]
async fn a_csr_with_extra_sans_or_a_ca_flag_is_refused() {
    let Some(db) = test_db().await else {
        return;
    };
    let service = create_ca(db.database()).await;
    let store = load(&service, db.database()).await;
    let tenant = Uuid::now_v7();
    let agent = Uuid::now_v7();
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("cannot generate the key");

    // A request with a DNS name instead of the identity URI.
    let params =
        CertificateParams::new(vec!["evil.example".to_owned()]).expect("cannot build the CSR");
    let error = service.issue_agent_certificate(&store, tenant, agent, &build_csr(params, &key));
    assert!(matches!(error, Err(CaError::CsrExtraSan)), "{error:?}");

    // Refuse a request that asks for a CA certificate.
    let mut params = CertificateParams::default();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let error = service.issue_agent_certificate(&store, tenant, agent, &build_csr(params, &key));
    assert!(matches!(error, Err(CaError::CsrCaFlag)), "{error:?}");

    // A request that carries the identity URI is accepted.
    let mut params = CertificateParams::default();
    params.subject_alt_names = vec![SanType::URI(
        rcgen::string::Ia5String::try_from(agent_san_uri(tenant, agent)).expect("the URI is valid"),
    )];
    let issued = service.issue_agent_certificate(&store, tenant, agent, &build_csr(params, &key));
    assert!(issued.is_ok(), "{issued:?}");

    db.cleanup().await;
}

#[tokio::test]
async fn a_csr_with_a_p384_key_is_refused() {
    let Some(db) = test_db().await else {
        return;
    };
    let service = create_ca(db.database()).await;
    let store = load(&service, db.database()).await;
    let key = KeyPair::generate_for(&PKCS_ECDSA_P384_SHA384).expect("cannot generate the key");
    let csr = build_csr(CertificateParams::default(), &key);
    let error = service.issue_agent_certificate(&store, Uuid::now_v7(), Uuid::now_v7(), &csr);
    assert!(matches!(error, Err(CaError::CsrKeyAlgorithm)), "{error:?}");

    db.cleanup().await;
}

#[tokio::test]
async fn a_csr_with_an_rsa_key_is_refused() {
    let Some(db) = test_db().await else {
        return;
    };
    let Some(csr) = rsa_csr_der() else {
        println!("skipped: openssl is not on PATH, so the test cannot build an RSA request");
        return;
    };
    let service = create_ca(db.database()).await;
    let store = load(&service, db.database()).await;
    let error = service.issue_agent_certificate(&store, Uuid::now_v7(), Uuid::now_v7(), &csr);
    assert!(
        matches!(
            error,
            Err(CaError::CsrKeyAlgorithm) | Err(CaError::BadCsr(_))
        ),
        "the RSA request must be refused: {error:?}"
    );

    db.cleanup().await;
}

#[tokio::test]
async fn the_chain_validates_with_webpki_for_client_and_server_usage() {
    let Some(db) = test_db().await else {
        return;
    };
    let service = create_ca(db.database()).await;
    let store = load(&service, db.database()).await;

    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("cannot generate the key");
    let csr = build_csr(CertificateParams::default(), &key);
    let agent = service
        .issue_agent_certificate(&store, Uuid::now_v7(), Uuid::now_v7(), &csr)
        .expect("cannot issue the agent certificate");
    verify_chain(&agent.cert_der, &agent.chain_der, KeyUsage::client_auth());

    let server = service
        .issue_server_certificate(&store, &["cp.example".to_owned()])
        .expect("cannot issue the server certificate");
    verify_chain(&server.cert_der, &server.chain_der, KeyUsage::server_auth());

    db.cleanup().await;
}

#[tokio::test]
async fn openssl_verifies_the_issued_chain() {
    if !openssl_available() {
        println!("skipped: openssl is not on PATH");
        return;
    }
    let Some(db) = test_db().await else {
        return;
    };
    let service = create_ca(db.database()).await;
    let store = load(&service, db.database()).await;
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("cannot generate the key");
    let csr = build_csr(CertificateParams::default(), &key);
    let agent = service
        .issue_agent_certificate(&store, Uuid::now_v7(), Uuid::now_v7(), &csr)
        .expect("cannot issue the agent certificate");

    let dir = temp_dir();
    let root = write_pem(&dir, "root.pem", &agent.chain_der[1]);
    let issuing = write_pem(&dir, "issuing.pem", &agent.chain_der[0]);
    let leaf = write_pem(&dir, "leaf.pem", &agent.cert_der);
    let output = Command::new("openssl")
        .args(["verify", "-CAfile"])
        .arg(&root)
        .arg("-untrusted")
        .arg(&issuing)
        .arg(&leaf)
        .output()
        .expect("cannot run openssl");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    // The line reaches the CI log, so the check is visible.
    println!("openssl verify: {}{}", stdout.trim(), stderr.trim());
    assert!(output.status.success(), "openssl verify failed");
    std::fs::remove_dir_all(&dir).ok();

    db.cleanup().await;
}

#[tokio::test]
async fn an_expired_root_refuses_the_start() {
    let Some(db) = test_db().await else {
        return;
    };
    let service = create_ca(db.database()).await;
    replace_root_with_expired(&db, provider().as_ref()).await;

    let error = service.ensure_initialized(db.database()).await;
    assert!(
        matches!(error, Err(CaError::RootExpired { .. })),
        "{error:?}"
    );

    db.cleanup().await;
}

#[tokio::test]
async fn a_near_expiry_issuing_ca_rotates_at_start() {
    let Some(db) = test_db().await else {
        return;
    };
    let service = create_ca(db.database()).await;
    // A newest issuing CA with 30 days left is inside the renewal window.
    insert_issuing_ca(db.database(), provider().as_ref(), Duration::days(30)).await;

    let store = service
        .ensure_initialized(db.database())
        .await
        .expect("cannot restart the CA");
    let issuing = pki_support::ca_rows(db.database())
        .await
        .into_iter()
        .filter(|row| row.role == CaRole::Issuing)
        .count();
    assert_eq!(
        issuing, 3,
        "the start must rotate the near-expiry issuing CA"
    );
    let current = store.current_issuing().expect("an issuing CA is present");
    assert!(
        current.not_after - OffsetDateTime::now_utc() > Duration::days(60),
        "the current issuing CA must be fresh"
    );

    db.cleanup().await;
}

#[tokio::test]
async fn a_root_without_an_issuing_ca_creates_one() {
    let Some(db) = test_db().await else {
        return;
    };
    let service = create_ca(db.database()).await;
    delete_issuing_cas(&db).await;

    service
        .ensure_initialized(db.database())
        .await
        .expect("the start must create the missing issuing CA");
    let issuing = pki_support::ca_rows(db.database())
        .await
        .into_iter()
        .filter(|row| row.role == CaRole::Issuing)
        .count();
    assert_eq!(issuing, 1);

    db.cleanup().await;
}

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("restichub-pki-{}", Uuid::now_v7()));
    std::fs::create_dir_all(&dir).expect("cannot create the temporary directory");
    dir
}

/// Builds an RSA certificate signing request with openssl and returns the DER.
fn rsa_csr_der() -> Option<Vec<u8>> {
    if !openssl_available() {
        return None;
    }
    let dir = temp_dir();
    let key = dir.join("rsa-key.pem");
    let csr = dir.join("rsa.csr.der");
    let status = Command::new("openssl")
        .args([
            "req",
            "-new",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-subj",
            "/CN=agent",
        ])
        .arg("-keyout")
        .arg(&key)
        .arg("-outform")
        .arg("DER")
        .arg("-out")
        .arg(&csr)
        .output()
        .expect("cannot run openssl");
    let der = status
        .status
        .success()
        .then(|| std::fs::read(&csr).expect("cannot read the RSA request"));
    std::fs::remove_dir_all(&dir).ok();
    der
}
