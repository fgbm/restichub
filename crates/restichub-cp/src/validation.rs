//! Input validation of the API.
//!
//! The functions trim the value and check the length. A caller stores the returned, trimmed value.
//! A failure gives a [`ValidationError`] that names the field of the request and a reason code.
//! Responses echo no user input.

use time::{Duration, OffsetDateTime};

use crate::password::{self, PasswordError};

/// Limit in characters.
pub const MAX_USERNAME: usize = 64;
/// Limit in characters.
pub const MAX_DISPLAY_NAME: usize = 128;
/// Limit in characters.
pub const MAX_TENANT_NAME: usize = 63;
/// Limit in characters.
pub const MAX_TOKEN_NAME: usize = 64;
/// Limit in characters.
pub const MAX_EMAIL: usize = 254;
/// Limit in characters.
pub const MAX_SECRET_NAME: usize = 64;
/// Limit in bytes.
pub const MAX_SECRET_VALUE: usize = 64 * 1024;
/// Limit in days.
pub const MAX_EXPIRY_DAYS: i64 = 2 * 365;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    Empty,
    TooShort,
    TooLong,
    InvalidCharacter,
    Reserved,
    Unknown,
    InvalidFormat,
    InvalidBody,
    InvalidCredentials,
    NotInFuture,
    TooFarInFuture,
}

impl Reason {
    /// In snake_case; stable.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::TooShort => "too_short",
            Self::TooLong => "too_long",
            Self::InvalidCharacter => "invalid_character",
            Self::Reserved => "reserved",
            Self::Unknown => "unknown",
            Self::InvalidFormat => "invalid_format",
            Self::InvalidBody => "invalid_body",
            Self::InvalidCredentials => "invalid_credentials",
            Self::NotInFuture => "not_in_future",
            Self::TooFarInFuture => "too_far_in_future",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the field `{field}` is not valid")]
pub struct ValidationError {
    /// Stable identifier; not user input.
    pub field: &'static str,
    pub reason: Reason,
}

impl ValidationError {
    pub fn new(field: &'static str, reason: Reason) -> Self {
        Self { field, reason }
    }
}

/// Validates a username and returns the trimmed value.
///
/// A username holds 1 to [`MAX_USERNAME`] characters from the set `a-z`, `0-9`, `.`, `_` and `-`.
/// Ignores the letter case.
pub fn validate_username(field: &'static str, value: &str) -> Result<String, ValidationError> {
    let value = value.trim();
    let count = value.chars().count();
    if count == 0 {
        return Err(ValidationError::new(field, Reason::Empty));
    }
    if count > MAX_USERNAME {
        return Err(ValidationError::new(field, Reason::TooLong));
    }
    if !value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-'))
    {
        return Err(ValidationError::new(field, Reason::InvalidCharacter));
    }
    Ok(value.to_owned())
}

/// Validates an email address and returns the lower-case value.
///
/// The value holds one `@`, a non-empty local part and a domain with a dot (ADR-0028). The
/// comparison of the stored value ignores the letter case, so the function lower-cases the result.
pub fn validate_email(field: &'static str, value: &str) -> Result<String, ValidationError> {
    let value = value.trim().to_ascii_lowercase();
    if value.is_empty() {
        return Err(ValidationError::new(field, Reason::Empty));
    }
    if value.len() > MAX_EMAIL {
        return Err(ValidationError::new(field, Reason::TooLong));
    }
    if value
        .chars()
        .any(|character| character.is_whitespace() || character.is_control())
    {
        return Err(ValidationError::new(field, Reason::InvalidCharacter));
    }
    let mut parts = value.split('@');
    let (local, domain) = match (parts.next(), parts.next(), parts.next()) {
        (Some(local), Some(domain), None) if !local.is_empty() && domain.contains('.') => {
            (local, domain)
        }
        _ => return Err(ValidationError::new(field, Reason::InvalidFormat)),
    };
    if local.starts_with('.')
        || local.ends_with('.')
        || domain.starts_with('.')
        || domain.ends_with('.')
    {
        return Err(ValidationError::new(field, Reason::InvalidFormat));
    }
    Ok(value)
}

pub fn validate_display_name(field: &'static str, value: &str) -> Result<String, ValidationError> {
    let value = value.trim();
    let count = value.chars().count();
    if count == 0 {
        return Err(ValidationError::new(field, Reason::Empty));
    }
    if count > MAX_DISPLAY_NAME {
        return Err(ValidationError::new(field, Reason::TooLong));
    }
    if value.chars().any(char::is_control) {
        return Err(ValidationError::new(field, Reason::InvalidCharacter));
    }
    Ok(value.to_owned())
}

/// Validates a tenant name and returns the trimmed value.
///
/// A name holds 1 to [`MAX_TENANT_NAME`] characters from the set `a-z`, `0-9` and `-`. It starts
/// with a letter. The reserved name `platform` is refused (FR-TN-02). The unique index of the
/// `tenants` table makes the name unique and ignores the letter case, so a duplicate gives a
/// conflict at the database.
pub fn validate_tenant_name(field: &'static str, value: &str) -> Result<String, ValidationError> {
    let value = value.trim();
    let count = value.chars().count();
    if count == 0 {
        return Err(ValidationError::new(field, Reason::Empty));
    }
    if count > MAX_TENANT_NAME {
        return Err(ValidationError::new(field, Reason::TooLong));
    }
    let valid = value.chars().all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
    });
    if !valid || !value.starts_with(|character: char| character.is_ascii_lowercase()) {
        return Err(ValidationError::new(field, Reason::InvalidCharacter));
    }
    if value.eq_ignore_ascii_case("platform") {
        return Err(ValidationError::new(field, Reason::Reserved));
    }
    Ok(value.to_owned())
}

