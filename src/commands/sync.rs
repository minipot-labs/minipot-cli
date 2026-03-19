use anyhow::{Context, Result};
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use crate::config::MinipotConfig;

pub fn execute() -> Result<()> {
    let config = MinipotConfig::load()?;
    let server_dir = config.server_dir();

    // Verifica che il server esista e sia pronto
    if !server_dir.join("paper.jar").exists() {
        anyhow::bail!(
            "Server not initialized at {}. Run `minipot run` first.",
            server_dir.display()
        );
    }

    // Trova il JAR più recente tra build/libs/ (Gradle) e target/ (Maven)
    let jar_path = find_latest_jar_any(&[Path::new("build/libs"), Path::new("target")])
        .context("No plugin jar found in build/libs/ or target/ — build your plugin first")?;

    println!("Found jar: {}", jar_path.display());

    // Leggi il nome del plugin da plugin.yml dentro il JAR
    let plugin_name = read_plugin_name(&jar_path);
    match &plugin_name {
        Some(name) => println!("Plugin name: {name}"),
        None => println!("Warning: could not read plugin.yml from jar, skipping old-version cleanup"),
    }

    let plugins_dir = server_dir.join("plugins");
    if !plugins_dir.exists() {
        fs::create_dir_all(&plugins_dir).context("Failed to create plugins directory")?;
    }

    // Rimuovi versioni precedenti dello stesso plugin
    if let Some(name) = &plugin_name {
        remove_old_plugin_jars(&plugins_dir, name)?;
    }

    // Copia il JAR nella cartella plugins
    let dest = plugins_dir.join(jar_path.file_name().unwrap());
    fs::copy(&jar_path, &dest)
        .with_context(|| format!("Failed to copy jar to {}", dest.display()))?;

    println!("Deployed {} -> {}", jar_path.display(), dest.display());
    Ok(())
}

/// Cerca il JAR più recente tra più directory candidate (Gradle: build/libs/, Maven: target/)
fn find_latest_jar_any(dirs: &[&Path]) -> Option<PathBuf> {
    dirs.iter()
        .filter_map(|dir| find_latest_jar(dir))
        .max_by_key(|p| {
            p.metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        })
}

/// Trova il JAR modificato più di recente in `dir`
fn find_latest_jar(dir: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(dir).ok()?;

    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|e| e == "jar").unwrap_or(false))
        .max_by_key(|p| {
            p.metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        })
}

/// Legge il campo `name:` da plugin.yml dentro il JAR (che è un archivio ZIP)
fn read_plugin_name(jar_path: &Path) -> Option<String> {
    let file = File::open(jar_path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let entry = archive.by_name("plugin.yml").ok()?;

    BufReader::new(entry).lines().flatten().find_map(|line| {
        let trimmed = line.trim().to_string();
        if trimmed.to_lowercase().starts_with("name:") {
            let value = trimmed.splitn(2, ':').nth(1)?.trim().to_string();
            let name = value
                .trim_matches('"')
                .trim_matches('\'')
                .trim()
                .to_string();
            if name.is_empty() { None } else { Some(name) }
        } else {
            None
        }
    })
}

/// Rimuove dalla cartella plugins tutti i JAR che hanno lo stesso nome plugin
fn remove_old_plugin_jars(plugins_dir: &Path, plugin_name: &str) -> Result<()> {
    for entry in fs::read_dir(plugins_dir).context("Failed to read plugins dir")? {
        let path = entry?.path();
        if path.extension().map(|e| e == "jar").unwrap_or(false) {
            if let Some(existing_name) = read_plugin_name(&path) {
                if existing_name.eq_ignore_ascii_case(plugin_name) {
                    fs::remove_file(&path)
                        .with_context(|| format!("Failed to remove {}", path.display()))?;
                    println!("Removed old version: {}", path.display());
                }
            }
        }
    }
    Ok(())
}
