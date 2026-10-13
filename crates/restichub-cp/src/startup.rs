//! The control plane start checks (ADR-0023, section Secrets; AC-RP-02).
//!
//! At start the control plane loads the key encryption key (KEK) and verifies the `kek_check` row
//! of the platform tenant. On the first start it creates the row. On every later start it verifies
//! the row, so the control plane refuses to start with an incorrect KEK and shows the cause
//! (AC-RP-02).

use restichub_db::{Database, PLATFORM_TENANT_ID, kek};
use restichub_secrets::{
    CryptoError, KekCheck, KekError, KekId, KekProvider, make_kek_check, verify_kek_check,
};

const KEK_CHECK_LOCK: &str = "restichub.kek_check:";

#[derive(Debug, thiserror::Error)]
pub enum StartupError {
    #[error(transparent)]
    Database(#[from] restichub_db::DbError),
    #[error(transparent)]
    Kek(#[from] KekError),
    #[error(transparent)]
    Crypto(#[from] CryptoError),
    #[error("the stored kek_check row is not valid")]
    BadCheck,
}

/// Verifies the `kek_check` row of the platform tenant, or creates it on the first start.
///
/// The function takes an advisory lock before the read, so two starts cannot create two rows. The
/// function refuses to start when the loaded KEK does not match the stored check: it returns
/// [`KekError::Mismatch`] with the two key IDs. The error message names the cause (AC-RP-02).
pub async fn ensure_kek_check(
    db: &Database,
    provider: &dyn KekProvider,
) -> Result<(), StartupError> {
    let key = provider.current();
    let mut tx = db.begin_tenant(PLATFORM_TENANT_ID).await?;
    // The lock serializes two starts. The read below happens after the lock, so the second start
    // sees the row that the first start created.
    tx.lock(KEK_CHECK_LOCK).await?;
    match kek::read_check(&mut tx).await? {
        Some(row) => {
            let check = KekCheck {
                kek_id: KekId::parse(&row.kek_id)?,
                nonce: row
                    .nonce
                    .as_slice()
                    .try_into()
                    .map_err(|_| StartupError::BadCheck)?,
                ciphertext: row.ciphertext,
            };
            verify_kek_check(key, &check)?;
        }
        None => {
            let check = make_kek_check(key)?;
            kek::insert_check(
                &mut tx,
                check.kek_id.as_str(),
                &check.nonce,
                &check.ciphertext,
            )
            .await?;
        }
    }
    tx.commit().await?;
    Ok(())
}
