//! The storage of the installation certificate authority (ADR-0026).
//!
//! The tests cover the key round trip, the database dump (AC-ID-02), the row binding and the row
//! swap (T19), the wrong key encryption key and the concurrent first start.

#![allow(clippy::expect_used)]

mod pki_support;

use restichub_canary_scan::{Canary, Scanner};
use restichub_cp::pki::{CaError, CaService};
use restichub_db::{CaRole, Database, PLATFORM_TENANT_ID};

use pki_support::{
    base64_encode, ca_rows, dump_all, hex, other_kek, provider, scalar_hex, swap_wrapped_keys,
    test_db, unwrap_ca_keys,
};

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
    let Some(db) = test_db().await else {
        return;
    };
    let service = CaService::new(provider());
    let store = service
        .ensure_initialized(db.database())
        .await
        .expect("cannot create the CA");
    assert_eq!(store.issuing().len(), 1);

    // The second start verifies the stored keys, the role and the chain.
    service
        .ensure_initialized(db.database())
        .await
        .expect("the second start must verify the keys");

    let keys = unwrap_ca_keys(db.database(), provider().as_ref()).await;
    assert_eq!(keys.len(), 2, "a root and an issuing CA");
    for (_, der) in &keys {
        rcgen::KeyPair::from_pkcs8_der_and_sign_algo(
            &rustls_pki_types::PrivatePkcs8KeyDer::from(der.as_slice()),
            &rcgen::PKCS_ECDSA_P256_SHA256,
        )
        .expect("the unwrapped key must be a valid key pair");
    }

    db.cleanup().await;
}

#[tokio::test]
async fn ac_id_02_a_database_dump_holds_no_ca_private_key() {
    let Some(db) = test_db().await else {
        return;
    };
    create_ca(db.database()).await;

    // The canary is the PKCS#8 key as hex and base64 and the raw private scalar as hex. A `bytea`
    // column renders as hex in the dump, so the hex forms catch a leaked key.
    let keys = unwrap_ca_keys(db.database(), provider().as_ref()).await;
    assert_eq!(keys.len(), 2, "a root and an issuing CA");
    let mut canaries = Vec::new();
    for (_, der) in &keys {
        canaries.push(Canary::parse(&hex(der)));
        canaries.push(Canary::parse(&base64_encode(der)));
        canaries.push(Canary::parse(&scalar_hex(der)));
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
async fn t19_a_swapped_root_and_issuing_blob_is_refused() {
    let Some(db) = test_db().await else {
        return;
    };
    let service = create_ca(db.database()).await;
    let rows = ca_rows(db.database()).await;
    let root = rows
        .iter()
        .find(|row| row.role == CaRole::Root)
        .expect("the root row is present");
    let issuing = rows
        .iter()
        .find(|row| row.role == CaRole::Issuing)
        .expect("the issuing row is present");

    // The associated data binds a blob to its role, so a root/issuing swap fails the check (T19).
    swap_wrapped_keys(&db, root.id, issuing.id).await;

    let error = service.ensure_initialized(db.database()).await;
    assert!(matches!(error, Err(CaError::Crypto(_))), "{error:?}");

    db.cleanup().await;
}

#[tokio::test]
async fn a_swapped_issuing_blob_is_refused_by_the_binding() {
    let Some(db) = test_db().await else {
        return;
    };
    let service = create_ca(db.database()).await;
    // A rotation leaves two issuing CAs with the same associated data (`issuing`).
    service
        .rotate_issuing_ca(
            db.database(),
            &service
                .load(db.database())
                .await
                .expect("cannot load the CA")
                .expect("the CA is present"),
        )
        .await
        .expect("cannot rotate the issuing CA");
    let issuing: Vec<_> = ca_rows(db.database())
        .await
        .into_iter()
        .filter(|row| row.role == CaRole::Issuing)
        .collect();
    assert_eq!(issuing.len(), 2);
    swap_wrapped_keys(&db, issuing[0].id, issuing[1].id).await;

    // The role in the associated data is the same, so only the key/certificate binding catches it.
    let error = service.ensure_initialized(db.database()).await;
    assert!(
        matches!(error, Err(CaError::KeyMismatch { .. })),
        "{error:?}"
    );

    db.cleanup().await;
}

#[tokio::test]
async fn a_replaced_certificate_is_refused() {
    let Some(db) = test_db().await else {
        return;
    };
    let service = create_ca(db.database()).await;
    let rows = ca_rows(db.database()).await;
    let root = rows
        .iter()
        .find(|row| row.role == CaRole::Root)
        .expect("the root row is present");
    let issuing = rows
        .iter()
        .find(|row| row.role == CaRole::Issuing)
        .expect("the issuing row is present");

    // Replace the issuing certificate with the root certificate: the key no longer matches.
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
    sqlx::query("UPDATE ca_keys SET cert_der = $1 WHERE id = $2")
        .bind(&root.cert_der)
        .bind(issuing.id)
        .execute(&mut *tx)
        .await
        .expect("cannot replace the certificate");
    tx.commit().await.expect("cannot commit the certificate");

    let error = service.ensure_initialized(db.database()).await;
    assert!(
        matches!(error, Err(CaError::KeyMismatch { .. })),
        "{error:?}"
    );

    db.cleanup().await;
}

#[tokio::test]
async fn a_changed_not_after_column_is_ignored() {
    let Some(db) = test_db().await else {
        return;
    };
    let service = create_ca(db.database()).await;
    let rows = ca_rows(db.database()).await;
    let issuing = rows
        .iter()
        .find(|row| row.role == CaRole::Issuing)
        .expect("the issuing row is present");

    // The validity comes from the certificate, so a changed column changes nothing.
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
    sqlx::query("UPDATE ca_keys SET not_after = now() - interval '1 day' WHERE id = $1")
        .bind(issuing.id)
        .execute(&mut *tx)
        .await
        .expect("cannot change the column");
    tx.commit().await.expect("cannot commit the change");

    let store = service
        .load(db.database())
        .await
        .expect("cannot load the CA")
        .expect("the CA is present");
    let current = store.current_issuing().expect("the issuing CA is present");
    assert!(
        current.not_after > time::OffsetDateTime::now_utc(),
        "the service must take the validity from the certificate"
    );

    db.cleanup().await;
}

#[tokio::test]
async fn a_wrong_kek_refuses_the_ca() {
    let Some(db) = test_db().await else {
        return;
    };
    create_ca(db.database()).await;

    // A provider that holds another key cannot unwrap the stored CA keys.
    let other = CaService::new(other_kek());
    let error = other.ensure_initialized(db.database()).await;
    assert!(
        matches!(
            error,
            Err(CaError::Crypto(restichub_secrets::CryptoError::UnknownKek(
                _
            )))
        ),
        "{error:?}"
    );

    db.cleanup().await;
}

#[tokio::test]
async fn two_concurrent_first_starts_create_one_ca() {
    let Some(db) = test_db().await else {
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
    assert_eq!(
        rows.iter().filter(|row| row.role == CaRole::Root).count(),
        1
    );
    assert_eq!(
        rows.iter()
            .filter(|row| row.role == CaRole::Issuing)
            .count(),
        1
    );

    db.cleanup().await;
}
