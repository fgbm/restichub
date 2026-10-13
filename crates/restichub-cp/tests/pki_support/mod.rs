//! Shared helpers for the certificate authority tests (ADR-0026).
//!
//! A helper builds a certificate, reads the `ca_keys` rows, unwraps a stored key or dumps the
//! tables. The tests use the fixed test key encryption key.

#![allow(dead_code)]
#![allow(clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use rcgen::{
    BasicConstraints, CertificateParams, CertificateSigningRequest, DistinguishedName, DnType,
    IsCa, Issuer, KeyPair, KeyUsagePurpose, PKCS_ECDSA_P256_SHA256, SerialNumber,
};
use restichub_db::ca::{self, CaKeyRow, NewCaKey};
use restichub_db::test_util::{TestDatabase, test_database};
use restichub_db::{CaRole, Database, PLATFORM_TENANT_ID, TenantId};
use restichub_secrets::{Kek, KekId, KekProvider, WrappedCaKey, unwrap_ca_key, wrap_ca_key};
use rustls_pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;
use x509_parser::prelude::{FromDer, X509Certificate};

pub async fn test_db() -> Option<TestDatabase> {
    test_database().await
}

/// A key encryption key provider that holds one key.
#[derive(Debug)]
pub struct OneKek(pub Kek);

impl KekProvider for OneKek {
    fn current(&self) -> &Kek {
        &self.0
    }

    fn by_id(&self, id: &KekId) -> Option<&Kek> {
        (self.0.id() == id).then_some(&self.0)
    }
}

fn provider_with(fill: u8) -> Arc<dyn KekProvider> {
    let mut bytes = [fill; 32];
    Arc::new(OneKek(Kek::from_bytes(&mut bytes)))
}

/// The fixed test key, the same as `common::test_kek`.
pub fn provider() -> Arc<dyn KekProvider> {
    provider_with(7)
}

/// A second provider with another key value, for the wrong-KEK test.
pub fn other_kek() -> Arc<dyn KekProvider> {
    provider_with(2)
}

pub async fn ca_rows(db: &Database) -> Vec<CaKeyRow> {
    let mut tx = db
        .begin_tenant(PLATFORM_TENANT_ID)
        .await
        .expect("cannot begin the transaction");
    let rows = ca::list(&mut tx).await.expect("cannot read the CA rows");
    tx.commit().await.expect("cannot commit the read");
    rows
}

pub fn wrapped(row: &CaKeyRow) -> WrappedCaKey {
    WrappedCaKey {
        kek_id: KekId::parse(&row.kek_id).expect("the stored key ID is valid"),
        nonce: row
            .nonce
            .as_slice()
            .try_into()
            .expect("the nonce is 96 bits"),
        ciphertext: row.wrapped_key.clone(),
    }
}

/// Decrypts the stored keys for the canary scan. The result is the plaintext key.
pub async fn unwrap_ca_keys(db: &Database, provider: &dyn KekProvider) -> Vec<(CaRole, Vec<u8>)> {
    ca_rows(db)
        .await
        .into_iter()
        .map(|row| {
            let der = unwrap_ca_key(
                provider,
                PLATFORM_TENANT_ID.as_uuid(),
                row.role.as_str(),
                &wrapped(&row),
            )
            .expect("the KEK must open the stored key");
            (row.role, der.to_vec())
        })
        .collect()
}

pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

pub fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// The hex of the raw P-256 private scalar inside a PKCS#8 document.
///
/// The scalar is the first 32-byte `OCTET STRING` (`04 20`) of the document, before the public key.
pub fn scalar_hex(pkcs8_der: &[u8]) -> String {
    let position = pkcs8_der
        .windows(2)
        .position(|pair| pair == [0x04, 0x20])
        .expect("the PKCS#8 document must hold the 32-byte scalar");
    hex(&pkcs8_der[position + 2..position + 2 + 32])
}

/// The whole content of the public tables, as `row_to_json` renders it.
pub async fn dump_all(db: &TestDatabase, tenant: TenantId) -> String {
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

pub fn build_csr(params: CertificateParams, key: &KeyPair) -> Vec<u8> {
    params
        .serialize_request(key)
        .expect("cannot build the CSR")
        .der()
        .to_vec()
}

pub fn parse_cert(der: &[u8]) -> X509Certificate<'_> {
    X509Certificate::from_der(der)
        .expect("cannot parse the certificate")
        .1
}

pub fn distinguished_name(common_name: &str) -> DistinguishedName {
    let mut name = DistinguishedName::new();
    name.push(DnType::CommonName, common_name);
    name.push(DnType::OrganizationName, "ResticHub");
    name
}

fn random_serial() -> SerialNumber {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).expect("the generator must work");
    bytes[0] &= 0x7f;
    SerialNumber::from_slice(&bytes)
}

