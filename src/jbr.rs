use anyhow::{bail, Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use serde::Deserialize;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

const GITHUB_RELEASES_URL: &str =
    "https://api.github.com/repos/JetBrains/JetBrainsRuntime/releases";
const CACHE_REDIRECTOR_BASE: &str = "https://cache-redirector.jetbrains.com/intellij-jbr";
const MAX_PAGES: u32 = 5;
const PER_PAGE: u32 = 100;

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Returns the JBR installation directory for the given Java major version.
/// Path: ~/.minipot/jbr/jbr-{version}/  (separate from the plugin cache at ~/.cache/minipot/)
pub fn jbr_dir(java_version: u32) -> Result<PathBuf> {
    let home = dirs::home_dir().context("Cannot determine home directory")?;
    Ok(home
        .join(".minipot")
        .join("jbr")
        .join(format!("jbr-{java_version}")))
}

/// Returns the path to the java binary inside the installed JBR.
pub fn java_bin(java_version: u32) -> Result<PathBuf> {
    let bin_name = if cfg!(windows) { "java.exe" } else { "java" };
    Ok(jbr_dir(java_version)?.join("bin").join(bin_name))
}

/// Returns true if JBR for the given Java major version is already installed.
pub fn is_installed(java_version: u32) -> bool {
    java_bin(java_version)
        .map(|p| p.exists())
        .unwrap_or(false)
}

/// Ensures JBR for the given Java major version is installed.
/// Downloads and extracts it from JetBrains GitHub releases if not present.
/// Returns the path to the java binary.
pub fn ensure_installed(java_version: u32) -> Result<PathBuf> {
    if is_installed(java_version) {
        return java_bin(java_version);
    }

    println!("[Minipot] JetBrains Runtime {java_version} not found — downloading...");

    let (java_str, build) = find_latest_release(java_version)
        .with_context(|| format!("Failed to find JBR {java_version} release on GitHub"))?;

    let dest = jbr_dir(java_version)?;
    download_and_extract(java_version, &java_str, &build, &dest)?;

    let bin = java_bin(java_version)?;
    if !bin.exists() {
        bail!(
            "JBR installation failed — java binary not found at {}",
            bin.display()
        );
    }

    // Verify the installation works
    let output = Command::new(&bin)
        .arg("-version")
        .output()
        .context("Failed to run java -version after JBR installation")?;

    if !output.status.success() {
        bail!("JBR installation verification failed — java -version returned non-zero exit code");
    }

    println!(
        "[Minipot] JBR {java_version} ready at {}",
        dest.display()
    );
    Ok(bin)
}

// ─── Release resolution ───────────────────────────────────────────────────────

/// Searches GitHub releases (paginated) for the latest JBR release matching
/// the given Java major version. Returns (java_str, build), e.g. ("21.0.10", "1163.110").
fn find_latest_release(java_version: u32) -> Result<(String, String)> {
    let prefix = format!("jbr-release-{java_version}.");

    let client = reqwest::blocking::Client::builder()
        .user_agent("minipot-cli")
        .build()
        .context("Failed to build HTTP client")?;

    for page in 1..=MAX_PAGES {
        let url = format!("{GITHUB_RELEASES_URL}?per_page={PER_PAGE}&page={page}");

        let releases: Vec<GithubRelease> = client
            .get(&url)
            .send()
            .with_context(|| format!("Failed to reach GitHub API (page {page})"))?
            .error_for_status()
            .context("GitHub API returned an error — you may have hit the rate limit (60 req/h), try again later")?
            .json()
            .context("Failed to parse GitHub releases response")?;

        if releases.is_empty() {
            break;
        }

        for release in &releases {
            if release.tag_name.starts_with(&prefix) {
                return parse_tag(&release.tag_name);
            }
        }
    }

    bail!(
        "No JBR {java_version} release found in the last {} GitHub releases",
        MAX_PAGES * PER_PAGE
    )
}

/// Parses a tag like "jbr-release-21.0.10b1163.110" into ("21.0.10", "1163.110").
fn parse_tag(tag: &str) -> Result<(String, String)> {
    let after_prefix = tag
        .strip_prefix("jbr-release-")
        .with_context(|| format!("Unexpected JBR tag format: {tag}"))?;

    // Java version is numeric+dots; build number follows the first 'b'
    let (java_str, build) = after_prefix
        .split_once('b')
        .with_context(|| format!("Cannot parse build number from JBR tag: {tag}"))?;

    Ok((java_str.to_string(), build.to_string()))
}

// ─── Download + extraction ────────────────────────────────────────────────────

fn download_and_extract(
    java_version: u32,
    java_str: &str,
    build: &str,
    dest: &Path,
) -> Result<()> {
    let (os_str, ext) = if cfg!(windows) {
        ("windows", "zip")
    } else {
        ("linux", "tar.gz")
    };

    let filename = format!("jbr-{java_str}-{os_str}-x64-b{build}.{ext}");
    let url = format!("{CACHE_REDIRECTOR_BASE}/{filename}");

    println!("[Minipot] Downloading {filename}...");

    let response = reqwest::blocking::get(&url)
        .with_context(|| format!("Failed to download JBR from {url}"))?
        .error_for_status()
        .context("JBR download request returned an error")?;

    let total = response
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);

    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("  [{bar:40.cyan/white}] {bytes}/{total_bytes} ({eta})")
            .unwrap()
            .progress_chars("█▓░"),
    );

    // Ensure the parent directory exists before downloading
    let jbr_base = dest.parent().context("JBR dir has no parent")?;
    fs::create_dir_all(jbr_base)
        .with_context(|| format!("Failed to create directory {}", jbr_base.display()))?;

    let tmp_archive = jbr_base.join(format!("jbr-{java_version}-download.tmp"));

    {
        let mut file = fs::File::create(&tmp_archive)
            .with_context(|| format!("Failed to create temp file {}", tmp_archive.display()))?;

        let mut src = pb.wrap_read(response);
        let mut buf = vec![0u8; 65536];
        loop {
            let n = src.read(&mut buf).context("Error reading JBR download")?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).context("Error writing JBR archive to disk")?;
        }
    }

    pb.finish_and_clear();

    // Extract into a temp directory, then move the JBR root to dest
    let tmp_extract = jbr_base.join(format!("jbr-{java_version}-extract-tmp"));
    if tmp_extract.exists() {
        fs::remove_dir_all(&tmp_extract).context("Failed to remove stale temp extract dir")?;
    }
    fs::create_dir_all(&tmp_extract).context("Failed to create temp extract dir")?;

    println!("[Minipot] Extracting JBR...");
    extract_archive(&tmp_archive, &tmp_extract)?;
    fs::remove_file(&tmp_archive).ok();

    // The archive has a top-level directory (e.g. jbr-21.0.10-linux-x64-b1163.110/).
    // Find it by looking for a directory that contains bin/.
    let jbr_root = find_jbr_root(&tmp_extract)?;

    if dest.exists() {
        fs::remove_dir_all(dest)
            .with_context(|| format!("Failed to remove existing JBR dir {}", dest.display()))?;
    }

    fs::rename(&jbr_root, dest)
        .with_context(|| format!("Failed to move JBR to {}", dest.display()))?;

    fs::remove_dir_all(&tmp_extract).ok();

    Ok(())
}

