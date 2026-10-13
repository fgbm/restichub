//! The installation certificate authority (ADR-0026).
//!
//! The tests cover the generation and the storage of the CA, the agent and server certificates, the
//! issuing CA rotation, the key encryption, the row swap (T19) and the database dump (AC-ID-02).

#![allow(clippy::expect_used)]

mod common;

use std::sync::Arc;

use rcgen::string::Ia5String;
use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, PKCS_ECDSA_P256_SHA256, SanType};
use restichub_canary_scan::{Canary, Scanner};
use restichub_cp::pki::{CaError, CaService, agent_san_uri};
use restichub_db::ca::{self, CaKeyRow};
use restichub_db::test_util::TestDatabase;
use restichub_db::{Database, PLATFORM_TENANT_ID, TenantId};
use restichub_secrets::{
    CaRole, CryptoError, Kek, KekId, KekProvider, WrappedCaKey, unwrap_ca_key,
};
use rustls_pki_types::PrivatePkcs8KeyDer;
use time::OffsetDateTime;
use uuid::Uuid;
use x509_parser::prelude::{FromDer, GeneralName, X509Certificate};

/// A key encryption key provider that holds one key.
#[derive(Debug)]
struct OneKek(Kek);

impl KekProvider for OneKek {
    fn current(&self) -> &Kek {
        &self.0
    }

    fn by_id(&self, id: &KekId) -> Option<&Kek> {
        (self.0.id() == id).then_some(&self.0)
    }
}

fn other_kek() -> Arc<dyn KekProvider> {
    let mut bytes = [2_u8; 32];
    Arc::new(OneKek(Kek::from_bytes(&mut bytes)))
}

fn provider() -> Arc<dyn KekProvider> {
    common::test_kek()
}

async fn ca_rows(db: &Database) -> Vec<CaKeyRow> {
    let mut tx = db
        .begin_tenant(PLATFORM_TENANT_ID)
        .await
        .expect("cannot begin the transaction");
    let rows = ca::list(&mut tx).await.expect("cannot read the CA rows");
    tx.commit().await.expect("cannot commit the read");
    rows
}

