//! Installs and updates the restic binary (ADR-0038, FR-AG-11).
//!
//! Only `register`, `install-service` and `restic update` call this module. `run` never does, so the
//! service makes no network request for restic.

use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::cli::ResticFileOptions;
use crate::http_get::HttpsClient;
use crate::openpgp::PublicKey;
use crate::restic_release::{Platform, Source, Supply, release_key, verify_release};
use crate::restic_store::{Layout, Record, probe_version};

/// Everything the install needs, so a test can replace the key, the platform and the probe.
pub struct Installer<'a> {
    pub layout: Layout,
    pub platform: Platform,
    pub key: PublicKey,
    pub probe: &'a dyn Fn(&Path) -> Result<String>,
}

impl Installer<'static> {
    /// Uses the paths of this host and the pinned release key (ADR-0038).
    ///
    /// # Errors
    ///
    /// Returns an error on a platform without a restic build.
    pub fn host() -> Result<Self> {
        Ok(Self {
            layout: Layout::host(),
            platform: Platform::host()?,
            key: release_key()?,
            probe: &probe_version,
        })
    }
}

impl Installer<'_> {
    /// Installs restic when no record exists, and otherwise only checks the installed one.
    ///
    /// # Errors
    ///
    /// Returns an error when the download or a verification step fails, or when an installed
    /// binary differs from its record.
    pub async fn ensure(&self, supply: &Supply) -> Result<Record> {
        if self.layout.has_record() {
            let record = self.layout.verify_installed().context(
                "the installed restic is not intact; run `restichub-agent restic update`",
            )?;
            println!("restic {} is installed", record.version);
            return Ok(record);
        }
        self.update(supply).await
    }

    /// Fetches a release, verifies it and swaps it in (FR-AG-11).
    ///
    /// # Errors
    ///
    /// Returns an error when the download or a verification step fails. The old binary stays in
    /// place then.
    pub async fn update(&self, supply: &Supply) -> Result<Record> {
        let files = supply.fetch(self.platform).await?;
        let release = verify_release(&files, self.platform, &self.key)?;
        let record = self.layout.install(&release, self.probe)?;
        println!(
            "restic {} is installed at {}",
            record.version,
            self.layout.binary_path().display()
        );
        Ok(record)
    }
}

/// Picks the supply from the options: the offline files, or the GitHub download.
///
/// # Errors
///
/// Returns an error when only some of the offline options are present, or when the download client
/// cannot start.
pub fn supply(options: &ResticFileOptions) -> Result<Supply> {
    match (&options.archive, &options.sums, &options.sums_signature) {
        (Some(archive), Some(sums), Some(sums_signature)) => Ok(Supply::Files {
            archive: archive.clone(),
            sums: sums.clone(),
            sums_signature: sums_signature.clone(),
        }),
        (None, None, None) => Ok(Supply::Download {
            source: Source::github(),
            client: HttpsClient::from_system()?,
        }),
        _ => bail!("give all of --restic-archive, --restic-sums and --restic-sums-sig, or none"),
    }
}

#[cfg(all(test, unix))]
mod tests {
    #![allow(clippy::expect_used)]

    use std::path::PathBuf;

    use super::*;

    const DATA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/signed-release");

    fn data(name: &str) -> PathBuf {
        PathBuf::from(DATA).join(name)
    }

    fn key(name: &str) -> PublicKey {
        let armored = std::fs::read_to_string(data(&format!("{name}.asc"))).expect("key");
        let fingerprint =
            std::fs::read_to_string(data(&format!("{name}.fingerprint"))).expect("fingerprint");
        PublicKey::from_armored(&armored, fingerprint.trim()).expect("parse")
    }

    fn base(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "restichub-agent-install-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        dir
    }

