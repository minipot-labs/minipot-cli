use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

use crate::config::MinipotConfig;
use crate::paper::{download_paper_jar, download_server_icon};

/// Prepara l'ambiente server (directory, eula, paper.jar, server-icon) senza avviare Java.
/// Usato da `minipot prepare` (per il plugin IntelliJ) e da `minipot run`.
pub fn prepare_server(config: &MinipotConfig, server_dir: &Path) -> Result<()> {
    // [1/3] Cartella server
    println!("[1/3] Preparing server directory...");
    if !server_dir.exists() {
        fs::create_dir_all(server_dir)
            .with_context(|| format!("Failed to create {}", server_dir.display()))?;
    }
    let plugins_dir = server_dir.join("plugins");
    if !plugins_dir.exists() {
        fs::create_dir_all(&plugins_dir).context("Failed to create plugins directory")?;
    }
    let eula_path = server_dir.join("eula.txt");
    if !eula_path.exists() {
        fs::write(&eula_path, "eula=true\n").context("Failed to write eula.txt")?;
    }

    // [2/3] Paper JAR
    println!("[2/3] Checking Paper {}...", config.server.version);
    download_paper_jar(&config.server.version, server_dir)?;

    // [3/3] Icona server
    println!("[3/3] Checking server icon...");
    if let Err(e) = download_server_icon(server_dir) {
        eprintln!("Warning: could not download server icon: {e}");
    }

    Ok(())
}

/// Entry point per `minipot prepare`.
/// Prepara l'ambiente e sincronizza il plugin JAR se disponibile.
/// Pensato per essere invocato dal plugin IntelliJ prima di costruire la RunConfiguration.
pub fn execute() -> Result<()> {
    let config = MinipotConfig::load()?;

    if config.server.version.trim().is_empty() {
        anyhow::bail!(
            "No server version set in minipot.yml.\n\
             Open the file and set the `version` field (e.g. \"1.21.4\")."
        );
    }

    let server_dir = config.server_dir();
    prepare_server(&config, &server_dir)?;

    // Sincronizza il plugin JAR se esiste un build disponibile — non fatale
    if let Err(e) = crate::commands::sync::execute() {
        eprintln!("Note: plugin sync skipped — {e}");
    }

    println!("  Server environment ready.");
    Ok(())
}