/// Decrypts the stored keys for the canary scan. The result is the plaintext key.
async fn unwrap_ca_keys(db: &Database, provider: &dyn KekProvider) -> Vec<(CaRole, Vec<u8>)> {
    ca_rows(db)
        .await
        .into_iter()
        .map(|row| {
            let role = CaRole::parse(&row.role).expect("the stored role is known");
            let nonce: [u8; 12] = row
                .nonce
                .as_slice()
                .try_into()
                .expect("the nonce is 96 bits");
            let wrapped = WrappedCaKey {
                kek_id: KekId::parse(&row.kek_id).expect("the stored key ID is valid"),
                nonce,
                ciphertext: row.wrapped_key,
            };
            let der = unwrap_ca_key(provider, PLATFORM_TENANT_ID.as_uuid(), role, &wrapped)
                .expect("the KEK must open the stored key");
            (role, der.to_vec())
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// The whole content of the public tables, as `row_to_json` renders it.
async fn dump_all(db: &TestDatabase, tenant: TenantId) -> String {
    let mut tx = db
        .owner_pool()
        .begin()
        .await
        .expect("cannot begin the owner transaction");
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .expect("cannot set the tenant scope");
    sqlx::query("SELECT set_config('restichub.scope', 'platform', true)")
        .execute(&mut *tx)
        .await
        .expect("cannot set the platform scope");
    let tables: Vec<(String,)> = sqlx::query_as(
        "SELECT table_name FROM information_schema.tables \
         WHERE table_schema = 'public' AND table_type = 'BASE TABLE' ORDER BY table_name",
    )
    .fetch_all(&mut *tx)
    .await
    .expect("cannot read the catalog");
    let mut dump = String::new();
    for (table,) in tables {
        let query = format!(
            "SELECT coalesce(string_agg(row_to_json(t)::text, E'\\n'), '') FROM \"{table}\" t"
        );
        let (text,): (String,) = sqlx::query_as(sqlx::AssertSqlSafe(query))
            .fetch_one(&mut *tx)
            .await
            .expect("cannot dump the table");
        dump.push_str(&table);
        dump.push('|');
        dump.push_str(&text);
        dump.push('\n');
    }
    tx.commit().await.expect("cannot commit the dump");
    dump
}

fn csr_der(params: CertificateParams, key: &KeyPair) -> Vec<u8> {
    params
        .serialize_request(key)
        .expect("cannot build the CSR")
        .der()
        .to_vec()
}

async fn create_ca(db: &Database) -> CaService {
    let service = CaService::new(provider());
    service
        .ensure_initialized(db)
        .await
        .expect("cannot create the CA");
    service
}

#[tokio::test]
async fn the_ca_key_round_trips_through_the_store() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let service = CaService::new(provider());
    let store = service
        .ensure_initialized(db.database())
        .await
        .expect("cannot create the CA");
    assert_eq!(store.issuing().len(), 1);

    // The second start verifies the stored keys with the KEK.
    service
        .ensure_initialized(db.database())
        .await
        .expect("the second start must verify the keys");

    let keys = unwrap_ca_keys(db.database(), provider().as_ref()).await;
    assert_eq!(keys.len(), 2, "a root and an issuing CA");
    for (_, der) in &keys {
        KeyPair::from_pkcs8_der_and_sign_algo(
            &PrivatePkcs8KeyDer::from(der.as_slice()),
            &PKCS_ECDSA_P256_SHA256,
        )
        .expect("the unwrapped key must be a valid key pair");
    }

    db.cleanup().await;
}

#[tokio::test]
async fn ac_id_02_a_database_dump_holds_no_ca_private_key() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let service = CaService::new(provider());
    service
        .ensure_initialized(db.database())
        .await
        .expect("cannot create the CA");

    // The plaintext key of each CA is the canary. A `bytea` column renders as hex in the dump, so
    // both the hex and the base64 form of the key are search values.
    let keys = unwrap_ca_keys(db.database(), provider().as_ref()).await;
    assert_eq!(keys.len(), 2, "a root and an issuing CA");
    let mut canaries = Vec::new();
    for (_, der) in &keys {
        canaries.push(Canary::parse(&hex(der)));
        canaries.push(Canary::parse(&base64_encode(der)));
    }

    let dump = dump_all(&db, PLATFORM_TENANT_ID).await;
    assert!(
        dump.contains("wrapped_key"),
        "the dump must reach the ca_keys table"
    );
    let scanner = Scanner::new(&canaries);
    let findings = scanner.scan("dump", dump.as_bytes());
    assert!(
        findings.is_empty(),
        "the dump holds a CA private key: {findings:?}"
    );

    db.cleanup().await;
}

#[tokio::test]
async fn t19_a_swapped_ca_key_blob_is_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let service = CaService::new(provider());
    service
        .ensure_initialized(db.database())
        .await
        .expect("cannot create the CA");
    let rows = ca_rows(db.database()).await;
    let root = rows
        .iter()
        .find(|row| row.role == "root")
        .expect("the root row is present");
    let issuing = rows
        .iter()
        .find(|row| row.role == "issuing")
        .expect("the issuing row is present");

    // Swap the wrapped key of the two rows. The associated data binds each blob to its role, so the
    // store must refuse the swap (T19).
    let mut tx = db
        .owner_pool()
        .begin()
        .await
        .expect("cannot begin the owner transaction");
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(PLATFORM_TENANT_ID.to_string())
        .execute(&mut *tx)
        .await
        .expect("cannot set the tenant scope");
    for (id, source) in [(root.id, issuing), (issuing.id, root)] {
        sqlx::query("UPDATE ca_keys SET wrapped_key = $1, nonce = $2, kek_id = $3 WHERE id = $4")
            .bind(&source.wrapped_key)
            .bind(&source.nonce)
            .bind(&source.kek_id)
            .bind(id)
            .execute(&mut *tx)
            .await
            .expect("cannot swap the row");
    }
    tx.commit().await.expect("cannot commit the swap");

    let error = service.ensure_initialized(db.database()).await;
    assert!(matches!(error, Err(CaError::Crypto(_))), "{error:?}");

    db.cleanup().await;
}

#[tokio::test]
async fn a_wrong_kek_refuses_the_ca() {
    let Some(db) = common::test_db().await else {
        return;
    };
    create_ca(db.database()).await;

    // A provider that holds another key cannot unwrap the stored CA keys.
    let other = CaService::new(other_kek());
    let error = other.ensure_initialized(db.database()).await;
    assert!(
        matches!(error, Err(CaError::Crypto(CryptoError::UnknownKek(_)))),
        "{error:?}"
    );

    db.cleanup().await;
}

#[tokio::test]
async fn two_concurrent_first_starts_create_one_ca() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let service = CaService::new(provider());
    // The advisory lock serializes the two starts, so exactly one creates the CA.
    let (first, second) = tokio::join!(
        service.ensure_initialized(db.database()),
        service.ensure_initialized(db.database())
    );
    assert!(first.is_ok(), "{first:?}");
    assert!(second.is_ok(), "{second:?}");

    let rows = ca_rows(db.database()).await;
    assert_eq!(rows.iter().filter(|row| row.role == "root").count(), 1);
    assert_eq!(rows.iter().filter(|row| row.role == "issuing").count(), 1);

    db.cleanup().await;
}

