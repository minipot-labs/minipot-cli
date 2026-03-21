// Preso da mcman src/sources/modrinth.rs
// Cambiamenti minimi: App → SourceContext, metodi specifici al server type rimossi/hardcodati per Paper.

use std::{collections::BTreeMap, time::Duration};

use anyhow::{anyhow, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::time::sleep;

use crate::downloadable::{CacheStrategy, ResolvedFile, SourceContext};

static API_URL: &str = "https://api.modrinth.com/v2";

// ─── Tipi API Modrinth (da mcman) ─────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ModrinthVersion {
    pub name: String,
    pub version_number: String,
    pub changelog: String,
    pub dependencies: Vec<ModrinthDependency>,
    pub game_versions: Vec<String>,
    pub version_type: VersionType,
    pub loaders: Vec<String>,
    pub featured: bool,
    pub status: ModrinthStatus,
    pub id: String,
    pub project_id: String,
    pub files: Vec<ModrinthFile>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ModrinthDependency {
    pub version_id: Option<String>,
    pub project_id: Option<String>,
    pub file_name: Option<String>,
    pub dependency_type: Option<DependencyType>,
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DependencyType {
    Required,
    Optional,
    Incompatible,
    Embedded,
    Unsupported,
    Unknown,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum VersionType {
    Release,
    Beta,
    Alpha,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum ModrinthStatus {
    Listed,
    Archived,
    Draft,
    Unlisted,
    Scheduled,
    Unknown,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ModrinthFile {
    pub hashes: BTreeMap<String, String>,
    pub url: String,
    pub filename: String,
    pub primary: bool,
    pub size: u64,
}

// ─── Rate limit (da mcman) ────────────────────────────────────────────────────

pub trait ModrinthWaitRatelimit<T> {
    async fn wait_ratelimit(self) -> Result<T>;
}

impl ModrinthWaitRatelimit<reqwest::Response> for reqwest::Response {
    async fn wait_ratelimit(self) -> Result<Self> {
        let res = if let Some(h) = self.headers().get("x-ratelimit-remaining") {
            if String::from_utf8_lossy(h.as_bytes()) == "1" {
                let ratelimit_reset =
                    String::from_utf8_lossy(self.headers()["x-ratelimit-reset"].as_bytes())
                        .parse::<u64>()?;
                println!(" (!) Modrinth ratelimit — waiting {ratelimit_reset}s...");
                sleep(Duration::from_secs(ratelimit_reset)).await;
            }
            self
        } else {
            self.error_for_status()?
        };
        Ok(res)
    }
}

// ─── ModrinthAPI ──────────────────────────────────────────────────────────────

pub struct ModrinthAPI<'a>(pub &'a SourceContext);

impl ModrinthAPI<'_> {
    async fn fetch_api<T: DeserializeOwned>(&self, url: &str) -> Result<T> {
        let json: T = self
            .0
            .http_client
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .wait_ratelimit()
            .await?
            .json()
            .await?;
        Ok(json)
    }

    async fn fetch_all_versions(&self, id: &str) -> Result<Vec<ModrinthVersion>> {
        self.fetch_api(&format!("{API_URL}/project/{id}/version"))
            .await
    }

    /// Filtra le versioni compatibili con Paper e la versione MC corrente.
    /// Più inclusivo di mcman: accetta paper, spigot, bukkit, purpur, folia.
    fn filter_versions(&self, list: &[ModrinthVersion]) -> Vec<ModrinthVersion> {
        let mc_ver = self.0.mc_version.as_str();
        list.iter()
            .filter(|v| v.game_versions.iter().any(|s| s.as_str() == mc_ver))
            .filter(|v| {
                v.loaders.iter().any(|l| {
                    matches!(
                        l.as_str(),
                        "paper" | "spigot" | "bukkit" | "purpur" | "folia"
                    )
                })
            })
            .cloned()
            .collect()
    }

    async fn fetch_version(&self, id: &str, version: &str) -> Result<ModrinthVersion> {
        let all_versions = self.fetch_all_versions(id).await?;
        let versions = self.filter_versions(&all_versions);

        let version_data = if let Some(v) = match version {
            "latest" => versions.first(),
            ver => versions
                .iter()
                .find(|v| v.id == ver || v.name == ver || v.version_number == ver),
        } {
            v.clone()
        } else {
            // Fallback senza filtro loader (da mcman)
            let v = match version {
                "latest" => all_versions.first(),
                ver => all_versions
                    .iter()
                    .find(|v| v.id == ver || v.name == ver || v.version_number == ver),
            }
            .ok_or(anyhow!(
                "Version '{version}' not found for Modrinth project '{id}'"
            ))?
            .clone();
            eprintln!("Warning: loader filter failed for modrinth:{id}@{version}, using unfiltered");
            v
        };

        Ok(version_data)
    }

    async fn fetch_file(&self, id: &str, version: &str) -> Result<(ModrinthFile, ModrinthVersion)> {
        let version = self.fetch_version(id, version).await?;
        let file = version
            .files
            .iter()
            .find(|f| f.primary)
            .or(version.files.first())
            .ok_or(anyhow!(
                "No file found for modrinth:{id}/{}",
                version.id
            ))?
            .clone();
        Ok((file, version))
    }

    pub async fn resolve_source(&self, id: &str, version: &str) -> Result<ResolvedFile> {
        let (file, version) = self.fetch_file(id, version).await?;
        let cached_file_path = format!("{id}/{}/{}", version.id, file.filename);

        Ok(ResolvedFile {
            url: file.url,
            filename: file.filename,
            cache: CacheStrategy::File {
                namespace: "modrinth".to_owned(),
                path: cached_file_path,
            },
            size: Some(file.size),
            hashes: file.hashes,
        })
    }
}
