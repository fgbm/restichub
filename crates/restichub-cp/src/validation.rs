//! Input validation of the API (issue #18).
//!
//! The functions trim the value and check the length. A caller stores the returned, trimmed value.

use time::{Duration, OffsetDateTime};

/// The largest username length in characters.
pub const MAX_USERNAME: usize = 64;
/// The largest display name length in characters.
pub const MAX_DISPLAY_NAME: usize = 128;
/// The largest tenant name length in characters.
pub const MAX_TENANT_NAME: usize = 63;
/// The largest token name length in characters.
pub const MAX_TOKEN_NAME: usize = 64;
/// The largest lifetime of an API token.
pub const MAX_EXPIRY_DAYS: i64 = 2 * 365;

/// The error of an input validation.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("the value is not valid")]
pub struct ValidationError;

/// Validates a username and returns the trimmed value.
///
/// A username holds 1 to [`MAX_USERNAME`] characters from the set `a-z`, `0-9`, `.`, `_` and `-`.
/// The comparison ignores the letter case.
pub fn validate_username(value: &str) -> Result<String, ValidationError> {
    let value = value.trim();
    let count = value.chars().count();
    if count == 0 || count > MAX_USERNAME {
        return Err(ValidationError);
    }
    if !value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-'))
    {
        return Err(ValidationError);
    }
    Ok(value.to_owned())
}

/// Validates a display name and returns the trimmed value.
pub fn validate_display_name(value: &str) -> Result<String, ValidationError> {
    let value = value.trim();
    let count = value.chars().count();
    if count == 0 || count > MAX_DISPLAY_NAME {
        return Err(ValidationError);
    }
    if value.chars().any(char::is_control) {
        return Err(ValidationError);
    }
    Ok(value.to_owned())
}

/// Validates a tenant name and returns the trimmed value.
///
/// A name holds 1 to [`MAX_TENANT_NAME`] characters from the set `a-z`, `0-9` and `-`. It starts
/// with a letter. The reserved name `platform` is refused (FR-TN-02). The unique index of the
/// `tenants` table makes the name unique and ignores the letter case, so a duplicate gives a
/// conflict at the database.
pub fn validate_tenant_name(value: &str) -> Result<String, ValidationError> {
    let value = value.trim();
    let count = value.chars().count();
    if count == 0 || count > MAX_TENANT_NAME {
        return Err(ValidationError);
    }
    let valid = value.chars().all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
    });
    if !valid || !value.starts_with(|character: char| character.is_ascii_lowercase()) {
        return Err(ValidationError);
    }
    if value.eq_ignore_ascii_case("platform") {
        return Err(ValidationError);
    }
    Ok(value.to_owned())
}

/// Validates a token name and returns the trimmed value.
pub fn validate_token_name(value: &str) -> Result<String, ValidationError> {
    let value = value.trim();
    let count = value.chars().count();
    if count == 0 || count > MAX_TOKEN_NAME {
        return Err(ValidationError);
    }
    if value.chars().any(char::is_control) {
        return Err(ValidationError);
    }
    Ok(value.to_owned())
}

/// Checks that an API token expiry is in the future and within two years.
pub fn validate_expiry(
    expires_at: OffsetDateTime,
    now: OffsetDateTime,
) -> Result<(), ValidationError> {
    if expires_at <= now {
        return Err(ValidationError);
    }
    if expires_at > now + Duration::days(MAX_EXPIRY_DAYS) {
        return Err(ValidationError);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use time::OffsetDateTime;

    use super::{
        MAX_DISPLAY_NAME, MAX_TENANT_NAME, MAX_USERNAME, validate_display_name, validate_expiry,
        validate_tenant_name, validate_token_name, validate_username,
    };

    #[test]
    fn a_username_is_trimmed_and_checked() {
        assert_eq!(validate_username("  alice  ").as_deref(), Ok("alice"));
        assert_eq!(validate_username("Alice-1._x").as_deref(), Ok("Alice-1._x"));
        assert!(validate_username("").is_err());
        assert!(validate_username("with space").is_err());
        assert!(validate_username("semi;colon").is_err());
        assert!(validate_username(&"a".repeat(MAX_USERNAME + 1)).is_err());
        assert!(validate_username(&"a".repeat(MAX_USERNAME)).is_ok());
    }

    #[test]
    fn a_display_name_is_trimmed_and_checked() {
        assert_eq!(validate_display_name("  Alice  ").as_deref(), Ok("Alice"));
        assert!(validate_display_name("   ").is_err());
        assert!(validate_display_name(&"a".repeat(MAX_DISPLAY_NAME + 1)).is_err());
        assert!(validate_display_name("line\nbreak").is_err());
    }

    #[test]
    fn a_tenant_name_is_trimmed_and_checked() {
        assert_eq!(validate_tenant_name("  acme  ").as_deref(), Ok("acme"));
        assert_eq!(validate_tenant_name("acme-1").as_deref(), Ok("acme-1"));
        assert!(validate_tenant_name("").is_err());
        assert!(validate_tenant_name("1acme").is_err());
        assert!(validate_tenant_name("-acme").is_err());
        assert!(validate_tenant_name("Acme").is_err());
        assert!(validate_tenant_name("acme_1").is_err());
        assert!(validate_tenant_name("with space").is_err());
        assert!(validate_tenant_name("platform").is_err());
        assert!(validate_tenant_name("PLATFORM").is_err());
        assert!(validate_tenant_name(&"a".repeat(MAX_TENANT_NAME + 1)).is_err());
        assert!(validate_tenant_name(&"a".repeat(MAX_TENANT_NAME)).is_ok());
    }

    #[test]
    fn a_token_name_is_trimmed_and_checked() {
        assert_eq!(validate_token_name("  ci  ").as_deref(), Ok("ci"));
        assert!(validate_token_name("").is_err());
    }

    #[test]
    fn an_expiry_is_in_the_future_and_within_two_years() {
        let now = OffsetDateTime::now_utc();
        assert!(validate_expiry(now + time::Duration::days(1), now).is_ok());
        assert!(validate_expiry(now - time::Duration::days(1), now).is_err());
        assert!(validate_expiry(now + time::Duration::days(2 * 365 + 1), now).is_err());
    }
}
