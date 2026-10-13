//! Password hashing (ADR-0023, section Local authentication).
//!
//! The hash is Argon2id with m = 19456 KiB, t = 2 and p = 1, in the PHC string format. The
//! minimum password length is 12 characters. The maximum is 1024 bytes.
//!
//! The verification runs in `spawn_blocking`, because Argon2id uses about 19 MiB of memory and
//! would block the async runtime. A global semaphore limits the number of concurrent hashes, so a
//! flood of sign-in attempts cannot exhaust the memory of the process. When the semaphore is
//! saturated for [`HASH_WAIT`], the caller gets [`PasswordError::Busy`].
//!
//! For an unknown user or tenant, the caller verifies against a fixed dummy hash. The time of a
//! failed sign-in is therefore similar for a known and an unknown user. The dummy hash is built
//! inside the blocking task, never on the async runtime.

use std::sync::OnceLock;
use std::time::Duration;

use argon2::{Algorithm, Argon2, Params, PasswordHasher, PasswordVerifier, Version};
use tokio::sync::Semaphore;

/// The memory cost of Argon2id in 1 KiB blocks (19456 KiB = 19 MiB).
const M_COST: u32 = 19456;
const T_COST: u32 = 2;
const P_COST: u32 = 1;

/// Limit in characters.
pub const MIN_PASSWORD_LEN: usize = 12;
/// Limit in bytes.
pub const MAX_PASSWORD_BYTES: usize = 1024;

/// Wait before [`PasswordError::Busy`].
pub const HASH_WAIT: Duration = Duration::from_secs(2);

/// Never a real password.
const DUMMY_PASSWORD: &str = "restichub-dummy-password-not-a-real-secret";

#[derive(Debug, thiserror::Error)]
pub enum PasswordError {
    #[error("the password must hold at least {MIN_PASSWORD_LEN} characters")]
    TooShort,
    #[error("the password must hold at most {MAX_PASSWORD_BYTES} bytes")]
    TooLong,
    #[error("cannot hash the password")]
    Hash,
    #[error("the password task stopped")]
    Task(#[source] tokio::task::JoinError),
    #[error("the password work is busy")]
    Busy,
}

pub fn validate_password(password: &str) -> Result<(), PasswordError> {
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err(PasswordError::TooShort);
    }
    if password.len() > MAX_PASSWORD_BYTES {
        return Err(PasswordError::TooLong);
    }
    Ok(())
}

fn hasher() -> Result<Argon2<'static>, PasswordError> {
    let params = Params::new(M_COST, T_COST, P_COST, None).map_err(|_| PasswordError::Hash)?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}

/// Blocks the caller.
pub fn hash_password(password: &str) -> Result<String, PasswordError> {
    validate_password(password)?;
    let hasher = hasher()?;
    let hash = hasher
        .hash_password(password.as_bytes())
        .map_err(|_| PasswordError::Hash)?;
    Ok(hash.to_string())
}

/// Blocks the caller. `false` means the value is not a valid PHC string.
pub fn verify_password(password: &str, hash: &str) -> bool {
    let Ok(hasher) = hasher() else {
        return false;
    };
    hasher.verify_password(password.as_bytes(), hash).is_ok()
}

/// The value is built once. The caller uses it when the user or the tenant is unknown, so the time
/// of the failure is similar to a wrong password. The function blocks the caller.
pub fn dummy_hash() -> &'static str {
    DUMMY_HASH.get_or_init(|| hash_password(DUMMY_PASSWORD).unwrap_or_default())
}

pub fn warm_dummy() {
    let _ = dummy_hash();
}

static DUMMY_HASH: OnceLock<String> = OnceLock::new();

/// Runs a blocking password task under the hash semaphore.
///
/// The function waits at most [`HASH_WAIT`] for a free slot. It returns [`PasswordError::Busy`]
/// when the wait expires.
async fn with_permit<T, F>(semaphore: &Semaphore, task: F) -> Result<T, PasswordError>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let permit = tokio::time::timeout(HASH_WAIT, semaphore.acquire())
        .await
        .map_err(|_| PasswordError::Busy)?
        .map_err(|_| PasswordError::Busy)?;
    let result = tokio::task::spawn_blocking(task)
        .await
        .map_err(PasswordError::Task);
    drop(permit);
    result
}

pub async fn hash_password_limited(
    semaphore: &Semaphore,
    password: String,
) -> Result<String, PasswordError> {
    with_permit(semaphore, move || hash_password(&password)).await?
}

pub async fn verify_password_limited(
    semaphore: &Semaphore,
    password: String,
    hash: String,
) -> Result<bool, PasswordError> {
    with_permit(semaphore, move || verify_password(&password, &hash)).await
}

/// `false` when the hash is absent; the dummy verification keeps the time similar. The dummy hash is
/// built inside the blocking task.
pub async fn verify_or_dummy(
    semaphore: &Semaphore,
    password: String,
    hash: Option<String>,
) -> Result<bool, PasswordError> {
    match hash {
        Some(hash) => verify_password_limited(semaphore, password, hash).await,
        None => {
            with_permit(semaphore, move || {
                let _ = verify_password(&password, dummy_hash());
            })
            .await?;
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_PASSWORD_BYTES, MIN_PASSWORD_LEN, dummy_hash, hash_password, validate_password,
        verify_password,
    };

    #[test]
    fn the_hash_is_argon2id_with_the_required_parameters() {
        let hash = hash_password("correct horse battery").unwrap_or_default();
        assert!(hash.starts_with("$argon2id$"), "{hash}");
        assert!(hash.contains("m=19456"), "{hash}");
        assert!(hash.contains("t=2"), "{hash}");
        assert!(hash.contains("p=1"), "{hash}");
    }

    #[test]
    fn a_password_round_trips() {
        let hash = hash_password("correct horse battery").unwrap_or_default();
        assert!(verify_password("correct horse battery", &hash));
        assert!(!verify_password("wrong password value", &hash));
    }

    #[test]
    fn a_short_password_is_refused() {
        let short = "a".repeat(MIN_PASSWORD_LEN - 1);
        assert!(validate_password(&short).is_err());
        let long_enough = "a".repeat(MIN_PASSWORD_LEN);
        assert!(validate_password(&long_enough).is_ok());
    }

    #[test]
    fn a_long_password_is_refused() {
        let too_long = "a".repeat(MAX_PASSWORD_BYTES + 1);
        assert!(validate_password(&too_long).is_err());
    }

    #[test]
    fn an_invalid_hash_is_refused() {
        assert!(!verify_password("anything at all", "not-a-phc-string"));
    }

    #[test]
    fn the_dummy_hash_holds_the_required_parameters() {
        assert!(dummy_hash().starts_with("$argon2id$"));
        assert!(!verify_password("wrong password value", dummy_hash()));
    }
}
