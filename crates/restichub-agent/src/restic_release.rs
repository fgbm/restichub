//! Fetches and verifies a restic release (ADR-0038, FR-AG-11, threat T20).
//!
//! The chain is: a signed `SHA256SUMS` file, the archive hash from that file, and then the unpacked
//! binary. A release counts only when every link holds.

use std::io::Read as _;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

use crate::http_get::{HttpsClient, Response};
use crate::openpgp::{self, PublicKey};

/// The oldest restic that ResticHub supports (NFR-CMP-03).
const MIN_VERSION: [u64; 3] = [0, 18, 1];
/// A higher major version has no compatibility test, so the agent refuses it (NFR-CMP-03).
const TESTED_MAJOR: u64 = 0;

const MAX_API_BYTES: usize = 1024 * 1024;
const MAX_SUMS_BYTES: usize = 1024 * 1024;
const MAX_SIGNATURE_BYTES: usize = 64 * 1024;
const MAX_ARCHIVE_BYTES: usize = 64 * 1024 * 1024;
const MAX_BINARY_BYTES: u64 = 256 * 1024 * 1024;

/// How many of the newest supported releases the fallback tries.
const MAX_FALLBACKS: usize = 3;

const GITHUB_API_LATEST: &str = "https://api.github.com/repos/restic/restic/releases/latest";
// One bounded page: the newest ten releases are enough to find a supported one.
const GITHUB_API_LIST: &str = "https://api.github.com/repos/restic/restic/releases?per_page=10";
const GITHUB_DOWNLOAD_BASE: &str = "https://github.com/restic/restic/releases/download";

/// The release that the build pins in `packaging/restic.env` (ADR-0011, ADR-0038).
const PINNED_ENV: &str = include_str!("../../../packaging/restic.env");

/// The signing key of the restic releases (ADR-0031, ADR-0038).
const RELEASE_KEY: &str = include_str!("../../../packaging/restic-release-key.asc");
const RELEASE_KEY_FINGERPRINT: &str = "CF8F18F2844575973F79D4E191A6868BD3F7A907";

/// Parses the embedded key block and selects the key with the pinned fingerprint.
///
/// # Errors
///
/// Returns an error when the embedded key block does not hold the pinned key.
pub fn release_key() -> Result<PublicKey> {
    PublicKey::from_armored(RELEASE_KEY, RELEASE_KEY_FINGERPRINT)
}

/// Applies the minimum version and the tested major version (NFR-CMP-03).
///
/// # Errors
///
/// Returns an error when the version is malformed, older than the minimum, or of a major version
/// above the tested one.
pub fn check_supported(version: &str) -> Result<()> {
    let parsed = parse_version(version)?;
    if parsed[0] > TESTED_MAJOR {
        bail!(
            "restic {version} is of a major version above the tested one ({TESTED_MAJOR}.x); ResticHub does not run it"
        );
    }
    if parsed < MIN_VERSION {
        bail!(
            "restic {version} is older than the minimum {}.{}.{}",
            MIN_VERSION[0],
            MIN_VERSION[1],
            MIN_VERSION[2]
        );
    }
    Ok(())
}

/// Tells whether `candidate` is an older release than `installed`.
///
/// # Errors
///
/// Returns an error when either text is not a plain version.
pub fn is_older(candidate: &str, installed: &str) -> Result<bool> {
    Ok(parse_version(candidate)? < parse_version(installed)?)
}

fn pinned_version() -> &'static str {
    PINNED_ENV
        .lines()
        .find_map(|line| line.strip_prefix("RESTIC_VERSION="))
        .map_or("", str::trim)
}

fn parse_version(version: &str) -> Result<[u64; 3]> {
    let mut parts = version.split('.');
    let mut parsed = [0_u64; 3];
    for slot in &mut parsed {
        let part = parts
            .next()
            .filter(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
            .with_context(|| format!("`{version}` is not a restic version like 0.19.1"))?;
        *slot = part
            .parse()
            .with_context(|| format!("`{version}` is not a restic version like 0.19.1"))?;
    }
    if parts.next().is_some() {
        bail!("`{version}` is not a restic version like 0.19.1");
    }
    Ok(parsed)
}

/// Operating system and processor, as the release archive names spell them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Platform {
    os: &'static str,
    arch: &'static str,
}

