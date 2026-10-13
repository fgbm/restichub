//! Embeds the SHA-256 of the bundled restic binary (ADR-0031, FR-AG-11).
//!
//! The packaging CI sets `RESTICHUB_RESTIC_SHA256` after it verifies the restic release against the
//! signed `SHA256SUMS`. A development build leaves the value empty, and the agent then refuses to
//! run restic unless it is built with the `dev-unpinned-restic` feature.

fn main() {
    println!("cargo:rerun-if-env-changed=RESTICHUB_RESTIC_SHA256");
    let sha256 = std::env::var("RESTICHUB_RESTIC_SHA256").unwrap_or_default();
    // A malformed value would embed a hash that never matches, so the build stops here instead.
    if !sha256.is_empty() {
        let valid = sha256.len() == 64
            && sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        assert!(
            valid,
            "RESTICHUB_RESTIC_SHA256 must be 64 lowercase hex characters, got {sha256:?}"
        );
    }
    println!("cargo:rustc-env=RESTICHUB_RESTIC_SHA256={sha256}");
}
