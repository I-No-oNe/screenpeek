//! Download and install release binaries without rerunning interactive setup.

use std::fs;
use std::io::{Cursor, Read};
use std::path::Path;
use std::time::Duration;

use anyhow::{bail, ensure, Context, Result};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};

const REPO: &str = "https://api.github.com/repos/I-No-oNe/screenpeek";
const DOWNLOAD: &str = "https://github.com/I-No-oNe/screenpeek/releases/download";
const MAX_DOWNLOAD: u64 = 256 * 1024 * 1024;

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
}

fn archive_name(os: &str, arch: &str) -> Result<&'static str> {
    match (os, arch) {
        ("linux", "x86_64") => Ok("screenpeek-x86_64-linux.tar.gz"),
        ("windows", "x86_64") => Ok("screenpeek-x86_64-windows.zip"),
        ("windows", "aarch64") => Ok("screenpeek-aarch64-windows.zip"),
        _ => bail!("no prebuilt release for {os}-{arch}; update from source with cargo install --git https://github.com/I-No-oNe/screenpeek"),
    }
}

fn newest<'a>(releases: &'a [Release], archive: &str) -> Result<(&'a Release, Version)> {
    releases
        .iter()
        .filter(|release| {
            !release.draft && release.assets.iter().any(|asset| asset.name == archive)
        })
        .filter_map(|release| {
            Version::parse(release.tag_name.trim_start_matches('v'))
                .ok()
                .map(|version| (release, version))
        })
        .max_by(|(_, a), (_, b)| a.cmp(b))
        .context("no compatible release found")
}

fn download(agent: &ureq::Agent, url: &str, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    agent
        .get(url)
        .call()
        .with_context(|| format!("cannot download {url}"))?
        .body_mut()
        .as_reader()
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "download exceeds size limit");
    Ok(bytes)
}