impl Platform {
    /// Maps the build target to the names in the release archives.
    ///
    /// # Errors
    ///
    /// Returns an error on an operating system or a processor that has no restic release build
    /// that ResticHub uses.
    pub fn host() -> Result<Self> {
        let os = match std::env::consts::OS {
            "linux" => "linux",
            "windows" => "windows",
            other => bail!("the restic download is not available on {other}"),
        };
        let arch = match std::env::consts::ARCH {
            "x86_64" => "amd64",
            "aarch64" => "arm64",
            other => bail!("the restic download is not available on {other}"),
        };
        Ok(Self { os, arch })
    }

    #[must_use]
    pub fn new(os: &'static str, arch: &'static str) -> Self {
        Self { os, arch }
    }

    fn extension(self) -> &'static str {
        if self.os == "windows" { "zip" } else { "bz2" }
    }

    fn archive_name(self, version: &str) -> String {
        format!(
            "restic_{version}_{}_{}.{}",
            self.os,
            self.arch,
            self.extension()
        )
    }

    fn version_of_archive(self, name: &str) -> Result<String> {
        let suffix = format!("_{}_{}.{}", self.os, self.arch, self.extension());
        let version = name
            .strip_prefix("restic_")
            .and_then(|rest| rest.strip_suffix(&suffix))
            .with_context(|| {
                format!(
                    "the archive name `{name}` is not restic_<version>{suffix}, the build for this host"
                )
            })?;
        parse_version(version)?;
        Ok(version.to_owned())
    }
}

/// Where the release files come from (ADR-0038).
#[derive(Debug, Clone)]
pub enum Source {
    /// The release assets of the restic project on GitHub.
    GitHub {
        api_latest: String,
        /// One page of the newest releases, for the fallback.
        api_list: String,
        download_base: String,
        /// The last resort when the API gives no usable release.
        pinned: String,
    },
}

impl Source {
    #[must_use]
    pub fn github() -> Self {
        Self::GitHub {
            api_latest: GITHUB_API_LATEST.to_owned(),
            api_list: GITHUB_API_LIST.to_owned(),
            download_base: GITHUB_DOWNLOAD_BASE.to_owned(),
            pinned: pinned_version().to_owned(),
        }
    }

    fn asset_url(&self, version: &str, file: &str) -> String {
        match self {
            Self::GitHub { download_base, .. } => format!("{download_base}/v{version}/{file}"),
        }
    }

    async fn recent_versions(&self, client: &HttpsClient) -> Result<Vec<String>> {
        match self {
            Self::GitHub { api_list, .. } => {
                let response = client
                    .get(api_list, "application/vnd.github+json", MAX_API_BYTES)
                    .await
                    .context("cannot ask GitHub for the list of restic releases")?;
                github_recent(&response)
            }
        }
    }

    async fn latest_version(&self, client: &HttpsClient) -> Result<String> {
        match self {
            Self::GitHub { api_latest, .. } => {
                let response = client
                    .get(api_latest, "application/vnd.github+json", MAX_API_BYTES)
                    .await
                    .context("cannot ask GitHub for the latest restic release")?;
                github_latest(&response)
            }
        }
    }
}

/// The newest supported releases of one API page, newest first.
fn github_recent(response: &Response) -> Result<Vec<String>> {
    #[derive(Deserialize)]
    struct Release {
        tag_name: String,
        #[serde(default)]
        draft: bool,
        #[serde(default)]
        prerelease: bool,
    }
    if response.status != 200 {
        bail!(
            "GitHub answered the release list query with HTTP {}",
            response.status
        );
    }
    let releases: Vec<Release> = serde_json::from_slice(&response.body)
        .context("the GitHub release list is not valid JSON")?;
    let mut versions: Vec<(String, [u64; 3])> = releases
        .into_iter()
        .filter(|release| !release.draft && !release.prerelease)
        .filter_map(|release| {
            let version = release.tag_name.trim_start_matches('v').to_owned();
            let parsed = parse_version(&version).ok()?;
            check_supported(&version).ok()?;
            Some((version, parsed))
        })
        .collect();
    versions.sort_by(|a, b| b.1.cmp(&a.1));
    Ok(versions
        .into_iter()
        .map(|(version, _)| version)
        .take(MAX_FALLBACKS)
        .collect())
}

