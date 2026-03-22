use std::{collections::BTreeMap, path::{Path, PathBuf}, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use sha2::{Digest, Sha256};
use tokio::task::JoinSet;

use crate::{
    cache::Cache,
    config::MinipotConfig,
    downloadable::{CacheStrategy, Downloadable, Resolvable, ResolvedFile, SourceContext},
    lock::{LockedPlugin, MinipotLock},
    paper::{download_paper_jar, download_server_icon, resolve_latest_build},
};

// ─── prepare_server ──────────────────────────────────────────────────────────

/// Prepara l'ambiente server (directory, eula, paper.jar, plugin, server-icon) senza avviare Java.
/// Usato da `minipot prepare` e da `minipot run`.
pub fn prepare_server(config: &MinipotConfig, server_dir: &Path) -> Result<()> {
    // [1/4] Cartella server
    println!("[1/4] Preparing server directory...");
    if !server_dir.exists() {
        std::fs::create_dir_all(server_dir)
            .with_context(|| format!("Failed to create {}", server_dir.display()))?;
    }
    let plugins_dir = server_dir.join("plugins");
    if !plugins_dir.exists() {
        std::fs::create_dir_all(&plugins_dir).context("Failed to create plugins directory")?;
    }
    let eula_path = server_dir.join("eula.txt");
    if !eula_path.exists() {
        std::fs::write(&eula_path, "eula=true\n").context("Failed to write eula.txt")?;
    }

    // [2/4] Paper JAR — risolve o usa il lockfile
    println!("[2/4] Checking Paper {}...", config.server.version);
    let mut lock = MinipotLock::load()?.unwrap_or_else(|| {
        // Placeholder: verrà popolato subito sotto
        MinipotLock {
            paper_version: String::new(),
            paper_build: 0,
            paper_sha256: String::new(),
            paper_url: String::new(),
            plugins: vec![],
        }
    });

    let version_changed = lock.paper_version != config.server.version;

    let (paper_url, paper_sha256) = if lock.paper_build != 0 && !version_changed {
        println!("  Using locked build #{} (minipot.lock).", lock.paper_build);
        (lock.paper_url.clone(), lock.paper_sha256.clone())
    } else {
        if version_changed && lock.paper_build != 0 {
            println!(
                "  Version changed ({} → {}) — re-resolving Paper build...",
                lock.paper_version, config.server.version
            );
        } else {
            println!("  No lock found — resolving latest Paper build...");
        }
        let build = resolve_latest_build(&config.server.version)?;
        println!("  Resolved build #{}.", build.build);
        lock.paper_version = config.server.version.clone();
        lock.paper_build = build.build;
        lock.paper_sha256 = build.sha256.clone();
        lock.paper_url = build.url.clone();
        (build.url, build.sha256)
    };

    download_paper_jar(&paper_url, &paper_sha256, server_dir)?;

    // [3/4] Plugin di dipendenza (async + parallel)
    if !config.server.plugins.is_empty() {
        println!("[3/4] Downloading {} plugin dependency/ies...", config.server.plugins.len());
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .context("Failed to create tokio runtime")?;

        let new_plugins = rt.block_on(download_plugins(
            &config.server.plugins,
            &lock.plugins,
            &config.server.version,
            &plugins_dir,
        ))?;

        // Pulizia: rimuovi JAR che erano nel lock ma non ci sono più
        let new_filenames: std::collections::HashSet<&str> =
            new_plugins.iter().map(|p| p.filename.as_str()).collect();
        for old in &lock.plugins {
            if !new_filenames.contains(old.filename.as_str()) {
                let path = plugins_dir.join(&old.filename);
                if path.exists() {
                    std::fs::remove_file(&path).ok();
                    println!("  Removed old plugin: {}", old.filename);
                }
            }
        }

        lock.plugins = new_plugins;
    } else {
        println!("[3/4] No dependency plugins declared.");
    }

    // Scrivi lock aggiornato
    lock.save()?;
    if lock.paper_build != 0 {
        println!("  minipot.lock updated — commit this file alongside minipot.yml.");
    }

    // [4/4] Icona server
    println!("[4/4] Checking server icon...");
    if let Err(e) = download_server_icon(server_dir) {
        eprintln!("Warning: could not download server icon: {e}");
    }

    Ok(())
}

// ─── Download plugin parallelo ────────────────────────────────────────────────

/// Scarica tutti i plugin dichiarati in parallelo.
/// Restituisce i LockedPlugin aggiornati. Preso concettualmente da mcman core/addons.rs.
async fn download_plugins(
    plugins: &[Downloadable],
    locked_plugins: &[LockedPlugin],
    mc_version: &str,
    plugins_dir: &Path,
) -> Result<Vec<LockedPlugin>> {
    let ctx = Arc::new(SourceContext::new(mc_version)?);
    let mp = Arc::new(MultiProgress::new());
    let plugins_dir = plugins_dir.to_path_buf();

    let mut set: JoinSet<Result<LockedPlugin>> = JoinSet::new();

    for plugin in plugins {
        let plugin = plugin.clone();
        let locked = locked_plugins.iter().find(|p| p.source == plugin).cloned();
        let ctx = Arc::clone(&ctx);
        let mp = Arc::clone(&mp);
        let plugins_dir = plugins_dir.clone();

        let pb = mp.add(ProgressBar::new_spinner());
        pb.set_style(
            ProgressStyle::with_template("{spinner:.cyan} {msg}")
                .unwrap()
                .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
        );
        pb.enable_steady_tick(Duration::from_millis(80));
        pb.set_message(format!("Resolving {}...", plugin));

        set.spawn(async move {
            download_single_plugin(&plugin, locked.as_ref(), &ctx, &plugins_dir, pb).await
        });
    }

    let mut results = Vec::with_capacity(plugins.len());
    while let Some(join_result) = set.join_next().await {
        match join_result {
            Ok(Ok(locked)) => results.push(locked),
            Ok(Err(e)) => {
                set.abort_all();
                return Err(e);
            }
            Err(join_err) => {
                set.abort_all();
                return Err(anyhow::anyhow!("Download task panicked: {join_err}"));
            }
        }
    }

    Ok(results)
}

/// Scarica (o verifica) un singolo plugin. Controlla cache prima di scaricare.
async fn download_single_plugin(
    plugin: &Downloadable,
    locked: Option<&LockedPlugin>,
    ctx: &SourceContext,
    plugins_dir: &Path,
    pb: ProgressBar,
) -> Result<LockedPlugin> {
    // Ottieni ResolvedFile: dal lock se disponibile, altrimenti dall'API
    let resolved = match locked {
        Some(lock) => {
            pb.set_message(format!("Checking {} (locked)...", lock.filename));
            let mut hashes = BTreeMap::new();
            if let Some(ref sha) = lock.sha256 {
                hashes.insert("sha256".to_owned(), sha.clone());
            }
            ResolvedFile {
                url: lock.url.clone(),
                filename: lock.filename.clone(),
                size: lock.size,
                hashes,
                cache: derive_cache_strategy(plugin, &lock.filename),
            }
        }
        None => {
            pb.set_message(format!("Resolving {}...", plugin));
            plugin.resolve_source(ctx).await?
        }
    };

    pb.set_message(format!("  {}", resolved.filename));
    let sha256 = download_resolved_file(&ctx.http_client, &resolved, plugins_dir, &pb).await?;

    pb.finish_and_clear();

    Ok(LockedPlugin {
        source: plugin.clone(),
        filename: resolved.filename,
        url: resolved.url,
        sha256: Some(sha256),
        size: resolved.size,
    })
}

/// Scarica un file risolto in `dest_dir`, usando la cache locale se disponibile.
/// Calcola sempre SHA256 (dal file esistente, dalla cache, o dal download).
/// Struttura analoga a mcman app/downloading.rs::download_resolved().
async fn download_resolved_file(
    http_client: &reqwest::Client,
    resolved: &ResolvedFile,
    dest_dir: &Path,
    pb: &ProgressBar,
) -> Result<String> {
    let dest_path = dest_dir.join(&resolved.filename);
    let expected_sha256 = resolved.hashes.get("sha256").cloned();

    // File già presente: verifica hash
    if dest_path.exists() {
        let actual = sha256_of_file(&dest_path).await?;
        let hash_ok = expected_sha256.as_deref().map_or(true, |exp| exp == actual);
        if hash_ok {
            pb.set_message(format!("✓ {} (already present)", resolved.filename));
            return Ok(actual);
        }
        // Hash sbagliato: re-download
    }

    // Controlla cache locale (~/.cache/minipot/)
    let cache_entry: Option<(PathBuf, bool)> = match &resolved.cache {
        CacheStrategy::File { namespace, path } => Cache::get(namespace)
            .map(|c| (c.path(path), c.exists(path))),
        CacheStrategy::None => None,
    };

    if let Some((ref cache_path, true)) = cache_entry {
        let cache_sha = sha256_of_file(cache_path).await?;
        let hash_ok = expected_sha256.as_deref().map_or(true, |exp| exp == cache_sha);
        if hash_ok {
            tokio::fs::create_dir_all(dest_path.parent().unwrap()).await?;
            tokio::fs::copy(cache_path, &dest_path).await?;
            pb.set_message(format!("✓ {} (from cache)", resolved.filename));
            return Ok(cache_sha);
        }
    }

    // Download
    pb.set_message(format!("Downloading {}...", resolved.filename));
    let response = http_client
        .get(&resolved.url)
        .send()
        .await
        .with_context(|| format!("Failed to download {}", resolved.filename))?
        .error_for_status()
        .with_context(|| format!("HTTP error downloading {}", resolved.filename))?;

    let bytes = response
        .bytes()
        .await
        .context("Failed to read download response")?;

    // Calcola SHA256
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let actual_sha = hex::encode(hasher.finalize());

    // Verifica hash se disponibile dalla sorgente
    if let Some(ref expected) = expected_sha256 {
        if &actual_sha != expected {
            anyhow::bail!(
                "SHA256 mismatch for {}!\n  expected: {}\n  got:      {}",
                resolved.filename,
                expected,
                actual_sha
            );
        }
    }

    // Salva in cache
    if let Some((ref cache_path, _)) = cache_entry {
        if let Some(parent) = cache_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(cache_path, &bytes).await?;
    }

    // Scrivi in dest
    tokio::fs::create_dir_all(dest_path.parent().unwrap()).await?;
    tokio::fs::write(&dest_path, &bytes).await?;

    pb.set_message(format!("✓ {} ({} KB)", resolved.filename, bytes.len() / 1024));
    Ok(actual_sha)
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Deriva la CacheStrategy dal tipo di sorgente e dal filename.
/// Usato per ricostruire la chiave cache da un LockedPlugin senza ri-chiamare l'API.
fn derive_cache_strategy(plugin: &Downloadable, filename: &str) -> CacheStrategy {
    match plugin {
        Downloadable::Modrinth { id, .. } => CacheStrategy::File {
            namespace: "modrinth".to_owned(),
            path: format!("{id}/{filename}"),
        },
        Downloadable::Hangar { id, .. } => CacheStrategy::File {
            namespace: "hangar".to_owned(),
            path: format!("{id}/{filename}"),
        },
        Downloadable::GithubRelease { repo, .. } => CacheStrategy::File {
            namespace: "github".to_owned(),
            path: format!("{repo}/{filename}"),
        },
        Downloadable::Url { .. } => CacheStrategy::None,
    }
}

async fn sha256_of_file(path: &Path) -> Result<String> {
    let bytes = tokio::fs::read(path)
        .await
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(hex::encode(hasher.finalize()))
}

// ─── execute ─────────────────────────────────────────────────────────────────

/// Entry point per `minipot prepare`.
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

    if let Err(e) = crate::commands::sync::execute() {
        eprintln!("Note: plugin sync skipped — {e}");
    }

    println!("  Server environment ready.");
    Ok(())
}