fn verify(bytes: &[u8], checksum: &[u8]) -> Result<()> {
    let checksum = std::str::from_utf8(checksum)?;
    let expected = checksum
        .split_whitespace()
        .next()
        .context("empty checksum")?;
    let actual: String = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    ensure!(
        expected.len() == 64 && expected.eq_ignore_ascii_case(&actual),
        "release archive does not match its SHA-256 checksum"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
fn extract(bytes: &[u8], directory: &Path) -> Result<()> {
    use flate2::read::GzDecoder;
    use std::os::unix::fs::PermissionsExt;

    let mut archive = tar::Archive::new(GzDecoder::new(Cursor::new(bytes)));
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        if path != Path::new("screenpeek") && path != Path::new("screenpeek-frames") {
            continue;
        }
        ensure!(
            entry.header().entry_type().is_file(),
            "release binary is not a regular file"
        );
        ensure!(
            entry.size() <= MAX_DOWNLOAD,
            "release binary exceeds size limit"
        );
        let path = directory.join(path);
        ensure!(!path.exists(), "duplicate release binary");
        let mut file = fs::File::create(&path)?;
        std::io::copy(&mut entry, &mut file)?;
        file.set_permissions(fs::Permissions::from_mode(0o755))?;
    }
    ensure!(
        directory.join("screenpeek").is_file(),
        "archive has no screenpeek binary"
    );
    Ok(())
}

#[cfg(windows)]
fn extract(bytes: &[u8], directory: &Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    let mut entry = archive.by_name("screenpeek.exe")?;
    ensure!(
        entry.is_file() && entry.size() <= MAX_DOWNLOAD,
        "invalid release binary"
    );
    let mut file = fs::File::create(directory.join("screenpeek.exe"))?;
    std::io::copy(&mut entry, &mut file)?;
    Ok(())
}

#[cfg(not(any(target_os = "linux", windows)))]
fn extract(_bytes: &[u8], _directory: &Path) -> Result<()> {
    bail!("no prebuilt release for this platform")
}

pub(crate) fn update() -> Result<()> {
    let archive = archive_name(std::env::consts::OS, std::env::consts::ARCH)?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(120)))
        .user_agent(concat!("screenpeek/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    out!("Checking for updates...");
    let releases: Vec<Release> = serde_json::from_slice(&download(
        &agent,
        &format!("{REPO}/releases?per_page=100"),
        8 * 1024 * 1024,
    )?)?;
    let (release, version) = newest(&releases, archive)?;
    if version <= Version::parse(env!("CARGO_PKG_VERSION"))? {
        out!("screenpeek {} is up to date", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let checksum = format!("{archive}.sha256");
    ensure!(
        release.assets.iter().any(|asset| asset.name == checksum),
        "release has no SHA-256 checksum; refusing to install"
    );
    let tag = &release.tag_name;
    out!("Downloading screenpeek {tag}...");
    let url = format!("{DOWNLOAD}/{tag}");
    let bytes = download(&agent, &format!("{url}/{archive}"), MAX_DOWNLOAD)?;
    verify(
        &bytes,
        &download(&agent, &format!("{url}/{checksum}"), 4096)?,
    )?;
    let executable = std::env::current_exe()?.canonicalize()?;
    let directory = executable
        .parent()
        .context("binary has no parent directory")?;
    // Stage beside the binary so helper replacement is an atomic rename.
    let staging = tempfile::tempdir_in(directory).context(
        "installation directory is not writable; update using its package manager or owner",
    )?;
    extract(&bytes, staging.path())?;
    #[cfg(target_os = "linux")]
    {
        let helper = staging.path().join("screenpeek-frames");
        if helper.exists() {
            fs::rename(helper, directory.join("screenpeek-frames"))
                .context("cannot replace screenpeek-frames")?;
        }
    }
    let binary = staging
        .path()
        .join(format!("screenpeek{}", std::env::consts::EXE_SUFFIX));
    self_replace::self_replace(binary).context("cannot replace screenpeek executable")?;
    out!("Updated to {tag}. Restart running MCP servers and daemons to use it.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_assets_match_release_names() {
        assert_eq!(
            archive_name("linux", "x86_64").unwrap(),
            "screenpeek-x86_64-linux.tar.gz"
        );
        assert_eq!(
            archive_name("windows", "aarch64").unwrap(),
            "screenpeek-aarch64-windows.zip"
        );
        assert!(archive_name("linux", "aarch64").is_err());
    }

    #[test]
    fn selects_newest_compatible_version_including_prereleases() {
        let releases = serde_json::from_str::<Vec<Release>>(
            r#"[
            {"tag_name":"v0.1.0-alpha.2","draft":false,"assets":[{"name":"binary"}]},
            {"tag_name":"v9.0.0","draft":true,"assets":[{"name":"binary"}]},
            {"tag_name":"v8.0.0","draft":false,"assets":[{"name":"other"}]},
            {"tag_name":"invalid","draft":false,"assets":[{"name":"binary"}]},
            {"tag_name":"v0.1.0-alpha.10","draft":false,"assets":[{"name":"binary"}]}
        ]"#,
        )
        .unwrap();
        assert_eq!(
            newest(&releases, "binary").unwrap().0.tag_name,
            "v0.1.0-alpha.10"
        );
        assert!(newest(&releases, "missing").is_err());
    }

    #[test]
    fn checksum_rejects_corruption_and_missing_hash() {
        let checksum =
            "9A3A45D01531A20E89AC6AE10B0B0BEB0492ACD7216A368AA062D1A5FECAF9CD  archive\n";
        verify(b"binary", checksum.as_bytes()).unwrap();
        assert!(verify(b"corrupted", checksum.as_bytes()).is_err());
        assert!(verify(b"binary", b"").is_err());
    }

    #[test]
    fn failed_download_is_reported() {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(&mut stream);
            loop {
                let mut line = String::new();
                assert!(reader.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
            }
            stream
                .write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
        });
        assert!(download(&ureq::Agent::new_with_defaults(), &url, 1024).is_err());
        server.join().unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn extracts_only_release_binaries_and_requires_main_binary() {
        use flate2::{write::GzEncoder, Compression};
        let mut archive = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
        for name in ["screenpeek", "screenpeek-frames", "unrelated"] {
            let mut header = tar::Header::new_gnu();
            header.set_size(6);
            header.set_mode(0o755);
            header.set_cksum();
            archive
                .append_data(&mut header, name, &b"binary"[..])
                .unwrap();
        }
        let bytes = archive.into_inner().unwrap().finish().unwrap();
        let directory = tempfile::tempdir().unwrap();
        extract(&bytes, directory.path()).unwrap();
        assert_eq!(
            fs::read(directory.path().join("screenpeek")).unwrap(),
            b"binary"
        );
        assert!(directory.path().join("screenpeek-frames").is_file());
        assert!(!directory.path().join("unrelated").exists());
        let empty = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
        let empty = empty.into_inner().unwrap().finish().unwrap();
        assert!(extract(&empty, tempfile::tempdir().unwrap().path()).is_err());
    }
}
