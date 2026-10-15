//! Input validation of the API.
//!
//! The functions trim the value and check the length. A caller stores the returned, trimmed value.
//! A failure gives a [`ValidationError`] that names the field of the request and a reason code.
//! Responses echo no user input.

use time::{Duration, OffsetDateTime};

use restichub_proto::validate::has_forbidden;

use crate::password::{self, PasswordError};

/// Limit in characters.
pub const MAX_USERNAME: usize = 64;
/// Limit in characters.
pub const MAX_DISPLAY_NAME: usize = 128;
/// Limit in characters.
pub const MAX_TENANT_NAME: usize = 63;
/// Limit in characters.
pub const MAX_TOKEN_NAME: usize = 64;
/// Limit in characters of one enrollment token label.
pub const MAX_LABEL: usize = 63;
/// Limit in characters.
pub const MAX_EMAIL: usize = 254;
/// Limit in characters.
pub const MAX_SECRET_NAME: usize = 64;
/// Limit in bytes.
pub const MAX_SECRET_VALUE: usize = 64 * 1024;
/// Limit in characters. The derived secret names stay within [`MAX_SECRET_NAME`].
pub const MAX_REPOSITORY_NAME: usize = 48;
/// Limit in characters of a backend endpoint.
pub const MAX_ENDPOINT: usize = 2048;
/// Limit in characters of an S3 bucket name.
pub const MAX_BUCKET: usize = 63;
/// Limit in characters of an S3 key prefix.
pub const MAX_S3_PREFIX: usize = 1024;
/// Limit in characters of a local path.
pub const MAX_PATH: usize = 4096;
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
    /// The operating system does not support the setting (FR-PF-05).
    Unsupported,
    /// The installed restic version lacks the option (AC-AG-09).
    Incompatible,
    /// Not available on the host (FR-HK-10).
    Unavailable,
    /// A confirmation value does not match the object (FR-SC-06).
    Mismatch,
    /// The host did not turn on the trust mode for server scripts (FR-HK-10).
    ServerScriptsDisabled,
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
            Self::Unsupported => "unsupported",
            Self::Incompatible => "incompatible",
            Self::Unavailable => "unavailable",
            Self::Mismatch => "mismatch",
            Self::ServerScriptsDisabled => "server_scripts_disabled",
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

/// A label holds 1 to [`MAX_LABEL`] characters from `a-z`, `0-9` and `-`.
pub fn validate_label(field: &'static str, value: &str) -> Result<String, ValidationError> {
    let value = value.trim();
    let count = value.chars().count();
    if count == 0 {
        return Err(ValidationError::new(field, Reason::Empty));
    }
    if count > MAX_LABEL {
        return Err(ValidationError::new(field, Reason::TooLong));
    }
    if !value.chars().all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
    }) {
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

/// Validates a repository name and returns the trimmed value.
///
/// A name holds 1 to [`MAX_REPOSITORY_NAME`] characters from the set `a-z`, `0-9`, `.`, `_` and
/// `-`. The unique index of the `repositories` table makes the name unique for the tenant, so a
/// duplicate gives a conflict at the database (FR-RP-01).
pub fn validate_repository_name(
    field: &'static str,
    value: &str,
) -> Result<String, ValidationError> {
    let value = value.trim();
    let count = value.chars().count();
    if count == 0 {
        return Err(ValidationError::new(field, Reason::Empty));
    }
    if count > MAX_REPOSITORY_NAME {
        return Err(ValidationError::new(field, Reason::TooLong));
    }
    if has_forbidden(value)
        || !value.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '.' | '_' | '-')
        })
    {
        return Err(ValidationError::new(field, Reason::InvalidCharacter));
    }
    Ok(value.to_owned())
}