pub fn validate_token_name(field: &'static str, value: &str) -> Result<String, ValidationError> {
    let value = value.trim();
    let count = value.chars().count();
    if count == 0 {
        return Err(ValidationError::new(field, Reason::Empty));
    }
    if count > MAX_TOKEN_NAME {
        return Err(ValidationError::new(field, Reason::TooLong));
    }
    if value.chars().any(char::is_control) {
        return Err(ValidationError::new(field, Reason::InvalidCharacter));
    }
    Ok(value.to_owned())
}

pub fn validate_expiry(
    field: &'static str,
    expires_at: OffsetDateTime,
    now: OffsetDateTime,
) -> Result<(), ValidationError> {
    if expires_at <= now {
        return Err(ValidationError::new(field, Reason::NotInFuture));
    }
    if expires_at > now + Duration::days(MAX_EXPIRY_DAYS) {
        return Err(ValidationError::new(field, Reason::TooFarInFuture));
    }
    Ok(())
}

/// Validates a secret name and returns the trimmed value.
///
/// A name holds 1 to [`MAX_SECRET_NAME`] characters from the set `a-z`, `0-9`, `.`, `_` and `-`
/// (FR-RP-05). The unique index of the `secrets` table makes the name unique for the tenant, so a
/// duplicate gives a conflict at the database.
pub fn validate_secret_name(field: &'static str, value: &str) -> Result<String, ValidationError> {
    let value = value.trim();
    let count = value.chars().count();
    if count == 0 {
        return Err(ValidationError::new(field, Reason::Empty));
    }
    if count > MAX_SECRET_NAME {
        return Err(ValidationError::new(field, Reason::TooLong));
    }
    if !value.chars().all(|character| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || matches!(character, '.' | '_' | '-')
    }) {
        return Err(ValidationError::new(field, Reason::InvalidCharacter));
    }
    Ok(value.to_owned())
}

/// Checks that a secret value holds 1 to [`MAX_SECRET_VALUE`] bytes (FR-RP-05).
///
/// The function does not return the value, so a caller cannot log it by accident.
pub fn validate_secret_value(field: &'static str, value: &str) -> Result<(), ValidationError> {
    let length = value.len();
    if length == 0 {
        return Err(ValidationError::new(field, Reason::Empty));
    }
    if length > MAX_SECRET_VALUE {
        return Err(ValidationError::new(field, Reason::TooLong));
    }
    Ok(())
}

pub fn validate_password(field: &'static str, value: &str) -> Result<(), ValidationError> {
    password::validate_password(value).map_err(|error| {
        let reason = match error {
            PasswordError::TooShort => Reason::TooShort,
            PasswordError::TooLong => Reason::TooLong,
            _ => Reason::InvalidCharacter,
        };
        ValidationError::new(field, reason)
    })
}

#[cfg(test)]
mod tests {
    use time::OffsetDateTime;

    use super::{
        MAX_DISPLAY_NAME, MAX_EMAIL, MAX_SECRET_NAME, MAX_SECRET_VALUE, MAX_TENANT_NAME,
        MAX_USERNAME, validate_display_name, validate_email, validate_expiry, validate_secret_name,
        validate_secret_value, validate_tenant_name, validate_token_name, validate_username,
    };

    const FIELD: &str = "value";

    #[test]
    fn a_username_is_trimmed_and_checked() {
        assert_eq!(
            validate_username(FIELD, "  alice  ").as_deref(),
            Ok("alice")
        );
        assert_eq!(
            validate_username(FIELD, "Alice-1._x").as_deref(),
            Ok("Alice-1._x")
        );
        assert!(validate_username(FIELD, "").is_err());
        assert!(validate_username(FIELD, "with space").is_err());
        assert!(validate_username(FIELD, "semi;colon").is_err());
        assert!(validate_username(FIELD, &"a".repeat(MAX_USERNAME + 1)).is_err());
        assert!(validate_username(FIELD, &"a".repeat(MAX_USERNAME)).is_ok());
    }

