//! The download flow into a protected layout, and the order of the registration (ADR-0038).

use std::path::{Path, PathBuf};

use restichub_agent::cli::RegisterOptions;
use restichub_agent::commands::provision_and_register;
use restichub_agent::http_get::HttpsClient;
use restichub_agent::openpgp::PublicKey;
use restichub_agent::restic_install::Installer;
use restichub_agent::restic_release::{Platform, Source, Supply, VerifiedRelease, hex};
use restichub_agent::restic_store::Layout;
use sha2::{Digest as _, Sha256};

use crate::server::{Reply, TestServer};

const DATA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/signed-release");

fn data(name: &str) -> Vec<u8> {
    std::fs::read(PathBuf::from(DATA).join(name)).expect("fixture")
}

fn test_key() -> PublicKey {
    let armored = String::from_utf8(data("test-key.asc")).expect("key");
    let fingerprint = String::from_utf8(data("test-key.fingerprint")).expect("fingerprint");
    PublicKey::from_armored(&armored, fingerprint.trim()).expect("parse")
}

fn base(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "restichub-agent-fetch-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    dir
}

fn installer(base: &Path) -> Installer<'static> {
    Installer {
        layout: Layout::new(base.join("bin"), base.join("config")),
        platform: Platform::new("linux", "amd64"),
        key: test_key(),
        probe: &|_| Ok("0.19.1".to_owned()),
    }
}

fn release_routes(sums_signature: Vec<u8>) -> impl Fn(&str) -> Reply + Send + Sync + 'static {
    move |path| match path {
        "/api/latest" => Reply::ok(r#"{"tag_name":"v0.19.1"}"#),
        "/dl/v0.19.1/restic_0.19.1_linux_amd64.bz2" => {
            Reply::ok(data("restic_0.19.1_linux_amd64.bz2"))
        }
        "/dl/v0.19.1/SHA256SUMS" => Reply::ok(data("SHA256SUMS")),
        "/dl/v0.19.1/SHA256SUMS.asc" => Reply::ok(sums_signature.clone()),
        _ => Reply::status(404),
    }
}

fn supply(server: &TestServer) -> Supply {
    Supply::Download {
        source: Source::GitHub {
            api_latest: server.url("/api/latest"),
            api_list: server.url("/api/list"),
            download_base: server.url("/dl"),
            pinned: "0.19.1".to_owned(),
        },
        client: HttpsClient::new(server.roots.clone(), None),
    }
}

