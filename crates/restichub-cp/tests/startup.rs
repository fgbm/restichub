//! AC-RP-02: the control plane refuses to start with an incorrect KEK.
//!
//! The test runs the startup function directly. The first start creates the `kek_check` row, a
//! correct start verifies it and an incorrect key is refused with the cause.

mod common;

use restichub_cp::startup::{StartupError, ensure_kek_check};
use restichub_db::{PLATFORM_TENANT_ID, kek};
use restichub_secrets::{Kek, KekError, KekId, KekProvider};

/// A key encryption key provider that holds one key. A test uses this type.
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

/// A provider that holds one key, built from a fill byte.
fn provider(fill: u8) -> OneKek {
    let mut bytes = [fill; 32];
    OneKek(Kek::from_bytes(&mut bytes))
}

#[tokio::test]
async fn ac_rp_02_the_cp_refuses_to_start_with_an_incorrect_kek() {
    let Some(db) = common::test_db().await else {
        return;
    };

    let correct = provider(1);
    assert!(ensure_kek_check(db.database(), &correct).await.is_ok());
    let row = {
        let mut tx = db
            .database()
            .begin_tenant(PLATFORM_TENANT_ID)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        let row = kek::read_check(&mut tx)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        tx.commit().await.unwrap_or_else(|error| panic!("{error}"));
        row
    };
    let row = row.unwrap_or_else(|| panic!("the first start must create the row"));
    assert_eq!(row.kek_id, correct.current().id().as_str());

    assert!(ensure_kek_check(db.database(), &provider(1)).await.is_ok());

    // Refuse an incorrect key with the cause.
    let wrong = provider(2);
    let error = ensure_kek_check(db.database(), &wrong).await;
    let message = error
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
    assert!(
        message.contains(correct.current().id().as_str()),
        "{message}"
    );
    assert!(message.contains(wrong.current().id().as_str()), "{message}");
    assert!(message.contains("does not start"), "{message}");

    db.cleanup().await;
}

#[tokio::test]
async fn ac_rp_02_two_concurrent_starts_agree_on_the_kek() {
    let Some(db) = common::test_db().await else {
        return;
    };

    // Two starts with different keys race. The advisory lock serializes them, so exactly one
    // creates the row and the other verifies the row of the first and refuses the mismatch.
    let first = provider(1);
    let second = provider(2);
    let database = db.database();
    let (first_result, second_result) = tokio::join!(
        ensure_kek_check(database, &first),
        ensure_kek_check(database, &second)
    );

    let oks = [first_result.is_ok(), second_result.is_ok()]
        .iter()
        .filter(|ok| **ok)
        .count();
    assert_eq!(oks, 1, "exactly one start must create the row");

    let error = if first_result.is_err() {
        first_result
    } else {
        second_result
    };
    assert!(
        matches!(error, Err(StartupError::Kek(KekError::Mismatch { .. }))),
        "{error:?}"
    );

    db.cleanup().await;
}