/// Reads the root row, unwraps the root key and signs a new issuing CA with a chosen lifetime.
///
/// The test uses the function to build a near-expiry issuing CA.
pub async fn insert_issuing_ca(db: &Database, provider: &dyn KekProvider, lifetime: Duration) {
    let rows = ca_rows(db).await;
    let root = rows
        .iter()
        .find(|row| row.role == CaRole::Root)
        .expect("the root row is present");
    let root_der = unwrap_ca_key(
        provider,
        PLATFORM_TENANT_ID.as_uuid(),
        "root",
        &wrapped(root),
    )
    .expect("the KEK must open the root key");
    let root_key = KeyPair::from_pkcs8_der_and_sign_algo(
        &PrivatePkcs8KeyDer::from(root_der.as_slice()),
        &PKCS_ECDSA_P256_SHA256,
    )
    .expect("the root key must load");
    let root_issuer =
        Issuer::from_ca_cert_der(&CertificateDer::from(root.cert_der.clone()), root_key)
            .expect("the root issuer must build");

    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("cannot generate the key");
    let now = OffsetDateTime::now_utc();
    let mut params = CertificateParams::default();
    params.distinguished_name = distinguished_name("ResticHub Test Issuing CA");
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    params.serial_number = Some(random_serial());
    params.not_before = now;
    params.not_after = now + lifetime;
    let cert = params
        .signed_by(&key, &root_issuer)
        .expect("cannot sign the issuing CA");

    let wrapped = wrap_ca_key(
        provider.current(),
        PLATFORM_TENANT_ID.as_uuid(),
        "issuing",
        key.serialized_der(),
    )
    .expect("cannot wrap the issuing key");
    let mut tx = db
        .begin_tenant(PLATFORM_TENANT_ID)
        .await
        .expect("cannot begin the transaction");
    ca::insert(
        &mut tx,
        &NewCaKey {
            id: Uuid::now_v7(),
            role: CaRole::Issuing,
            kek_id: wrapped.kek_id.as_str(),
            nonce: &wrapped.nonce,
            wrapped_key: &wrapped.ciphertext,
            cert_der: cert.der().as_ref(),
            not_before: now,
            not_after: now + lifetime,
        },
    )
    .await
    .expect("cannot insert the issuing CA");
    tx.commit().await.expect("cannot commit the issuing CA");
}

/// Builds an expired, self-signed root CA and replaces the stored root row with it.
pub async fn replace_root_with_expired(db: &TestDatabase, provider: &dyn KekProvider) {
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("cannot generate the key");
    let now = OffsetDateTime::now_utc();
    let mut params = CertificateParams::default();
    params.distinguished_name = distinguished_name("ResticHub Expired Root CA");
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(1));
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    params.serial_number = Some(random_serial());
    params.not_before = now - Duration::days(3650);
    params.not_after = now - Duration::days(1);
    let cert = params.self_signed(&key).expect("cannot sign the root CA");

    let wrapped = wrap_ca_key(
        provider.current(),
        PLATFORM_TENANT_ID.as_uuid(),
        "root",
        key.serialized_der(),
    )
    .expect("cannot wrap the root key");
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
    // The issuing CAs of the old root cannot chain to the new root, so remove them.
    sqlx::query("DELETE FROM ca_keys WHERE role = 'issuing'")
        .execute(&mut *tx)
        .await
        .expect("cannot remove the issuing CAs");
    sqlx::query(
        "UPDATE ca_keys SET cert_der = $1, wrapped_key = $2, nonce = $3, kek_id = $4 \
         WHERE role = 'root'",
    )
    .bind(cert.der().as_ref())
    .bind(&wrapped.ciphertext)
    .bind(wrapped.nonce.as_slice())
    .bind(wrapped.kek_id.as_str())
    .execute(&mut *tx)
    .await
    .expect("cannot replace the root row");
    tx.commit().await.expect("cannot commit the root row");
}

/// Swaps the wrapped key of two `ca_keys` rows in the owner connection.
pub async fn swap_wrapped_keys(db: &TestDatabase, first: Uuid, second: Uuid) {
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
    for (id, other) in [(first, second), (second, first)] {
        sqlx::query(
            "UPDATE ca_keys SET wrapped_key = \
                 (SELECT wrapped_key FROM ca_keys WHERE id = $1), \
                 nonce = (SELECT nonce FROM ca_keys WHERE id = $1), \
                 kek_id = (SELECT kek_id FROM ca_keys WHERE id = $1) \
             WHERE id = $2",
        )
        .bind(other)
        .bind(id)
        .execute(&mut *tx)
        .await
        .expect("cannot swap the row");
    }
    tx.commit().await.expect("cannot commit the swap");
}

pub async fn delete_issuing_cas(db: &TestDatabase) {
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
    sqlx::query("DELETE FROM ca_keys WHERE role = 'issuing'")
        .execute(&mut *tx)
        .await
        .expect("cannot remove the issuing CAs");
    tx.commit().await.expect("cannot commit the removal");
}

pub fn openssl_available() -> bool {
    Command::new("openssl")
        .arg("version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Writes a DER certificate as a PEM file in `dir` and returns its path.
pub fn write_pem(dir: &Path, name: &str, der: &[u8]) -> PathBuf {
    let body = base64_encode(der);
    let mut text = String::from("-----BEGIN CERTIFICATE-----\n");
    for chunk in body.as_bytes().chunks(64) {
        text.push_str(std::str::from_utf8(chunk).expect("base64 is ASCII"));
        text.push('\n');
    }
    text.push_str("-----END CERTIFICATE-----\n");
    let path = dir.join(name);
    std::fs::write(&path, text).expect("cannot write the PEM file");
    path
}

pub fn certificate_serial(der: &[u8]) -> Vec<u8> {
    parse_cert(der).raw_serial().to_vec()
}

pub fn csr_from(params: CertificateParams, key: &KeyPair) -> CertificateSigningRequest {
    params.serialize_request(key).expect("cannot build the CSR")
}
