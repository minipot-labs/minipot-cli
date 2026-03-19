use anyhow::{anyhow, Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::path::Path;

const PAPER_API: &str = "https://qing762.is-a.dev/api/papermc";
const SERVER_ICON_URL: &str =
    "https://minipot-assets.s3.eu-central-1.amazonaws.com/minipot-icon-server.png";

/// Struttura della risposta dell'API PaperMC
#[derive(Deserialize, Debug)]
pub struct PaperApiResponse {
    #[allow(dead_code)]
    pub latest: String,
    pub versions: HashMap<String, String>,
}

impl PaperApiResponse {
    pub fn fetch() -> Result<Self> {
        let resp = reqwest::blocking::get(PAPER_API)
            .context("Failed to reach PaperMC API")?
            .json::<PaperApiResponse>()
            .context("Failed to parse PaperMC API response")?;
        Ok(resp)
    }

    pub fn download_url(&self, version: &str) -> Result<&str> {
        self.versions
            .get(version)
            .map(String::as_str)
            .ok_or_else(|| anyhow!("Version '{}' not found in PaperMC API", version))
    }
}

/// Scarica il server-icon.png di Minipot nella cartella server.
/// Se il file esiste già, non lo riscarica.
pub fn download_server_icon(server_dir: &Path) -> Result<()> {
    let icon_path = server_dir.join("server-icon.png");
    if icon_path.exists() {
        return Ok(());
    }

    let mut response = reqwest::blocking::get(SERVER_ICON_URL)
        .context("Failed to download server icon")?;
    let mut file = File::create(&icon_path)
        .with_context(|| format!("Failed to create {}", icon_path.display()))?;
    io::copy(&mut response, &mut file).context("Failed to write server-icon.png")?;
    Ok(())
}

/// Scarica il JAR Paper per la versione richiesta nella cartella di destinazione.
/// Se il file esiste già, non lo riscarica.
pub fn download_paper_jar(version: &str, server_dir: &Path) -> Result<()> {
    let jar_path = server_dir.join("paper.jar");

    if jar_path.exists() {
        println!("paper.jar already present, skipping download.");
        return Ok(());
    }

    println!("  Fetching PaperMC API...");
    let api = PaperApiResponse::fetch()?;
    let url = api.download_url(version)?;

    println!("  Downloading Paper {version}...");
    let response = reqwest::blocking::get(url)
        .with_context(|| format!("Failed to download Paper {version}"))?;

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

    let mut src = pb.wrap_read(response);
    io::copy(&mut src, &mut file).context("Failed to write paper.jar")?;

    pb.finish_and_clear();
    println!("  paper.jar ready ({} MB).", jar_path.metadata()?.len() / 1_048_576);
    Ok(())
}