#[cfg(unix)]
fn extract_archive(archive_path: &Path, dest: &Path) -> Result<()> {
    use flate2::read::GzDecoder;
    use tar::Archive;

    let file = fs::File::open(archive_path)
        .with_context(|| format!("Failed to open archive {}", archive_path.display()))?;
    let gz = GzDecoder::new(file);
    let mut archive = Archive::new(gz);
    archive
        .unpack(dest)
        .context("Failed to extract JBR tar.gz archive")?;
    Ok(())
}

#[cfg(windows)]
fn extract_archive(archive_path: &Path, dest: &Path) -> Result<()> {
    let file = fs::File::open(archive_path)
        .with_context(|| format!("Failed to open archive {}", archive_path.display()))?;
    let mut archive = zip::ZipArchive::new(file).context("Failed to read JBR zip archive")?;
    archive
        .extract(dest)
        .context("Failed to extract JBR zip archive")?;
    Ok(())
}

/// Finds the JBR root inside the extraction directory.
/// The JBR root is the directory that contains a bin/ subdirectory.
fn find_jbr_root(extract_dir: &Path) -> Result<PathBuf> {
    for entry in fs::read_dir(extract_dir)
        .with_context(|| format!("Failed to read extract dir {}", extract_dir.display()))?
    {
        let entry = entry.context("Failed to read dir entry")?;
        let path = entry.path();
        if path.is_dir() && path.join("bin").is_dir() {
            return Ok(path);
        }
    }
    // Fallback: the extract dir itself might be the JBR root (flat archives)
    if extract_dir.join("bin").is_dir() {
        return Ok(extract_dir.to_path_buf());
    }
    bail!(
        "Could not find JBR root (no bin/ directory) inside {}",
        extract_dir.display()
    )
}
