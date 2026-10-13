//! Embedded web UI.
//!
//! The build output of `ui/build` is embedded at compile time. A build without the UI embeds an
//! empty set and serves a placeholder page instead, so the Rust build never needs Node.
//!
//! Routing. The router registers `/api/*`, `/healthz`, `/readyz` and the OpenAPI document before
//! this fallback, so the UI never shadows them. The fallback serves an embedded file for a path
//! that names one, and `index.html` for every other path (the single-page application fallback).
//!
//! Responses. An immutable asset under `/_app/immutable/` is cached for one year. `index.html` and
//! the fallback are not cached. Every response carries the security headers.

use std::sync::OnceLock;

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

/// `allow_missing` keeps the build working when `ui/build` does not exist. The build script of the
/// release image runs the UI build first, so a released binary always holds the files.
#[derive(RustEmbed)]
#[folder = "../../ui/build"]
#[allow_missing = true]
struct UiAssets;

const INDEX: &str = "index.html";

const PLACEHOLDER: &str = "<!doctype html>\n\
<html lang=\"en\">\n\
<head>\n\
<meta charset=\"utf-8\" />\n\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n\
<title>ResticHub</title>\n\
</head>\n\
<body>\n\
<h1>The web UI was not built</h1>\n\
<p>Run <code>npm ci &amp;&amp; npm run build</code> in <code>ui/</code> and build the control plane again.</p>\n\
</body>\n\
</html>\n";

const IMMUTABLE_PREFIX: &str = "/_app/immutable/";

const IMMUTABLE_CACHE: &str = "public, max-age=31536000, immutable";
const NO_CACHE: &str = "no-cache";

/// The policy of a build without the SvelteKit inline scripts.
///
/// A built UI replaces the `script-src` directive with the hashes of its inline scripts. See
/// [`build_csp`].
const DEFAULT_CSP: &str = "default-src 'self'; img-src 'self' data:; \
style-src 'self'; script-src 'self'; connect-src 'self'; \
frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

/// Serves the embedded UI for one request.
///
/// A path that names an embedded file serves that file. Every other path serves `index.html`.
pub fn serve(uri: Uri) -> Response {
    let request_path = uri.path();
    let (asset_path, fallback) = match embedded_path(request_path) {
        Some(path) => (path, false),
        None if looks_like_asset(request_path) => return StatusCode::NOT_FOUND.into_response(),
        None => (INDEX.to_owned(), true),
    };
    let cache = cache_control(request_path, fallback);
    let body = match UiAssets::get(&asset_path) {
        Some(file) => Body::from(file.data.into_owned()),
        None if asset_path == INDEX => Body::from(PLACEHOLDER.as_bytes().to_vec()),
        None => return StatusCode::NOT_FOUND.into_response(),
    };
    let mut response = Response::new(body);
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(content_type(&asset_path)),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    add_security_headers(headers);
    response
}

fn embedded_path(request_path: &str) -> Option<String> {
    let path = request_path.trim_start_matches('/').trim_end_matches('/');
    if path.is_empty() {
        return None;
    }
    if UiAssets::get(path).is_some() {
        return Some(path.to_owned());
    }
    // A directory route may carry a trailing slash.
    let with_index = format!("{path}/index.html");
    if UiAssets::get(&with_index).is_some() {
        return Some(with_index);
    }
    None
}

/// Returns true when a request path names a file and not a single-page application route.
///
/// A missing build asset never falls back to `index.html`: a wrong `/_app/...` URL and a wrong URL
/// with a file extension answer HTTP 404. A route of the application holds no extension.
fn looks_like_asset(request_path: &str) -> bool {
    let path = request_path.trim_start_matches('/');
    if path.starts_with("_app/") {
        return true;
    }
    path.rsplit('/')
        .next()
        .is_some_and(|segment| segment.contains('.'))
}

fn cache_control(request_path: &str, fallback: bool) -> &'static str {
    if fallback || !request_path.starts_with(IMMUTABLE_PREFIX) {
        NO_CACHE
    } else {
        IMMUTABLE_CACHE
    }
}

fn add_security_headers(headers: &mut HeaderMap) {
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(csp()),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
}

fn csp() -> &'static str {
    static CSP: OnceLock<String> = OnceLock::new();
    CSP.get_or_init(build_csp)
}