fn github_latest(response: &Response) -> Result<String> {
    #[derive(Deserialize)]
    struct Latest {
        tag_name: String,
    }
    match response.status {
        200 => {}
        403 | 429
            if response
                .headers
                .get("x-ratelimit-remaining")
                .is_some_and(|v| v == "0") =>
        {
            let reset = response
                .headers
                .get("x-ratelimit-reset")
                .and_then(|value| value.to_str().ok())
                .map_or_else(String::new, |value| {
                    format!(" (resets at Unix time {value})")
                });
            bail!(
                "GitHub refused the request because of its rate limit{reset}; try again later or give --restic-archive, --restic-sums and --restic-sums-sig"
            );
        }
        status => bail!("GitHub answered the release query with HTTP {status}"),
    }
    let latest: Latest = serde_json::from_slice(&response.body)
        .context("the GitHub release answer is not valid JSON")?;
    let version = latest.tag_name.trim_start_matches('v').to_owned();
    parse_version(&version).context("the latest release tag of restic is not a plain version")?;
    Ok(version)
}

/// The three files of one release and the name of the archive.
#[derive(Debug)]
pub struct ReleaseFiles {
    pub archive_name: String,
    pub archive: Vec<u8>,
    pub sums: Vec<u8>,
    pub sums_signature: String,
}

/// How the agent gets the release files (ADR-0038).
pub enum Supply {
    Download {
        source: Source,
        client: HttpsClient,
    },
    /// Files that the operator copied to an offline host.
    Files {
        archive: PathBuf,
        sums: PathBuf,
        sums_signature: PathBuf,
    },
}

impl Supply {
    /// Reads or downloads the release files for `platform`.
    ///
    /// # Errors
    ///
    /// Returns an error when a file cannot be read or downloaded.
    pub async fn fetch(&self, platform: Platform) -> Result<ReleaseFiles> {
        match self {
            Self::Download { .. } => {
                let version = self.latest_version().await?;
                self.fetch_version(platform, &version).await
            }
            Self::Files {
                archive,
                sums,
                sums_signature,
            } => {
                let archive_name = archive
                    .file_name()
                    .and_then(|name| name.to_str())
                    .context("the archive path has no file name")?
                    .to_owned();
                platform.version_of_archive(&archive_name)?;
                Ok(ReleaseFiles {
                    archive_name,
                    archive: read_bounded(archive, MAX_ARCHIVE_BYTES)?,
                    sums: read_bounded(sums, MAX_SUMS_BYTES)?,
                    sums_signature: String::from_utf8(read_bounded(
                        sums_signature,
                        MAX_SIGNATURE_BYTES,
                    )?)
                    .context("the signature file is not text")?,
                })
            }
        }
    }

    /// The version that the project marks as the latest release.
    ///
    /// # Errors
    ///
    /// Returns an error for offline files, or when the query fails.
    pub async fn latest_version(&self) -> Result<String> {
        match self {
            Self::Download { source, client } => source.latest_version(client).await,
            Self::Files { .. } => bail!("offline files name no latest release"),
        }
    }

    /// The newest supported releases of one bounded API page, newest first.
    ///
    /// # Errors
    ///
    /// Returns an error for offline files, or when the query fails.
    pub async fn recent_versions(&self) -> Result<Vec<String>> {
        match self {
            Self::Download { source, client } => source.recent_versions(client).await,
            Self::Files { .. } => bail!("offline files name no releases"),
        }
    }

