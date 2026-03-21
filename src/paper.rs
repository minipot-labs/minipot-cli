use anyhow::{anyhow, bail, Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;

const PAPER_API_BASE: &str = "https://api.papermc.io/v2/projects/paper";
const SERVER_ICON_URL: &str =
    "https://minipot-assets.s3.eu-central-1.amazonaws.com/minipot-icon-server.png";

// ─── PaperMC API types ────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct BuildsResponse {
    builds: Vec<BuildEntry>,
}

#[derive(Deserialize)]
struct BuildEntry {
    build: u32,
    channel: String,
    downloads: Downloads,
}

#[derive(Deserialize)]
struct Downloads {
    application: ApplicationDownload,
}

#[derive(Deserialize)]
struct ApplicationDownload {
    name: String,
    sha256: String,
}

// ─── Public types ─────────────────────────────────────────────────────────────

pub struct PaperBuild {
    pub build: u32,
    pub sha256: String,
    pub url: String,
}

// ─── API resolution ───────────────────────────────────────────────────────────

/// Interroga l'API ufficiale PaperMC e restituisce la build stabile più recente
/// per la versione MC richiesta, con il suo SHA256 e URL di download.
pub fn resolve_latest_build(version: &str) -> Result<PaperBuild> {
    let url = format!("{PAPER_API_BASE}/versions/{version}/builds");
    let resp: BuildsResponse = reqwest::blocking::get(&url)
        .with_context(|| format!("Failed to reach PaperMC API for version {version}"))?
        .error_for_status()
        .with_context(|| format!("Paper version '{version}' not found — check your minipot.yml"))?
        .json()
        .context("Failed to parse PaperMC builds response")?;

    let latest = resp
        .builds
        .iter()
        .filter(|b| b.channel == "default")
        .last()
        .ok_or_else(|| anyhow!("No stable builds found for Paper {version}"))?;

    let download_url = format!(
        "{PAPER_API_BASE}/versions/{version}/builds/{}/downloads/{}",
        latest.build, latest.downloads.application.name
    );

    Ok(PaperBuild {
        build: latest.build,
        sha256: latest.downloads.application.sha256.clone(),
        url: download_url,
    })
}

// ─── Download + verifica ──────────────────────────────────────────────────────

/// Scarica paper.jar dall'URL fornito con progress bar.
/// Se il file esiste già e il suo SHA256 coincide con quello atteso, salta il download.
/// Dopo il download, verifica sempre il SHA256 prima di considerare il file valido.
pub fn download_paper_jar(url: &str, expected_sha256: &str, server_dir: &Path) -> Result<()> {
    let jar_path = server_dir.join("paper.jar");

    if jar_path.exists() {
        let existing_hash = sha256_of_file(&jar_path)?;
        if existing_hash == expected_sha256 {
            println!("  paper.jar already present and verified, skipping download.");
            return Ok(());
        }
        println!("  paper.jar hash mismatch — re-downloading...");
    }

    println!("  Downloading Paper...");
    let response = reqwest::blocking::get(url)
        .with_context(|| format!("Failed to download Paper from {url}"))?
        .error_for_status()
        .context("PaperMC download returned an error")?;

    let total = response
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);

    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("  [{bar:40.magenta/white}] {bytes}/{total_bytes} ({eta})")
            .unwrap()
            .progress_chars("█▓░"),
    );

    let mut file = File::create(&jar_path)
        .with_context(|| format!("Failed to create {}", jar_path.display()))?;

    let mut hasher = Sha256::new();
    let mut src = pb.wrap_read(response);
    let mut buf = vec![0u8; 65536];

    loop {
        let n = src.read(&mut buf).context("Error reading download stream")?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n]).context("Error writing paper.jar")?;
    }

    pb.finish_and_clear();

    let actual_hash = hex::encode(hasher.finalize());
    if actual_hash != expected_sha256 {
        std::fs::remove_file(&jar_path).ok();
        bail!(
            "SHA256 mismatch for paper.jar!\n  expected: {expected_sha256}\n  got:      {actual_hash}"
        );
    }

    println!(
        "  paper.jar ready ({} MB) — SHA256 verified.",
        jar_path.metadata()?.len() / 1_048_576
    );
    Ok(())
}

fn sha256_of_file(path: &Path) -> Result<String> {
    let mut file =
        File::open(path).with_context(|| format!("Failed to open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 65536];
    loop {
        let n = file.read(&mut buf).context("Error reading file for hash")?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

// ─── Server icon ─────────────────────────────────────────────────────────────

pub fn download_server_icon(server_dir: &Path) -> Result<()> {
    let icon_path = server_dir.join("server-icon.png");
    if icon_path.exists() {
        return Ok(());
    }

    let mut response =
        reqwest::blocking::get(SERVER_ICON_URL).context("Failed to download server icon")?;
    let mut file = File::create(&icon_path)
        .with_context(|| format!("Failed to create {}", icon_path.display()))?;
    io::copy(&mut response, &mut file).context("Failed to write server-icon.png")?;
    Ok(())
}
