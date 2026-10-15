//! Certificate rotation with an injected clock (FR-ID-03, AC-ID-03, ADR-0026).
//!
//! The tests call the rotation of the control plane and the check of the real agent directly, so
//! the clock moves by days. The stream level tests with real certificates are in
//! `cert_rotation_stream`.

#![allow(clippy::expect_used)]

mod common;
mod queue_support;
mod rotation_support;

use restichub_agent::rotation as agent_rotation;
use restichub_cp::rotation::{self, PresentedCertificate};
use restichub_db::agents as agent_repo;
use restichub_proto::agent::v1::CertificateRejectReason;
use time::Duration;

use queue_support::{Env, test_queue};
use rotation_support::{agent_of, csr_for_key, csr_with_uri, hex, rotation_audit_count, row};

/// FR-ID-03: an agent that enrolled keeps working past the lifetime of its first certificate,
/// because it rotates at two thirds of each lifetime. Two rotations move the expiry from day 30
/// to day 70, and a request with the first certificate gets nothing.
#[tokio::test]
async fn ac_id_03_an_agent_keeps_working_past_the_lifetime_through_two_rotations() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let enrolled = env.enroll_agent("rotation-host").await;
    let agent = agent_of(&enrolled);
    let first = PresentedCertificate::from_der(&enrolled.certificate_der).expect("a certificate");
    let mut presented = first.clone();
    let mut stored = rotation_support::stored_identity(&enrolled);
    assert_eq!(
        (first.not_after - first.not_before).whole_days(),
        30,
        "the default lifetime is 30 days"
    );

    let mut serials = vec![hex(&first.serial)];
    for round in 1..=2 {
        let due = agent_rotation::rotation_due(presented.not_before, presented.not_after);
        let (pending, csr) = agent_rotation::begin().expect("a key pair");
        let (issued, next) = rotation::issue(&env.state, env.tenant, agent, &presented, &csr, due)
            .await
            .expect("the rotation must be signed");
        let rotated = agent_rotation::accept(&stored, &pending, &issued, due)
            .expect("the agent must accept the certificate");

        // The old certificate stays the enrolled one until the confirmation.
        let before = row(&env, agent).await;
        assert_eq!(
            before.serial,
            *serials.last().expect("a serial"),
            "round {round}"
        );
        assert_eq!(
            before.pending_serial,
            Some(hex(&next.serial)),
            "round {round}"
        );

        assert!(
            rotation::confirm(&env.state, env.tenant, agent, &next.serial, due)
                .await
                .expect("the confirmation"),
            "round {round}"
        );
        let after = row(&env, agent).await;
        assert_eq!(after.serial, hex(&next.serial), "round {round}");
        assert_eq!(after.pending_serial, None, "round {round}");
        assert_eq!(after.expires_at, next.not_after, "round {round}");
        assert!(after.last_rotation.is_some(), "round {round}");

        serials.push(hex(&next.serial));
        stored.certificate_der = rotated.certificate_der;
        stored.ca_chain_der = rotated.ca_chain_der;
        stored.key_pem = rotated.key_pem;
        presented = next;
    }

    // Day 65 is past the first certificate (day 30) and past the second one (day 50).
    let day_65 = first.not_before + Duration::days(65);
    assert!(presented.not_after > day_65);
    let mut tx = env.state.db().begin_tenant(env.tenant).await.expect("a tx");
    let current = serials.last().expect("a serial");
    assert!(
        agent_repo::touch_last_seen_guarded(&mut tx, agent, current, day_65)
            .await
            .expect("the guard"),
        "the newest certificate keeps the agent online"
    );
    assert!(
        !agent_repo::touch_last_seen_guarded(&mut tx, agent, &serials[0], day_65)
            .await
            .expect("the guard"),
        "the first certificate is refused after the rotations"
    );
    drop(tx);

    // The first certificate asks again and gets nothing: its serial is not the enrolled one.
    let due = agent_rotation::rotation_due(first.not_before, first.not_after);
    let (_, csr) = agent_rotation::begin().expect("a key pair");
    assert_eq!(
        rotation::issue(&env.state, env.tenant, agent, &first, &csr, due)
            .await
            .expect_err("the old certificate cannot rotate"),
        CertificateRejectReason::InvalidRequest
    );
    assert_eq!(
        rotation_audit_count(&env).await,
        2,
        "one record for each rotation"
    );
    env.db.cleanup().await;
}

