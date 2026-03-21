use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::downloadable::Downloadable;

pub const CONFIG_FILE: &str = "minipot.yml";

#[derive(Serialize, Deserialize, Debug)]
pub struct MinipotConfig {
    pub server: ServerConfig,
    pub bots: Vec<BotConfig>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ServerConfig {
    pub version: String,
    #[serde(rename = "type")]
    pub server_type: String,
    pub port: u16,
    #[serde(default)]
    pub plugins: Vec<Downloadable>,
    pub jvm_flags: Vec<String>,
    /// Comandi inviati automaticamente alla console Paper appena il server è pronto.
    #[serde(default)]
    pub startup_commands: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct BotConfig {
    pub name: String,
    pub script: Option<String>,
}

#[allow(dead_code)]
impl MinipotConfig {
    pub fn default() -> Self {
        MinipotConfig {
            server: ServerConfig {
                version: "1.21.4".to_string(),
                server_type: "paper".to_string(),
                port: 25565,
                plugins: vec![],  // Vec<Downloadable>
                jvm_flags: vec![
                    "-Xms512M".to_string(),
                    "-Xmx2G".to_string(),
                    "-XX:+UseG1GC".to_string(),
                ],
                startup_commands: vec![],
            },
            bots: vec![],
        }
    }

    pub fn load() -> Result<Self> {
        let content = fs::read_to_string(CONFIG_FILE)
            .with_context(|| format!("Cannot find {CONFIG_FILE} — run `minipot init` first"))?;
        serde_yaml::from_str(&content).context("Failed to parse minipot.yml")
    }

    pub fn save(&self) -> Result<()> {
        let yaml = serde_yaml::to_string(self).context("Failed to serialize config")?;
        fs::write(CONFIG_FILE, yaml).context("Failed to write minipot.yml")
    }

    /// Percorso della cartella server relativo alla cwd
    pub fn server_dir(&self) -> PathBuf {
        Path::new("minipot-server").to_path_buf()
    }
}
