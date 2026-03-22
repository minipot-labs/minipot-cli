use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

use crate::downloadable::Downloadable;

pub const LOCK_FILE: &str = "minipot.lock";

/// Stato riprodotto del server: build Paper esatta + plugin pinned con hash.
/// Va committato nel repository insieme a minipot.yml.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MinipotLock {
    /// Versione MC a cui si riferisce il lock (es. "1.21.4"). Se diversa da minipot.yml → ri-risoluzione.
    #[serde(default)]
    pub paper_version: String,
    pub paper_build: u32,
    pub paper_sha256: String,
    pub paper_url: String,

    /// Plugin di dipendenza pinned. Ogni entry include la sorgente dichiarata in minipot.yml
    /// (usata per rilevare cambiamenti) e le coordinate esatte del file scaricato.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plugins: Vec<LockedPlugin>,
}

/// Versione pinned di un singolo plugin di dipendenza.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LockedPlugin {
    /// La sorgente dichiarata in minipot.yml. Se cambia → il plugin viene ri-risolto.
    pub source: Downloadable,
    pub filename: String,
    pub url: String,
    /// SHA256 calcolato al momento del download (sempre presente dopo il primo prepare).
    pub sha256: Option<String>,
    pub size: Option<u64>,
}

impl MinipotLock {
    pub fn load() -> Result<Option<Self>> {
        if !Path::new(LOCK_FILE).exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(LOCK_FILE).context("Failed to read minipot.lock")?;
        serde_json::from_str(&content)
            .context("Failed to parse minipot.lock")
            .map(Some)
    }

    pub fn save(&self) -> Result<()> {
        let json =
            serde_json::to_string_pretty(self).context("Failed to serialize minipot.lock")?;
        fs::write(LOCK_FILE, json + "\n").context("Failed to write minipot.lock")
    }
}