    /// The release that the build pins, which needs no API query.
    #[must_use]
    pub fn pinned_version(&self) -> Option<&str> {
        match self {
            Self::Download {
                source: Source::GitHub { pinned, .. },
                ..
            } => Some(pinned.as_str()),
            Self::Files { .. } => None,
        }
    }

    /// Downloads the three files of one release. A version that the agent does not support is
    /// refused before the download (NFR-CMP-03).
    ///
    /// # Errors
    ///
    /// Returns an error for offline files, for an unsupported version, or when a download fails.
    pub async fn fetch_version(&self, platform: Platform, version: &str) -> Result<ReleaseFiles> {
        let Self::Download { source, client } = self else {
            bail!("offline files hold one release only");
        };
        check_supported(version)?;
        let archive_name = platform.archive_name(version);
        let archive = download(client, source, version, &archive_name, MAX_ARCHIVE_BYTES).await?;
        let sums = download(client, source, version, "SHA256SUMS", MAX_SUMS_BYTES).await?;
        let signature = download(
            client,
            source,
            version,
            "SHA256SUMS.asc",
            MAX_SIGNATURE_BYTES,
        )
        .await?;
        Ok(ReleaseFiles {
            archive_name,
            archive,
            sums,
            sums_signature: String::from_utf8(signature).context("SHA256SUMS.asc is not text")?,
        })
    }
}

async fn download(
    client: &HttpsClient,
    source: &Source,
    version: &str,
    file: &str,
    max_bytes: usize,
) -> Result<Vec<u8>> {
    let url = source.asset_url(version, file);
    let response = client
        .get(&url, "application/octet-stream", max_bytes)
        .await
        .with_context(|| format!("cannot download {file}"))?;
    if response.status != 200 {
        bail!(
            "the download of {file} failed with HTTP {}",
            response.status
        );
    }
    Ok(response.body)
}

fn read_bounded(path: &std::path::Path, max_bytes: usize) -> Result<Vec<u8>> {
    let file =
        std::fs::File::open(path).with_context(|| format!("cannot read {}", path.display()))?;
    let limit = u64::try_from(max_bytes).unwrap_or(u64::MAX);
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("cannot read {}", path.display()))?;
    if bytes.len() > max_bytes {
        bail!("{} is larger than {max_bytes} bytes", path.display());
    }
    Ok(bytes)
}

/// A restic binary that passed every check of the release chain.
#[derive(Debug)]
pub struct VerifiedRelease {
    pub version: String,
    pub binary: Vec<u8>,
    pub sha256: String,
}

/// Verifies the signature, the archive hash and the version, and unpacks the binary.
///
/// # Errors
///
/// Returns an error when the signature does not verify with `key`, when the archive is not listed
/// in the signed sums or does not match them, when the version is unsupported, or when the archive
/// does not hold one restic binary.
pub fn verify_release(
    files: &ReleaseFiles,
    platform: Platform,
    key: &PublicKey,
) -> Result<VerifiedRelease> {
    openpgp::verify_detached(key, &files.sums, &files.sums_signature)
        .context("the signature of SHA256SUMS is not valid")?;
    let version = platform.version_of_archive(&files.archive_name)?;
    check_supported(&version)?;
    let sums = std::str::from_utf8(&files.sums).context("SHA256SUMS is not text")?;
    let expected = sum_of(sums, &files.archive_name)?;
    let actual = hex(&Sha256::digest(&files.archive));
    if actual != expected {
        bail!(
            "{} does not match the signed SHA256SUMS: expected {expected}, got {actual}",
            files.archive_name
        );
    }
    let binary = unpack(&files.archive_name, &files.archive)?;
    let sha256 = hex(&Sha256::digest(&binary));
    Ok(VerifiedRelease {
        version,
        binary,
        sha256,
    })
}

fn sum_of(sums: &str, name: &str) -> Result<String> {
    let mut found = None;
    for line in sums.lines() {
        let Some((hash, file)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        if file.trim_start().trim_start_matches('*') == name {
            let hash = hash.to_ascii_lowercase();
            if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                bail!("SHA256SUMS holds a bad hash for {name}");
            }
            if found.replace(hash).is_some() {
                bail!("SHA256SUMS lists {name} twice");
            }
        }
    }
    found.with_context(|| format!("the signed SHA256SUMS does not list {name}"))
}

