//! Session tokens, API tokens, cookies and the CSRF header (ADR-0023).
//!
//! A session token is 256 random bits from the OS CSPRNG, in base64url without padding. The
//! database keeps only the SHA-256 hash. The cookie is `__Host-restichub_session` with HttpOnly,
//! Secure, SameSite=Strict and Path=/. For local development over plain HTTP, the configuration
//! `server.insecure_cookies` drops the `Secure` attribute and the `__Host-` prefix.
//!
//! An API token is `rh_` plus 256 random bits in base64url. The database keeps the SHA-256 hash
//! and the first eight characters after `rh_` as the display prefix.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

pub const SESSION_COOKIE: &str = "__Host-restichub_session";
pub const INSECURE_SESSION_COOKIE: &str = "restichub_session";
pub const CSRF_HEADER: &str = "x-restichub-csrf";
pub const CSRF_VALUE: &str = "1";
pub const API_TOKEN_PREFIX: &str = "rh_";
pub const DISPLAY_PREFIX_LEN: usize = 8;
const TOKEN_BYTES: usize = 32;

#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    #[error("cannot read random bytes")]
    Random(#[source] getrandom::Error),
}

pub fn session_cookie_name(insecure: bool) -> &'static str {
    if insecure {
        INSECURE_SESSION_COOKIE
    } else {
        SESSION_COOKIE
    }
}

pub fn session_cookie(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    let header = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    for pair in header.split(';') {
        let pair = pair.trim();
        if let Some((key, value)) = pair.split_once('=')
            && key.trim() == name
        {
            return Some(value.trim().to_owned());
        }
    }
    None
}

pub fn bearer_token(headers: &axum::http::HeaderMap) -> Option<String> {
    let header = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let (scheme, value) = header.split_once(' ')?;
    if scheme.eq_ignore_ascii_case("bearer") {
        let value = value.trim();
        if value.is_empty() {
            None
        } else {
            Some(value.to_owned())
        }
    } else {
        None
    }
}

/// 32 random bytes in base64url.
pub fn random_token() -> Result<String, TokenError> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(TokenError::Random)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// The value is `rh_<base64url>`; the prefix is the first eight characters after `rh_`.
pub fn random_api_token() -> Result<(String, String), TokenError> {
    let value = random_token()?;
    let prefix: String = value.chars().take(DISPLAY_PREFIX_LEN).collect();
    Ok((format!("{API_TOKEN_PREFIX}{value}"), prefix))
}

pub fn hash_token(value: &str) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hasher.finalize().to_vec()
}

pub fn set_cookie_header(name: &str, value: &str, secure: bool) -> String {
    let secure_attribute = if secure { "; Secure" } else { "" };
    format!("{name}={value}; HttpOnly; SameSite=Strict; Path=/{secure_attribute}")
}

pub fn clear_cookie_header(name: &str, secure: bool) -> String {
    let secure_attribute = if secure { "; Secure" } else { "" };
    format!("{name}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0{secure_attribute}")
}

pub fn is_unsafe_method(method: &axum::http::Method) -> bool {
    matches!(
        *method,
        axum::http::Method::POST
            | axum::http::Method::PUT
            | axum::http::Method::PATCH
            | axum::http::Method::DELETE
    )
}

pub fn csrf_ok(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get(CSRF_HEADER)
        .and_then(|value| value.to_str().ok())
        == Some(CSRF_VALUE)
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue, Method};

    use super::{
        API_TOKEN_PREFIX, CSRF_HEADER, CSRF_VALUE, INSECURE_SESSION_COOKIE, SESSION_COOKIE,
        clear_cookie_header, csrf_ok, hash_token, is_unsafe_method, random_api_token, random_token,
        session_cookie, session_cookie_name, set_cookie_header,
    };

    #[test]
    fn a_session_token_is_random_and_url_safe() {
        let first = random_token().unwrap_or_default();
        let second = random_token().unwrap_or_default();
        assert_ne!(first, second);
        assert_eq!(first.len(), 43, "32 bytes in base64url without padding");
        assert!(
            first
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
    }

    #[test]
    fn an_api_token_has_the_prefix_and_the_display_prefix() {
        let (value, prefix) = match random_api_token() {
            Ok(pair) => pair,
            Err(error) => panic!("cannot build the token: {error}"),
        };
        assert!(value.starts_with(API_TOKEN_PREFIX));
        assert_eq!(
            prefix,
            &value[API_TOKEN_PREFIX.len()..API_TOKEN_PREFIX.len() + 8]
        );
        assert_eq!(prefix.len(), 8);
    }

    #[test]
    fn the_hash_is_stable_and_32_bytes() {
        let hash = hash_token("rh_abc");
        assert_eq!(hash.len(), 32);
        assert_eq!(hash, hash_token("rh_abc"));
        assert_ne!(hash, hash_token("rh_abd"));
    }

    #[test]
    fn a_secure_cookie_holds_the_required_attributes() {
        let value = set_cookie_header(SESSION_COOKIE, "abc", true);
        assert!(value.starts_with("__Host-restichub_session=abc"));
        assert!(value.contains("HttpOnly"));
        assert!(value.contains("Secure"));
        assert!(value.contains("SameSite=Strict"));
        assert!(value.contains("Path=/"));
    }

    #[test]
    fn an_insecure_cookie_drops_secure_and_the_host_prefix() {
        let value = set_cookie_header(INSECURE_SESSION_COOKIE, "abc", false);
        assert!(value.starts_with("restichub_session=abc"));
        assert!(!value.contains("Secure"));
        assert!(value.contains("HttpOnly"));
    }

    #[test]
    fn the_cookie_names_follow_the_mode() {
        assert_eq!(session_cookie_name(false), SESSION_COOKIE);
        assert_eq!(session_cookie_name(true), INSECURE_SESSION_COOKIE);
    }

    #[test]
    fn a_clear_cookie_expires_immediately() {
        let value = clear_cookie_header(SESSION_COOKIE, true);
        assert!(value.contains("Max-Age=0"));
        assert!(value.contains("Secure"));
    }

    #[test]
    fn the_session_cookie_is_read_from_the_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            HeaderValue::from_static("other=1; __Host-restichub_session=value; more=2"),
        );
        assert_eq!(
            session_cookie(&headers, SESSION_COOKIE).as_deref(),
            Some("value")
        );
        assert_eq!(session_cookie(&headers, INSECURE_SESSION_COOKIE), None);
    }

    #[test]
    fn the_csrf_header_is_checked() {
        let mut headers = HeaderMap::new();
        assert!(!csrf_ok(&headers));
        headers.insert(CSRF_HEADER, HeaderValue::from_static(CSRF_VALUE));
        assert!(csrf_ok(&headers));
        headers.insert(CSRF_HEADER, HeaderValue::from_static("0"));
        assert!(!csrf_ok(&headers));
    }

    #[test]
    fn the_unsafe_methods_are_known() {
        assert!(is_unsafe_method(&Method::POST));
        assert!(is_unsafe_method(&Method::PUT));
        assert!(is_unsafe_method(&Method::PATCH));
        assert!(is_unsafe_method(&Method::DELETE));
        assert!(!is_unsafe_method(&Method::GET));
        assert!(!is_unsafe_method(&Method::HEAD));
    }
}