/// FR-ID-03: a request before half of the lifetime, with the old key, with a foreign identity or
/// with garbage gets a typed refusal, and the registry row stays as it was.
#[tokio::test]
async fn fr_id_03_a_bad_rotation_request_is_refused_and_changes_nothing() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let enrolled = env.enroll_agent("refusal-host").await;
    let agent = agent_of(&enrolled);
    let presented = PresentedCertificate::from_der(&enrolled.certificate_der).expect("a cert");
    let due = agent_rotation::rotation_due(presented.not_before, presented.not_after);
    let issue = async |csr: &[u8], now| {
        rotation::issue(&env.state, env.tenant, agent, &presented, csr, now).await
    };

    let (_, fresh) = agent_rotation::begin().expect("a key pair");
    assert_eq!(
        issue(&fresh, presented.not_before + Duration::days(1))
            .await
            .expect_err("too early"),
        CertificateRejectReason::TooEarly
    );
    assert_eq!(
        issue(&csr_for_key(&enrolled.key_pem), due)
            .await
            .expect_err("the same key"),
        CertificateRejectReason::SameKey
    );
    let foreign = format!(
        "restichub://tenant/{}/agent/{}",
        enrolled.tenant_id,
        uuid::Uuid::now_v7()
    );
    assert_eq!(
        issue(&csr_with_uri(&foreign), due)
            .await
            .expect_err("a foreign identity"),
        CertificateRejectReason::InvalidRequest
    );
    assert_eq!(
        issue(b"not a request", due).await.expect_err("garbage"),
        CertificateRejectReason::InvalidRequest
    );
    assert_eq!(
        issue(&vec![0_u8; rotation::MAX_CSR_BYTES + 1], due)
            .await
            .expect_err("an oversized request"),
        CertificateRejectReason::InvalidRequest
    );
    let unchanged = row(&env, agent).await;
    assert_eq!(unchanged.pending_serial, None);
    assert_eq!(unchanged.serial, hex(&presented.serial));
    assert_eq!(rotation_audit_count(&env).await, 0);

    // A confirmation of a serial that no rotation issued changes nothing.
    assert!(
        !rotation::confirm(&env.state, env.tenant, agent, &[1, 2, 3], due)
            .await
            .expect("the confirmation")
    );
    env.db.cleanup().await;
}

/// FR-ID-04, FR-ID-03: a revoked agent gets no certificate, and the revoke drops a pending one.
#[tokio::test]
async fn fr_id_04_a_revoked_agent_gets_no_rotated_certificate() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let enrolled = env.enroll_agent("revoked-host").await;
    let agent = agent_of(&enrolled);
    let presented = PresentedCertificate::from_der(&enrolled.certificate_der).expect("a cert");
    let due = agent_rotation::rotation_due(presented.not_before, presented.not_after);

    let (_, csr) = agent_rotation::begin().expect("a key pair");
    rotation::issue(&env.state, env.tenant, agent, &presented, &csr, due)
        .await
        .expect("the first request is signed");
    assert!(row(&env, agent).await.pending_serial.is_some());

    let (status, _, body) = common::send(
        &env.app,
        common::bearer_request(
            "POST",
            &format!("/api/v1/agents/{agent}/revoke"),
            &env.admin,
            None,
        ),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::NO_CONTENT, "{body}");
    assert_eq!(
        row(&env, agent).await.pending_serial,
        None,
        "the revoke drops it"
    );

    let (_, csr) = agent_rotation::begin().expect("a key pair");
    assert_eq!(
        rotation::issue(&env.state, env.tenant, agent, &presented, &csr, due)
            .await
            .expect_err("revoked"),
        CertificateRejectReason::Revoked
    );
    assert!(
        !rotation::confirm(&env.state, env.tenant, agent, &presented.serial, due)
            .await
            .expect("the confirmation")
    );
    env.db.cleanup().await;
}
