//! The local setting that turns the trust mode on (FR-HK-08, AC-HK-08).

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use restichub_proto::agent::v1::ServerScriptsState;

use crate::trust::{self, Trust};

/// The most bytes that the agent reads from the setting file.
const MAX_BYTES: u64 = 4096;

const KEY: &str = "server_scripts";

/// Whether the host trusts scripts from the control plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptMode {
    /// No file, or a file that does not say `enabled`. This is the default.
    Disabled,
    Enabled,
    /// Another account can change the file or a directory above it. The mode is off.
    Insecure,
}

impl ScriptMode {
    #[must_use]
    pub fn to_proto(self) -> ServerScriptsState {
        match self {
            Self::Disabled => ServerScriptsState::Disabled,
            Self::Enabled => ServerScriptsState::Enabled,
            Self::Insecure => ServerScriptsState::Insecure,
        }
    }
}

/// The setting file of this host, next to the configuration directory like the allow-list, so a
/// swap of the identity never removes it.
#[must_use]
pub fn mode_file() -> PathBuf {
    mode_file_for(&crate::paths::config_dir())
}

/// The setting file that belongs to a configuration directory.
#[must_use]
pub fn mode_file_for(config_dir: &Path) -> PathBuf {
    let name = config_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("restichub-agent");
    config_dir.with_file_name(format!("{name}-server-scripts"))
}

/// The allow-list file of an older agent when it still exists. The agent no longer reads it
/// (ADR-0040), so the start warns about the path.
#[must_use]
pub fn removed_allow_list_file(config_dir: &Path) -> Option<PathBuf> {
    let name = config_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("restichub-agent");
    let path = config_dir.with_file_name(format!("{name}-hooks.toml"));
    std::fs::symlink_metadata(&path).is_ok().then_some(path)
}

pub(super) fn read(path: &Path, trust: Trust) -> ScriptMode {
    match std::fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return ScriptMode::Disabled,
        Err(error) => {
            tracing::warn!(%error, "cannot read the server scripts setting");
            return ScriptMode::Insecure;
        }
    }
    if let Err(error) = trust::check_file(path, trust) {
        tracing::warn!(error = %format!("{error:#}"), "the server scripts setting is not trusted, so no script runs");
        return ScriptMode::Insecure;
    }
    match read_text(path) {
        Ok(text) if parse(&text) => ScriptMode::Enabled,
        Ok(_) => ScriptMode::Disabled,
        Err(error) => {
            tracing::warn!(error = %format!("{error:#}"), "cannot read the server scripts setting");
            ScriptMode::Disabled
        }
    }
}

fn read_text(path: &Path) -> Result<String> {
    use std::io::Read as _;
    let mut text = String::new();
    std::fs::File::open(path)?
        .take(MAX_BYTES + 1)
        .read_to_string(&mut text)?;
    if text.len() as u64 > MAX_BYTES {
        bail!("the file is larger than {MAX_BYTES} bytes");
    }
    Ok(text)
}

/// True when the text says `server_scripts = enabled`. Anything else means off, so a damaged file
/// never turns the mode on.
fn parse(text: &str) -> bool {
    let mut enabled = false;
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return false;
        };
        if key.trim() != KEY {
            return false;
        }
        enabled = value.trim() == "enabled";
    }
    enabled
}

/// Turns the mode on or off for this host. Only root or Administrators can do it: the command
/// removes the file again when the agent would not trust it.
///
/// # Errors
///
/// Returns an error when the file cannot be written or the host does not trust it afterwards.
pub fn set_mode(config_dir: &Path, enabled: bool) -> Result<()> {
    set_mode_trusting(config_dir, enabled, Trust::Host)
}

/// Like [`set_mode`], with the account that the written file must belong to. A test passes
/// [`Trust::CurrentUser`]; the commands always pass [`Trust::Host`].
///
/// # Errors
///
/// Returns an error when the file cannot be written or `trust` does not accept it afterwards.
pub fn set_mode_trusting(config_dir: &Path, enabled: bool, trust: Trust) -> Result<()> {
    set_mode_at(&mode_file_for(config_dir), enabled, trust)
}

pub(super) fn set_mode_at(path: &Path, enabled: bool, trust: Trust) -> Result<()> {
    let value = if enabled { "enabled" } else { "disabled" };
    let text = format!("# Written by `restichub-agent trust-server-scripts`.\n{KEY} = {value}\n");
    crate::paths::replace_file(path, text.as_bytes())
        .with_context(|| format!("cannot write {}", path.display()))?;
    if let Err(error) = trust::check_file(path, trust) {
        let _ = std::fs::remove_file(path);
        bail!(
            "the agent would not trust {} ({error:#}); run the command as root or Administrator",
            path.display()
        );
    }
    Ok(())
}