    #[test]
    fn an_email_is_lower_cased_and_checked() {
        assert_eq!(
            validate_email(FIELD, "  Alice@Example.COM  ").as_deref(),
            Ok("alice@example.com")
        );
        assert!(validate_email(FIELD, "").is_err());
        assert!(validate_email(FIELD, "no-at-sign").is_err());
        assert!(validate_email(FIELD, "a@b").is_err());
        assert!(validate_email(FIELD, "@example.com").is_err());
        assert!(validate_email(FIELD, "a@.com").is_err());
        assert!(validate_email(FIELD, "with space@example.com").is_err());
        assert!(validate_email(FIELD, &format!("a@{}", "b".repeat(MAX_EMAIL))).is_err());
    }

    #[test]
    fn a_display_name_is_trimmed_and_checked() {
        assert_eq!(
            validate_display_name(FIELD, "  Alice  ").as_deref(),
            Ok("Alice")
        );
        assert!(validate_display_name(FIELD, "   ").is_err());
        assert!(validate_display_name(FIELD, &"a".repeat(MAX_DISPLAY_NAME + 1)).is_err());
        assert!(validate_display_name(FIELD, "line\nbreak").is_err());
    }

    #[test]
    fn a_tenant_name_is_trimmed_and_checked() {
        assert_eq!(
            validate_tenant_name(FIELD, "  acme  ").as_deref(),
            Ok("acme")
        );
        assert_eq!(
            validate_tenant_name(FIELD, "acme-1").as_deref(),
            Ok("acme-1")
        );
        assert!(validate_tenant_name(FIELD, "").is_err());
        assert!(validate_tenant_name(FIELD, "1acme").is_err());
        assert!(validate_tenant_name(FIELD, "-acme").is_err());
        assert!(validate_tenant_name(FIELD, "Acme").is_err());
        assert!(validate_tenant_name(FIELD, "acme_1").is_err());
        assert!(validate_tenant_name(FIELD, "with space").is_err());
        assert!(validate_tenant_name(FIELD, "platform").is_err());
        assert!(validate_tenant_name(FIELD, "PLATFORM").is_err());
        assert!(validate_tenant_name(FIELD, &"a".repeat(MAX_TENANT_NAME + 1)).is_err());
        assert!(validate_tenant_name(FIELD, &"a".repeat(MAX_TENANT_NAME)).is_ok());
    }

    #[test]
    fn a_token_name_is_trimmed_and_checked() {
        assert_eq!(validate_token_name(FIELD, "  ci  ").as_deref(), Ok("ci"));
        assert!(validate_token_name(FIELD, "").is_err());
    }

    #[test]
    fn a_secret_name_is_trimmed_and_checked() {
        assert_eq!(
            validate_secret_name(FIELD, "  repo.main  ").as_deref(),
            Ok("repo.main")
        );
        assert_eq!(
            validate_secret_name(FIELD, "s3_key-1").as_deref(),
            Ok("s3_key-1")
        );
        assert!(validate_secret_name(FIELD, "").is_err());
        assert!(validate_secret_name(FIELD, "Upper").is_err());
        assert!(validate_secret_name(FIELD, "with space").is_err());
        assert!(validate_secret_name(FIELD, "semi;colon").is_err());
        assert!(validate_secret_name(FIELD, &"a".repeat(MAX_SECRET_NAME + 1)).is_err());
        assert!(validate_secret_name(FIELD, &"a".repeat(MAX_SECRET_NAME)).is_ok());
    }

    #[test]
    fn a_secret_value_holds_one_to_64_kib() {
        assert!(validate_secret_value(FIELD, "x").is_ok());
        assert!(validate_secret_value(FIELD, &"x".repeat(MAX_SECRET_VALUE)).is_ok());
        assert!(validate_secret_value(FIELD, "").is_err());
        assert!(validate_secret_value(FIELD, &"x".repeat(MAX_SECRET_VALUE + 1)).is_err());
    }

    #[test]
    fn an_expiry_is_in_the_future_and_within_two_years() {
        let now = OffsetDateTime::now_utc();
        assert!(validate_expiry(FIELD, now + time::Duration::days(1), now).is_ok());
        assert!(validate_expiry(FIELD, now - time::Duration::days(1), now).is_err());
        assert!(validate_expiry(FIELD, now + time::Duration::days(2 * 365 + 1), now).is_err());
    }
}