    fn installer(base: &Path, key: PublicKey) -> Installer<'static> {
        Installer {
            layout: Layout::new(base.join("bin"), base.join("config")),
            platform: Platform::new("linux", "amd64"),
            key,
            probe: &|_| Ok("0.19.1".to_owned()),
        }
    }

    /// Copies the signed archive under `name` into `base`, since the name carries the version.
    fn offline(base: &Path, archive_name: &str) -> Supply {
        let archive = base.join(archive_name);
        std::fs::copy(data("restic_0.19.1_linux_amd64.bz2"), &archive).expect("copy");
        Supply::Files {
            archive,
            sums: data("SHA256SUMS"),
            sums_signature: data("SHA256SUMS.asc"),
        }
    }

    #[tokio::test]
    async fn fr_ag_11_the_offline_files_install_restic() {
        let dir = base("offline");
        let installer = installer(&dir, key("test-key"));
        let supply = offline(&dir, "restic_0.19.1_linux_amd64.bz2");
        let record = installer.update(&supply).await.expect("install");
        assert_eq!(record.version, "0.19.1");
        let installed = std::fs::read_to_string(installer.layout.binary_path()).expect("binary");
        assert!(installed.contains("restic 0.19.1"));
        installer.layout.verify_installed().expect("intact");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn nfr_sec_06_a_signature_from_another_key_is_refused() {
        let dir = base("wrong-key");
        let installer = installer(&dir, key("other-key"));
        let supply = offline(&dir, "restic_0.19.1_linux_amd64.bz2");
        let error = installer.update(&supply).await.expect_err("wrong key");
        assert!(format!("{error:#}").contains("signature"), "{error:#}");
        assert!(!installer.layout.has_record());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn nfr_sec_06_a_changed_sums_file_is_refused() {
        let dir = base("bad-sig");
        let installer = installer(&dir, key("test-key"));
        let mut supply = offline(&dir, "restic_0.19.1_linux_amd64.bz2");
        let sums = dir.join("SHA256SUMS");
        let text = std::fs::read_to_string(data("SHA256SUMS")).expect("sums");
        std::fs::write(
            &sums,
            text.replace("restic_0.19.1_linux", "restic_0.19.2_linux"),
        )
        .expect("write");
        if let Supply::Files { sums: path, .. } = &mut supply {
            *path = sums;
        }
        let error = installer.update(&supply).await.expect_err("changed sums");
        assert!(format!("{error:#}").contains("signature"), "{error:#}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn nfr_sec_06_an_archive_that_differs_from_the_signed_hash_is_refused() {
        let dir = base("hash");
        let installer = installer(&dir, key("test-key"));
        // The signed sums list this name with an all-zero hash.
        let supply = offline(&dir, "restic_0.18.5_linux_amd64.bz2");
        let error = installer.update(&supply).await.expect_err("hash mismatch");
        assert!(
            error
                .to_string()
                .contains("does not match the signed SHA256SUMS"),
            "{error:#}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn nfr_cmp_03_a_version_below_the_minimum_is_refused() {
        let dir = base("old");
        let installer = installer(&dir, key("test-key"));
        let supply = offline(&dir, "restic_0.17.0_linux_amd64.bz2");
        let error = installer.update(&supply).await.expect_err("too old");
        assert!(
            error.to_string().contains("older than the minimum"),
            "{error:#}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn nfr_cmp_03_a_major_version_above_the_tested_one_is_refused() {
        let dir = base("major");
        let installer = installer(&dir, key("test-key"));
        let supply = offline(&dir, "restic_1.0.0_linux_amd64.bz2");
        let error = installer.update(&supply).await.expect_err("too new");
        assert!(
            error.to_string().contains("major version above"),
            "{error:#}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn fr_ag_11_ensure_keeps_an_intact_install_and_refuses_a_changed_one() {
        let dir = base("ensure");
        let installer = installer(&dir, key("test-key"));
        let supply = offline(&dir, "restic_0.19.1_linux_amd64.bz2");
        installer.ensure(&supply).await.expect("first install");
        let before = std::fs::read(installer.layout.binary_path()).expect("binary");
        installer.ensure(&supply).await.expect("second call");
        assert_eq!(
            std::fs::read(installer.layout.binary_path()).expect("binary"),
            before
        );
        std::fs::write(installer.layout.binary_path(), b"evil").expect("tamper");
        let error = installer.ensure(&supply).await.expect_err("changed binary");
        assert!(format!("{error:#}").contains("restic update"), "{error:#}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fr_ag_11_a_partial_set_of_offline_options_is_refused() {
        let options = ResticFileOptions {
            archive: Some(PathBuf::from("a.bz2")),
            ..ResticFileOptions::default()
        };
        assert!(supply(&options).is_err());
    }

    #[test]
    fn fr_ag_11_an_archive_for_another_platform_is_refused() {
        let platform = Platform::new("linux", "amd64");
        let supply = Supply::Files {
            archive: PathBuf::from("/tmp/restic_0.19.1_windows_amd64.zip"),
            sums: data("SHA256SUMS"),
            sums_signature: data("SHA256SUMS.asc"),
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        assert!(runtime.block_on(supply.fetch(platform)).is_err());
    }
}