/// Validates a backend endpoint and returns it.
///
/// The endpoint is an absolute URL with the scheme `http` or `https` and a host (FR-RP-01). The
/// endpoint holds no user information, no query and no fragment: the rest-server credential comes
/// from a secret field only. A rest-server URL and an S3-compatible endpoint use the same rule.
pub fn validate_endpoint(field: &'static str, value: &str) -> Result<String, ValidationError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ValidationError::new(field, Reason::Empty));
    }
    if value.chars().count() > MAX_ENDPOINT {
        return Err(ValidationError::new(field, Reason::TooLong));
    }
    if has_forbidden(value) {
        return Err(ValidationError::new(field, Reason::InvalidCharacter));
    }
    let uri = value
        .parse::<axum::http::Uri>()
        .map_err(|_| ValidationError::new(field, Reason::InvalidFormat))?;
    let authority = uri
        .authority()
        .ok_or_else(|| ValidationError::new(field, Reason::InvalidFormat))?;
    if !matches!(uri.scheme_str(), Some("http" | "https"))
        || authority.as_str().contains('@')
        || uri.query().is_some()
        || value.contains('#')
    {
        return Err(ValidationError::new(field, Reason::InvalidFormat));
    }
    Ok(value.to_owned())
}

/// Validates an S3 bucket name and returns the trimmed value.
///
/// The rules follow the S3 naming rules: 3 to 63 characters from `a-z`, `0-9`, `.` and `-`, and the
/// first and the last character is a letter or a digit (FR-RP-01).
pub fn validate_bucket(field: &'static str, value: &str) -> Result<String, ValidationError> {
    let value = value.trim();
    let count = value.chars().count();
    if count == 0 {
        return Err(ValidationError::new(field, Reason::Empty));
    }
    if count > MAX_BUCKET {
        return Err(ValidationError::new(field, Reason::TooLong));
    }
    if count < 3 {
        return Err(ValidationError::new(field, Reason::TooShort));
    }
    let allowed = |character: char| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || matches!(character, '.' | '-')
    };
    if has_forbidden(value) || !value.chars().all(allowed) {
        return Err(ValidationError::new(field, Reason::InvalidCharacter));
    }
    let starts_ok = value.starts_with(|character: char| character.is_ascii_alphanumeric());
    let ends_ok = value.ends_with(|character: char| character.is_ascii_alphanumeric());
    if !starts_ok || !ends_ok || value.contains("..") {
        return Err(ValidationError::new(field, Reason::InvalidFormat));
    }
    Ok(value.to_owned())
}

/// Validates an S3 key prefix and returns the trimmed value, or `None` for an empty value.
///
/// A prefix holds no leading slash, no `..` segment, no empty segment and no forbidden character
/// (FR-RP-01).
pub fn validate_s3_prefix(
    field: &'static str,
    value: &str,
) -> Result<Option<String>, ValidationError> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.chars().count() > MAX_S3_PREFIX {
        return Err(ValidationError::new(field, Reason::TooLong));
    }
    if value.starts_with('/')
        || has_forbidden(value)
        || value
            .split('/')
            .any(|segment| segment == ".." || segment.is_empty())
    {
        return Err(ValidationError::new(field, Reason::InvalidFormat));
    }
    Ok(Some(value.to_owned()))
}