#[tokio::test]
async fn an_issued_agent_certificate_chains_to_the_root_and_has_the_san_and_lifetime() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let service = create_ca(db.database()).await;
    let store = service
        .load(db.database())
        .await
        .expect("cannot load the CA")
        .expect("the CA is present");

    let tenant = Uuid::now_v7();
    let agent = Uuid::now_v7();
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("cannot generate the key");
    let csr = csr_der(CertificateParams::default(), &key);
    let issued = service
        .issue_agent_certificate(&store, tenant, agent, &csr)
        .expect("cannot issue the agent certificate");

    let (_, cert) =
        X509Certificate::from_der(&issued.cert_der).expect("cannot parse the certificate");
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

    assert_eq!(issued.chain_der.len(), 2);
    let (_, issuing) =
        X509Certificate::from_der(&issued.chain_der[0]).expect("cannot parse the issuer");
    let (_, root) = X509Certificate::from_der(&issued.chain_der[1]).expect("cannot parse the root");
    cert.verify_signature(Some(issuing.public_key()))
        .expect("the agent certificate must chain to the issuing CA");
    issuing
        .verify_signature(Some(root.public_key()))
        .expect("the issuing CA must chain to the root");
    root.verify_signature(None)
        .expect("the root must be self-signed");
    assert!(
        issuing
            .basic_constraints()
            .expect("cannot read the basic constraints")
            .expect("the basic constraints are present")
            .value
            .ca
    );
    assert!(
        root.basic_constraints()
            .expect("cannot read the basic constraints")
            .expect("the basic constraints are present")
            .value
            .ca
    );

    db.cleanup().await;
}

#[tokio::test]
async fn an_issued_server_certificate_has_server_auth() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let service = create_ca(db.database()).await;
    let store = service
        .load(db.database())
        .await
        .expect("cannot load the CA")
        .expect("the CA is present");

    let sans = vec!["cp.example".to_owned(), "127.0.0.1".to_owned()];
    let issued = service
        .issue_server_certificate(&store, &sans)
        .expect("cannot issue the server certificate");
    assert!(!issued.key_der.is_empty());
    assert!(format!("{issued:?}").contains("<redacted>"));

    let (_, cert) =
        X509Certificate::from_der(&issued.cert_der).expect("cannot parse the certificate");
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
    let (_, issuing) =
        X509Certificate::from_der(&issued.chain_der[0]).expect("cannot parse the issuer");
    cert.verify_signature(Some(issuing.public_key()))
        .expect("the server certificate must chain to the issuing CA");

    db.cleanup().await;
}

#[tokio::test]
async fn rotation_keeps_both_issuing_cas_valid() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let service = create_ca(db.database()).await;
    let store = service
        .load(db.database())
        .await
        .expect("cannot load the CA")
        .expect("the CA is present");
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
    let (_, root) =
        X509Certificate::from_der(&rotated.root().cert_der).expect("cannot parse the root");
    for ca in rotated.issuing() {
        assert!(ca.not_after > now, "an issuing CA is already expired");
        let (_, cert) =
            X509Certificate::from_der(&ca.cert_der).expect("cannot parse the issuing CA");
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

    // An agent certificate issued after the rotation uses the new issuing CA.
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("cannot generate the key");
    let csr = csr_der(CertificateParams::default(), &key);
    let issued = service
        .issue_agent_certificate(&rotated, Uuid::now_v7(), Uuid::now_v7(), &csr)
        .expect("cannot issue the agent certificate");
    assert_eq!(issued.chain_der[0], current.cert_der);

    db.cleanup().await;
}

#[tokio::test]
async fn a_csr_with_extra_sans_or_a_ca_flag_is_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let service = create_ca(db.database()).await;
    let store = service
        .load(db.database())
        .await
        .expect("cannot load the CA")
        .expect("the CA is present");
    let tenant = Uuid::now_v7();
    let agent = Uuid::now_v7();
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("cannot generate the key");

    // A request with a DNS name instead of the identity URI.
    let params =
        CertificateParams::new(vec!["evil.example".to_owned()]).expect("cannot build the CSR");
    let error = service.issue_agent_certificate(&store, tenant, agent, &csr_der(params, &key));
    assert!(matches!(error, Err(CaError::CsrExtraSan)), "{error:?}");

    // Refuse a request that asks for a CA certificate.
    let mut params = CertificateParams::default();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let error = service.issue_agent_certificate(&store, tenant, agent, &csr_der(params, &key));
    assert!(matches!(error, Err(CaError::CsrCaFlag)), "{error:?}");

    // A request that carries the identity URI is accepted.
    let mut params = CertificateParams::default();
    params.subject_alt_names = vec![SanType::URI(
        Ia5String::try_from(agent_san_uri(tenant, agent)).expect("the URI is valid"),
    )];
    let issued = service.issue_agent_certificate(&store, tenant, agent, &csr_der(params, &key));
    assert!(issued.is_ok(), "{issued:?}");

    db.cleanup().await;
}
