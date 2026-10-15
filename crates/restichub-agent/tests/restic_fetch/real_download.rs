//! The real download from GitHub. It needs the network, so it runs only on request:
//! `cargo test -p restichub-agent --test restic_fetch -- --ignored`.

use restichub_agent::http_get::HttpsClient;
use restichub_agent::restic_install::Installer;
use restichub_agent::restic_release::{Platform, Source, Supply, release_key};
use restichub_agent::restic_store::{Layout, probe_version};

#[tokio::test]
#[ignore = "needs the network"]
async fn fr_ag_11_the_latest_real_release_downloads_and_verifies() {
    let base = std::env::temp_dir().join(format!("restichub-agent-real-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let installer = Installer {
        layout: Layout::new(base.join("bin"), base.join("config")),
        platform: Platform::host().expect("platform"),
        key: release_key().expect("key"),
        probe: &probe_version,
    };
    let supply = Supply::Download {
        source: Source::github(),
        client: HttpsClient::from_system().expect("client"),
    };
    let record = installer.update(&supply).await.expect("the real download");
    println!(
        "installed restic {} sha256 {}",
        record.version, record.sha256
    );
    installer.layout.verify_installed().expect("intact");
    let _ = std::fs::remove_dir_all(&base);
}