/// Builds the content security policy.
///
/// A built UI holds a `<meta http-equiv="content-security-policy">` tag with the SHA-256 hashes of
/// the inline bootstrap scripts (the SvelteKit CSP configuration). The function reuses that policy,
/// so the header and the meta tag agree and the scripts need no `unsafe-inline`. The directives
/// `frame-ancestors` and `base-uri` are dropped from the meta tag, because SvelteKit ignores them
/// for a prerendered page, and appended once. The placeholder page holds no inline script, so the
/// default policy applies.
fn build_csp() -> String {
    let policy = UiAssets::get(INDEX)
        .and_then(|file| meta_csp(&file.data))
        .unwrap_or_else(|| DEFAULT_CSP.to_owned());
    let base = strip_directives(&policy, &["frame-ancestors", "base-uri"]);
    format!("{base}; frame-ancestors 'none'; base-uri 'none'")
}

/// Removes the named directives from a policy.
///
/// The function keeps one copy of each named directive out, so the caller can append it once.
fn strip_directives(policy: &str, names: &[&str]) -> String {
    policy
        .split(';')
        .map(str::trim)
        .filter(|directive| {
            !directive.is_empty()
                && !names
                    .iter()
                    .any(|name| *directive == *name || directive.starts_with(&format!("{name} ")))
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn meta_csp(html: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(html).ok()?;
    let marker = "http-equiv=\"content-security-policy\"";
    let after_marker = &text[text.find(marker)?..];
    let content = &after_marker[after_marker.find("content=\"")? + "content=\"".len()..];
    Some(content[..content.find('"')?].to_owned())
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json" | "map") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("ttf") => "font/ttf",
        Some("txt") => "text/plain; charset=utf-8",
        Some("webmanifest") => "application/manifest+json",
        _ => "application/octet-stream",
    }
}

pub fn asset_paths() -> Vec<String> {
    UiAssets::iter().map(|path| path.into_owned()).collect()
}

#[cfg(test)]
mod tests {
    use super::{
        IMMUTABLE_CACHE, NO_CACHE, cache_control, content_type, looks_like_asset, strip_directives,
    };

    #[test]
    fn an_immutable_asset_is_cached_for_one_year() {
        assert_eq!(
            cache_control("/_app/immutable/entry/start.js", false),
            IMMUTABLE_CACHE
        );
    }

    #[test]
    fn the_entry_point_and_the_fallback_are_not_cached() {
        assert_eq!(cache_control("/index.html", false), NO_CACHE);
        assert_eq!(cache_control("/", false), NO_CACHE);
        assert_eq!(cache_control("/users", true), NO_CACHE);
        // A fallback path that looks immutable is still not cached.
        assert_eq!(cache_control("/_app/immutable/nope.js", true), NO_CACHE);
    }

    #[test]
    fn the_content_types_are_known() {
        assert_eq!(content_type("index.html"), "text/html; charset=utf-8");
        assert_eq!(
            content_type("_app/immutable/entry/start.js"),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(
            content_type("_app/immutable/assets/0.css"),
            "text/css; charset=utf-8"
        );
        assert_eq!(content_type("favicon.svg"), "image/svg+xml");
        assert_eq!(content_type("data.bin"), "application/octet-stream");
    }

    #[test]
    fn a_build_asset_and_an_extension_are_not_application_routes() {
        assert!(looks_like_asset("/_app/immutable/nope.js"));
        assert!(looks_like_asset("/missing.js"));
        assert!(looks_like_asset("/nope.png"));
        assert!(looks_like_asset("//_app/x"));
        assert!(!looks_like_asset("/"));
        assert!(!looks_like_asset("/users"));
        assert!(!looks_like_asset("/users/42"));
        assert!(!looks_like_asset("/platform/tenants"));
    }

    #[test]
    fn the_named_directives_are_removed_once() {
        let policy =
            "default-src 'self'; frame-ancestors 'self'; base-uri 'self'; script-src 'self'";
        assert_eq!(
            strip_directives(policy, &["frame-ancestors", "base-uri"]),
            "default-src 'self'; script-src 'self'"
        );
        // Keep a directive with a similar prefix.
        let other = "default-src 'self'; frame-ancestors-extra 'self'";
        assert_eq!(
            strip_directives(other, &["frame-ancestors"]),
            "default-src 'self'; frame-ancestors-extra 'self'"
        );
    }
}