/// Validates a local path and returns the trimmed value.
///
/// The path is absolute: it starts with `/` or a drive letter (`C:\`). A UNC path is not accepted.
/// The path holds no `..` segment, no empty segment and no forbidden character (FR-RP-01).
pub fn validate_local_path(field: &'static str, value: &str) -> Result<String, ValidationError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ValidationError::new(field, Reason::Empty));
    }
    if value.chars().count() > MAX_PATH {
        return Err(ValidationError::new(field, Reason::TooLong));
    }
    let bytes = value.as_bytes();
    let drive = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/');
    if !(value.starts_with('/') || drive) {
        return Err(ValidationError::new(field, Reason::InvalidFormat));
    }
    let body = if let Some(rest) = value.strip_prefix('/') {
        rest
    } else {
        &value[3..]
    };
    if has_forbidden(value)
        || body
            .split(['/', '\\'])
            .any(|segment| segment == ".." || segment.is_empty())
    {
        return Err(ValidationError::new(field, Reason::InvalidFormat));
    }
    Ok(value.to_owned())
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
        MAX_BUCKET, MAX_DISPLAY_NAME, MAX_EMAIL, MAX_REPOSITORY_NAME, MAX_SECRET_NAME,
        MAX_SECRET_VALUE, MAX_TENANT_NAME, MAX_USERNAME, validate_bucket, validate_display_name,
        validate_email, validate_endpoint, validate_expiry, validate_local_path,
        validate_repository_name, validate_s3_prefix, validate_secret_name, validate_secret_value,
        validate_tenant_name, validate_token_name, validate_username,
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
    fn a_repository_name_is_trimmed_and_checked() {
        assert_eq!(
            validate_repository_name(FIELD, "  s3-prod.main  ").as_deref(),
            Ok("s3-prod.main")
        );
        assert!(validate_repository_name(FIELD, "").is_err());
        assert!(validate_repository_name(FIELD, "Upper").is_err());
        assert!(validate_repository_name(FIELD, "with space").is_err());
        assert!(validate_repository_name(FIELD, &"a".repeat(MAX_REPOSITORY_NAME)).is_ok());
        assert!(validate_repository_name(FIELD, &"a".repeat(MAX_REPOSITORY_NAME + 1)).is_err());
    }

    #[test]
    fn an_endpoint_is_an_absolute_url() {
        assert_eq!(
            validate_endpoint(FIELD, "  https://s3.example.net:9000  ").as_deref(),
            Ok("https://s3.example.net:9000")
        );
        assert!(validate_endpoint(FIELD, "http://rest.example.net/main").is_ok());
        assert!(validate_endpoint(FIELD, "").is_err());
        assert!(validate_endpoint(FIELD, "s3.example.net").is_err());
        assert!(validate_endpoint(FIELD, "ftp://s3.example.net").is_err());
        assert!(validate_endpoint(FIELD, "https://").is_err());
        // The endpoint holds no user information, no query and no fragment.
        assert!(validate_endpoint(FIELD, "https://u:p@h.example.net").is_err());
        assert!(validate_endpoint(FIELD, "https://h.example.net?x=1").is_err());
        assert!(validate_endpoint(FIELD, "https://h.example.net#frag").is_err());
    }

    #[test]
    fn a_bucket_follows_the_naming_rules() {
        assert_eq!(
            validate_bucket(FIELD, "  backups-prod  ").as_deref(),
            Ok("backups-prod")
        );
        assert!(validate_bucket(FIELD, "backups.prod").is_ok());
        assert!(validate_bucket(FIELD, "ab").is_err());
        assert!(validate_bucket(FIELD, "-backups").is_err());
        assert!(validate_bucket(FIELD, "backups-").is_err());
        assert!(validate_bucket(FIELD, "Backups").is_err());
        assert!(validate_bucket(FIELD, "back..ups").is_err());
        assert!(validate_bucket(FIELD, &"a".repeat(MAX_BUCKET + 1)).is_err());
    }

    #[test]
    fn a_s3_prefix_is_optional_and_has_no_leading_slash() {
        assert_eq!(validate_s3_prefix(FIELD, "   ").ok().flatten(), None);
        assert_eq!(
            validate_s3_prefix(FIELD, " snapshots/prod ").ok().flatten(),
            Some("snapshots/prod".to_owned())
        );
        assert!(validate_s3_prefix(FIELD, "/snapshots").is_err());
        assert!(validate_s3_prefix(FIELD, "a/../b").is_err());
        assert!(validate_s3_prefix(FIELD, "a//b").is_err());
    }

    #[test]
    fn a_local_path_is_absolute() {
        assert_eq!(
            validate_local_path(FIELD, "  /mnt/backup/restic  ").as_deref(),
            Ok("/mnt/backup/restic")
        );
        assert!(validate_local_path(FIELD, "C:\\backup\\restic").is_ok());
        assert!(validate_local_path(FIELD, "").is_err());
        assert!(validate_local_path(FIELD, "relative/path").is_err());
        // A UNC path is not accepted as an absolute path.
        assert!(validate_local_path(FIELD, "\\\\server\\share").is_err());
        // A `..` segment and an empty segment are refused.
        assert!(validate_local_path(FIELD, "/mnt/../etc").is_err());
        assert!(validate_local_path(FIELD, "/mnt//backup").is_err());
        assert!(validate_local_path(FIELD, "C:\\a\\..\\b").is_err());
    }

    #[test]
    fn an_expiry_is_in_the_future_and_within_two_years() {
        let now = OffsetDateTime::now_utc();
        assert!(validate_expiry(FIELD, now + time::Duration::days(1), now).is_ok());
        assert!(validate_expiry(FIELD, now - time::Duration::days(1), now).is_err());
        assert!(validate_expiry(FIELD, now + time::Duration::days(2 * 365 + 1), now).is_err());
    }
}