#[tokio::test]
async fn fr_ag_11_the_latest_release_is_downloaded_verified_and_installed() {
    let dir = base("download");
    let server = TestServer::start(release_routes(data("SHA256SUMS.asc"))).await;
    let installer = installer(&dir);
    let record = installer.update(&supply(&server)).await.expect("install");
    assert_eq!(record.version, "0.19.1");
    installer.layout.verify_installed().expect("intact");
    assert_eq!(
        server.paths(),
        [
            "/api/latest",
            "/dl/v0.19.1/restic_0.19.1_linux_amd64.bz2",
            "/dl/v0.19.1/SHA256SUMS",
            "/dl/v0.19.1/SHA256SUMS.asc",
        ]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn nfr_sec_06_a_download_with_a_bad_signature_installs_nothing() {
    let dir = base("bad-signature");
    let mut broken = data("SHA256SUMS.asc");
    let at = broken.len() / 2;
    broken[at] = if broken[at] == b'A' { b'B' } else { b'A' };
    let server = TestServer::start(release_routes(broken)).await;
    let installer = installer(&dir);
    let error = installer
        .update(&supply(&server))
        .await
        .expect_err("a bad signature");
    assert!(
        format!("{error:#}").to_lowercase().contains("signature"),
        "{error:#}"
    );
    assert!(!installer.layout.binary_path().exists());
    assert!(!installer.layout.has_record());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn fr_ag_11_a_github_rate_limit_stops_the_download() {
    let dir = base("rate-limit");
    let server = TestServer::start(|_| Reply {
        status: 403,
        headers: vec![("x-ratelimit-remaining", "0".to_owned())],
        body: Vec::new(),
    })
    .await;
    let installer = installer(&dir);
    let error = installer
        .update(&supply(&server))
        .await
        .expect_err("rate limit");
    assert!(error.to_string().contains("rate limit"), "{error:#}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The files of the signed 0.19.1 release, plus the API answers that the test gives.
fn fallback_routes(
    latest: &'static str,
    list: &'static str,
) -> impl Fn(&str) -> Reply + Send + Sync + 'static {
    let release = release_routes(data("SHA256SUMS.asc"));
    move |path| match path {
        "/api/latest" if latest.is_empty() => Reply::status(500),
        "/api/latest" => Reply::ok(latest),
        "/api/list" if list.is_empty() => Reply::status(500),
        "/api/list" => Reply::ok(list),
        other => release(other),
    }
}

#[tokio::test]
async fn nfr_cmp_03_a_refused_latest_release_falls_back_to_the_newest_supported_one() {
    let dir = base("fallback-list");
    let server = TestServer::start(fallback_routes(
        r#"{"tag_name":"v1.0.0"}"#,
        r#"[{"tag_name":"v1.0.0"},{"tag_name":"v0.19.1"},{"tag_name":"v0.18.1"}]"#,
    ))
    .await;
    let installer = installer(&dir);
    let record = installer.update(&supply(&server)).await.expect("fallback");
    assert_eq!(record.version, "0.19.1");
    assert!(
        server.paths().iter().all(|path| !path.contains("v1.0.0")),
        "the refused release is never downloaded: {:?}",
        server.paths()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn nfr_cmp_03_the_pinned_release_is_the_last_resort() {
    let dir = base("fallback-pinned");
    let server = TestServer::start(fallback_routes("", "")).await;
    let installer = installer(&dir);
    let record = installer.update(&supply(&server)).await.expect("pinned");
    assert_eq!(record.version, "0.19.1");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn nfr_cmp_03_a_failure_of_every_release_names_the_offline_options() {
    let dir = base("fallback-none");
    let server = TestServer::start(|_| Reply::status(404)).await;
    let installer = installer(&dir);
    let error = installer
        .update(&supply(&server))
        .await
        .expect_err("nothing installs");
    let text = format!("{error:#}");
    assert!(
        text.contains("--restic-archive")
            && text.contains("--restic-sums ")
            && text.contains("--restic-sums-sig"),
        "{text}"
    );
    assert!(!installer.layout.has_record());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn nfr_cmp_03_a_downgrade_needs_the_explicit_flag() {
    let dir = base("downgrade");
    let server = TestServer::start(release_routes(data("SHA256SUMS.asc"))).await;
    let installer = installer(&dir);
    let newer = VerifiedRelease {
        version: "0.20.0".to_owned(),
        binary: b"newer".to_vec(),
        sha256: hex(&Sha256::digest(b"newer")),
    };
    installer
        .layout
        .install(&newer, &|_| Ok("0.20.0".to_owned()))
        .expect("installed");
    let error = installer
        .update(&supply(&server))
        .await
        .expect_err("a downgrade");
    assert!(error.to_string().contains("--allow-downgrade"), "{error:#}");
    assert_eq!(
        installer.layout.read_record().expect("record").version,
        "0.20.0"
    );
    let record = installer
        .update_with(&supply(&server), true)
        .await
        .expect("allowed");
    assert_eq!(record.version, "0.19.1");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A host without restic and without network fails before the enrollment request (FR-IN-07,
/// ADR-0038).
#[tokio::test]
async fn fr_in_07_register_without_restic_and_network_fails_before_the_enrollment() {
    let dir = base("register-order");
    let control_plane = TestServer::start(|_| Reply::status(500)).await;
    let installer = installer(&dir);
    let unreachable = Supply::Download {
        source: Source::GitHub {
            api_latest: "https://localhost:9/api/latest".to_owned(),
            api_list: "https://localhost:9/api/list".to_owned(),
            download_base: "https://localhost:9/dl".to_owned(),
            pinned: "0.19.1".to_owned(),
        },
        client: HttpsClient::new(rustls::RootCertStore::empty(), None),
    };
    let options = RegisterOptions {
        url: Some(control_plane.url("")),
        token: Some("re_test".to_owned()),
        ..RegisterOptions::default()
    };
    let error = provision_and_register(
        &options,
        &dir.join("config"),
        &dir.join("cache"),
        &installer,
        &unreachable,
    )
    .await
    .expect_err("no network");
    assert!(error.to_string().contains("GitHub"), "{error:#}");
    assert!(
        control_plane.paths().is_empty(),
        "the enrollment request must not start"
    );
    assert!(!dir.join("config").join("identity.json").exists());
    let _ = std::fs::remove_dir_all(&dir);
}