fn unpack(archive_name: &str, archive: &[u8]) -> Result<Vec<u8>> {
    let mut binary = Vec::new();
    if archive_name.ends_with(".bz2") {
        bzip2::read::BzDecoder::new(archive)
            .take(MAX_BINARY_BYTES + 1)
            .read_to_end(&mut binary)
            .context("cannot unpack the bz2 archive")?;
    } else if archive_name.ends_with(".zip") {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive))
            .context("cannot read the zip archive")?;
        if zip.len() != 1 {
            bail!("the zip archive must hold exactly one file");
        }
        let entry = zip.by_index(0).context("cannot read the zip entry")?;
        let name = entry
            .name()
            .context("the zip entry name is not valid")?
            .into_owned();
        if !entry.is_file() || !name.starts_with("restic_") || !name.ends_with(".exe") {
            bail!("the zip entry `{name}` is not the restic executable");
        }
        entry
            .take(MAX_BINARY_BYTES + 1)
            .read_to_end(&mut binary)
            .context("cannot unpack the zip entry")?;
    } else {
        bail!("{archive_name} is neither a bz2 nor a zip archive");
    }
    if binary.is_empty() {
        bail!("the archive holds an empty file");
    }
    if u64::try_from(binary.len()).unwrap_or(u64::MAX) > MAX_BINARY_BYTES {
        bail!("the unpacked binary is larger than {MAX_BINARY_BYTES} bytes");
    }
    Ok(binary)
}

