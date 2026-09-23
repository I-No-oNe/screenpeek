//! The file that tells clients where the daemon listens, and its token.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};

/// Remove the endpoint only if it still belongs to this daemon.
pub(super) fn release_endpoint(port: u16) {
    if matches!(read_endpoint(), Ok((listed, _)) if listed == port) {
        let _ = endpoint_path().map(fs::remove_file);
    }
}

pub(super) fn endpoint_path() -> Result<PathBuf> {
    Ok(dirs::cache_dir()
        .ok_or_else(|| anyhow!("no cache directory on this system"))?
        .join("screenpeek")
        .join("daemon-v3"))
}

pub(super) fn write_endpoint(port: u16, token: &str) -> Result<()> {
    let path = endpoint_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let partial = path.with_extension("partial");
    fs::write(&partial, format!("{port} {token}"))
        .with_context(|| format!("cannot write {}", partial.display()))?;
    owner_only(&partial)?;
    fs::rename(&partial, &path).with_context(|| format!("cannot write {}", path.display()))
}

#[cfg(unix)]
pub(super) fn owner_only(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("cannot restrict {}", path.display()))
}

#[cfg(not(unix))]
pub(super) fn owner_only(_path: &Path) -> Result<()> {
    Ok(())
}

pub(super) fn read_endpoint() -> Result<(u16, String)> {
    let raw = fs::read_to_string(endpoint_path()?)?;
    let (port, token) = raw
        .split_once(' ')
        .ok_or_else(|| anyhow!("the daemon file is malformed"))?;
    Ok((port.parse()?, token.to_owned()))
}

pub(super) fn new_token() -> String {
    let mut hasher = DefaultHasher::new();
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|age| age.as_nanos())
        .unwrap_or(0)
        .hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}
