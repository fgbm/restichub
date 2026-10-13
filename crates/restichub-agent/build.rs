//! Embeds the SHA-256 of the bundled restic binary (ADR-0031, FR-AG-11).
//!
//! The packaging CI sets `RESTICHUB_RESTIC_SHA256` after it verifies the restic release against the
//! signed `SHA256SUMS`. A development build leaves the value empty, and the agent then refuses to
//! run restic unless it is built with the `dev-unpinned-restic` feature.

fn main() {
    println!("cargo:rerun-if-env-changed=RESTICHUB_RESTIC_SHA256");
    let sha256 = std::env::var("RESTICHUB_RESTIC_SHA256").unwrap_or_default();
    println!("cargo:rustc-env=RESTICHUB_RESTIC_SHA256={sha256}");
}
