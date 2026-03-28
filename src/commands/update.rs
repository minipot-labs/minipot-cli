use anyhow::{Context, Result};
use reqwest::blocking::Client;
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;

const GITHUB_REPO: &str = "minipot-labs/minipot-cli";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

fn asset_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "minipot-x86_64-pc-windows-gnu.exe"
    } else {
        "minipot-x86_64-unknown-linux-gnu"
    }
}

fn strip_v(tag: &str) -> &str {
    tag.strip_prefix('v').unwrap_or(tag)
}

pub fn execute() -> Result<()> {
    let client = Client::builder()
        .user_agent("minipot-cli")
        .build()
        .context("Failed to build HTTP client")?;

    // Fetch latest release
    let url = format!("https://api.github.com/repos/{GITHUB_REPO}/releases/latest");
    let release: Release = client
        .get(&url)
        .send()
        .context("Failed to reach GitHub — check your internet connection")?
        .error_for_status()
        .context("GitHub API returned an error")?
        .json()
        .context("Failed to parse GitHub release")?;

    let latest = strip_v(&release.tag_name);

    if latest == CURRENT_VERSION {
        println!("Already up to date (v{CURRENT_VERSION}).");
        return Ok(());
    }

    // Find the right asset for this OS
    let expected = asset_name();
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == expected)
        .with_context(|| {
            let available: Vec<&str> = release.assets.iter().map(|a| a.name.as_str()).collect();
            format!(
                "No asset '{}' in release {}. Available: {:?}",
                expected, release.tag_name, available
            )
        })?;

    // Determine paths
    let current_exe = std::env::current_exe().context("Failed to locate current binary")?;
    let dir = current_exe
        .parent()
        .context("Binary has no parent directory")?;
    let tmp_path = dir.join(if cfg!(target_os = "windows") {
        "minipot.tmp.exe"
    } else {
        "minipot.tmp"
    });

    println!(
        "Updating minipot v{} → v{} ...",
        CURRENT_VERSION, latest
    );

    // Download to temp file
    let bytes = client
        .get(&asset.browser_download_url)
        .send()
        .context("Failed to download new binary")?
        .error_for_status()
        .context("Download failed")?
        .bytes()
        .context("Failed to read download")?;

    fs::write(&tmp_path, &bytes).with_context(|| {
        format!(
            "Failed to write temporary file at {}. Permission denied — run with sudo or move minipot to a user-writable path.",
            tmp_path.display()
        )
    })?;

    // Replace binary
    replace_binary(&current_exe, &tmp_path)?;

    println!("Updated minipot v{CURRENT_VERSION} → v{latest}.");
    Ok(())
}

#[cfg(unix)]
fn replace_binary(current: &PathBuf, tmp: &PathBuf) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(tmp, fs::Permissions::from_mode(0o755))
        .context("Failed to set executable permission")?;

    fs::rename(tmp, current).with_context(|| {
        format!(
            "Failed to replace {}. Permission denied — run with sudo or move minipot to a user-writable path.",
            current.display()
        )
    })?;

    Ok(())
}

#[cfg(windows)]
fn replace_binary(current: &PathBuf, tmp: &PathBuf) -> Result<()> {
    let old_path = current.with_extension("old.exe");

    // Rename running binary out of the way (Windows allows rename but not overwrite)
    if old_path.exists() {
        let _ = fs::remove_file(&old_path);
    }
    fs::rename(current, &old_path).context(
        "Failed to rename current binary. Close any other minipot processes and try again.",
    )?;

    // Move new binary into place
    if let Err(e) = fs::rename(tmp, current) {
        // Rollback: restore the old binary
        let _ = fs::rename(&old_path, current);
        return Err(e).context("Failed to install new binary");
    }

    // Clean up old binary (best effort)
    let _ = fs::remove_file(&old_path);

    Ok(())
}