/// Formats bytes as lowercase hexadecimal, as `SHA256SUMS` does.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use std::io::Write as _;

    use super::*;

    #[test]
    fn nfr_cmp_03_the_supported_range_is_enforced() {
        check_supported("0.18.1").expect("the minimum");
        check_supported("0.19.1").expect("the target");
        assert!(check_supported("0.18.0").is_err());
        assert!(check_supported("0.17.3").is_err());
        let major = check_supported("1.0.0").expect_err("a higher major version");
        assert!(major.to_string().contains("major version above"));
        assert!(check_supported("0.19").is_err());
        assert!(check_supported("0.19.1-beta").is_err());
        assert!(check_supported("0.19.1.2").is_err());
    }

    #[test]
    fn nfr_cmp_03_the_pinned_release_is_supported() {
        check_supported(pinned_version()).expect("the pinned release");
    }

    #[test]
    fn nfr_cmp_03_the_fallback_list_keeps_the_newest_supported_releases() {
        let body = br#"[
            {"tag_name":"v1.0.0"},
            {"tag_name":"v0.20.0-rc1","prerelease":true},
            {"tag_name":"v0.19.1"},
            {"tag_name":"v0.18.1"},
            {"tag_name":"v0.19.2","draft":true},
            {"tag_name":"v0.17.0"},
            {"tag_name":"nightly"},
            {"tag_name":"v0.19.0"},
            {"tag_name":"v0.18.9"}
        ]"#;
        let response = Response {
            status: 200,
            headers: hyper::HeaderMap::new(),
            body: body.to_vec(),
        };
        assert_eq!(
            github_recent(&response).expect("list"),
            ["0.19.1", "0.19.0", "0.18.9"]
        );
        let failed = Response {
            status: 403,
            ..response
        };
        assert!(github_recent(&failed).is_err());
    }

    #[test]
    fn nfr_cmp_03_a_downgrade_is_detected() {
        assert!(is_older("0.18.1", "0.19.1").expect("compare"));
        assert!(!is_older("0.19.1", "0.19.1").expect("compare"));
        assert!(!is_older("0.19.10", "0.19.9").expect("compare"));
    }

    #[test]
    fn fr_ag_11_the_archive_name_gives_the_version() {
        let linux = Platform::new("linux", "amd64");
        assert_eq!(
            linux.archive_name("0.19.1"),
            "restic_0.19.1_linux_amd64.bz2"
        );
        assert_eq!(
            linux
                .version_of_archive("restic_0.19.1_linux_amd64.bz2")
                .expect("version"),
            "0.19.1"
        );
        assert!(
            linux
                .version_of_archive("restic_0.19.1_linux_arm64.bz2")
                .is_err()
        );
        assert!(linux.version_of_archive("evil.bz2").is_err());
        let windows = Platform::new("windows", "amd64");
        assert_eq!(
            windows.archive_name("0.19.1"),
            "restic_0.19.1_windows_amd64.zip"
        );
    }

    #[test]
    fn fr_ag_11_the_sum_lookup_wants_exactly_one_entry() {
        let hash = "a".repeat(64);
        let sums = format!("{hash}  restic_0.19.1_linux_amd64.bz2\n{hash}  other\n");
        assert_eq!(
            sum_of(&sums, "restic_0.19.1_linux_amd64.bz2").expect("sum"),
            hash
        );
        assert!(sum_of(&sums, "missing").is_err());
        let twice = format!("{sums}{hash}  restic_0.19.1_linux_amd64.bz2\n");
        assert!(sum_of(&twice, "restic_0.19.1_linux_amd64.bz2").is_err());
        assert!(sum_of("xyz  name\n", "name").is_err());
    }

    #[test]
    fn fr_ag_11_a_bz2_archive_is_unpacked() {
        let mut encoder = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::fast());
        encoder.write_all(b"restic bytes").expect("write");
        let archive = encoder.finish().expect("finish");
        assert_eq!(
            unpack("restic_0.19.1_linux_amd64.bz2", &archive).expect("unpack"),
            b"restic bytes"
        );
    }

    #[test]
    fn fr_ag_11_a_zip_archive_is_unpacked() {
        let mut buffer = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(&mut buffer);
        writer
            .start_file(
                "restic_0.19.1_windows_amd64.exe",
                zip::write::SimpleFileOptions::default(),
            )
            .expect("start");
        writer.write_all(b"restic bytes").expect("write");
        writer.finish().expect("finish");
        let archive = buffer.into_inner();
        assert_eq!(
            unpack("restic_0.19.1_windows_amd64.zip", &archive).expect("unpack"),
            b"restic bytes"
        );
    }

    #[test]
    fn fr_ag_11_a_zip_with_a_foreign_entry_is_refused() {
        let mut buffer = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(&mut buffer);
        writer
            .start_file("evil.exe", zip::write::SimpleFileOptions::default())
            .expect("start");
        writer.write_all(b"x").expect("write");
        writer.finish().expect("finish");
        assert!(unpack("restic_0.19.1_windows_amd64.zip", &buffer.into_inner()).is_err());
    }

    #[test]
    fn fr_ag_11_the_release_key_is_the_pinned_key() {
        release_key().expect("the pinned release key must load");
    }

    #[test]
    fn fr_ag_11_a_github_rate_limit_is_explained() {
        let mut headers = hyper::HeaderMap::new();
        headers.insert("x-ratelimit-remaining", "0".parse().expect("header"));
        headers.insert("x-ratelimit-reset", "1760000000".parse().expect("header"));
        let response = Response {
            status: 403,
            headers,
            body: Vec::new(),
        };
        let error = github_latest(&response).expect_err("rate limited");
        assert!(error.to_string().contains("rate limit"), "{error}");
        assert!(error.to_string().contains("--restic-archive"), "{error}");
    }

    #[test]
    fn fr_ag_11_the_latest_tag_is_read() {
        let response = Response {
            status: 200,
            headers: hyper::HeaderMap::new(),
            body: br#"{"tag_name":"v0.19.1"}"#.to_vec(),
        };
        assert_eq!(github_latest(&response).expect("version"), "0.19.1");
        let bad = Response {
            status: 200,
            headers: hyper::HeaderMap::new(),
            body: br#"{"tag_name":"nightly"}"#.to_vec(),
        };
        assert!(github_latest(&bad).is_err());
    }
}
