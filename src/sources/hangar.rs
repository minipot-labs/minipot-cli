// Preso da mcman src/sources/hangar.rs
// Cambiamenti: App → SourceContext, Platform fisso a Paper, thiserror → anyhow.

use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use crate::downloadable::{CacheStrategy, ResolvedFile, SourceContext};

const API_V1: &str = "https://hangar.papermc.io/api/v1";

// ─── Tipi API Hangar (da mcman) ───────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Hash, Clone)]
#[serde(rename_all = "UPPERCASE")]
pub enum Platform {
    Paper,
    Waterfall,
    Velocity,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProjectVersion {
    pub name: String,
    pub downloads: HashMap<Platform, PlatformVersionDownload>,
    pub platform_dependencies: HashMap<Platform, Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase", untagged)]
pub enum PlatformVersionDownload {
    #[serde(rename_all = "camelCase")]
    Hangar {
        file_info: FileInfo,
        download_url: String,
    },
    #[serde(rename_all = "camelCase")]
    External {
        file_info: FileInfo,
        external_url: String,
    },
}

impl PlatformVersionDownload {
    pub fn get_url(&self) -> String {
        match self.clone() {
            Self::Hangar { download_url, .. } => download_url,
            Self::External { external_url, .. } => external_url,
        }
    }

    pub fn get_file_info(&self) -> FileInfo {
        match self.clone() {
            Self::Hangar { file_info, .. } | Self::External { file_info, .. } => file_info,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FileInfo {
    pub name: String,
    pub size_bytes: u64,
    pub sha256_hash: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct ProjectChannel {
    pub name: String,
    pub flags: HashSet<ChannelFlag>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Hash, Clone)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[allow(dead_code)]
pub enum ChannelFlag {
    Frozen,
    Unstable,
    Pinned,
    SendsNotifications,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Pagination {
    pub limit: u64,
    pub offset: u64,
    pub count: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProjectVersionsResponse {
    pub pagination: Pagination,
    pub result: Vec<ProjectVersion>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PlatformFilter {
    pub limit: u64,
    pub offset: u64,
    pub channel: Option<String>,
    pub platform: Option<Platform>,
}

impl Default for PlatformFilter {
    fn default() -> Self {
        Self {
            limit: 25,
            offset: 0,
            channel: None,
            platform: None,
        }
    }
}

// ─── Funzioni standalone (da mcman) ──────────────────────────────────────────

async fn fetch_project_versions(
    http_client: &reqwest::Client,
    id: &str,
    filter: Option<PlatformFilter>,
) -> Result<ProjectVersionsResponse> {
    let filter = filter.unwrap_or_default();
    let slug = id.split_once('/').map_or(id, |(_, s)| s);
    Ok(http_client
        .get(format!("{API_V1}/projects/{slug}/versions"))
        .query(&filter)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

async fn fetch_project_version(
    http_client: &reqwest::Client,
    id: &str,
    name: &str,
) -> Result<ProjectVersion> {
    let slug = id.split_once('/').map_or(id, |(_, s)| s);
    Ok(http_client
        .get(format!("{API_V1}/projects/{slug}/versions/{name}"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

/// Cerca la versione compatibile, gestisce la paginazione (da mcman).
async fn get_project_version(
    http_client: &reqwest::Client,
    id: &str,
    filter: Option<PlatformFilter>,
    platform_version: Option<String>,
    plugin_version: Option<&str>,
) -> Result<ProjectVersion> {
    let mut current_filter = filter.unwrap_or_default();

    let find_version = |versions: &[ProjectVersion]| -> Option<ProjectVersion> {
        let mut compatible = versions.iter().filter(|v| {
            if let (Some(platform), Some(pv)) = (&current_filter.platform, &platform_version) {
                v.platform_dependencies
                    .get(platform)
                    .map_or(false, |deps| deps.contains(pv))
            } else {
                true
            }
        });

        if let Some(pv) = plugin_version {
            compatible
                .find(|v| v.name == pv)
                .or_else(|| versions.iter().find(|v| v.name.contains(pv)))
                .cloned()
        } else {
            compatible.next().cloned()
        }
    };

    loop {
        let resp = fetch_project_versions(http_client, id, Some(current_filter.clone())).await?;

        if let Some(found) = find_version(&resp.result) {
            return Ok(found);
        }

        if (resp.result.len() as u64) < current_filter.limit {
            break;
        }

        current_filter.offset += current_filter.limit;
    }

    Err(anyhow!(
        "No compatible versions for Hangar project '{id}'{}",
        plugin_version.map_or(String::new(), |v| format!(" (requested: {v})"))
    ))
}

// ─── HangarAPI ────────────────────────────────────────────────────────────────

pub struct HangarAPI<'a>(pub &'a SourceContext);

impl HangarAPI<'_> {
    /// minipot supporta solo Paper — Platform sempre Paper.
    fn platform() -> Platform {
        Platform::Paper
    }

    fn platform_filter() -> PlatformFilter {
        PlatformFilter {
            platform: Some(Self::platform()),
            ..Default::default()
        }
    }

    async fn fetch_hangar_version(&self, id: &str, version: &str) -> Result<ProjectVersion> {
        let filter = Self::platform_filter();
        let platform_version = Some(self.0.mc_version.clone());

        if version == "latest" {
            get_project_version(
                &self.0.http_client,
                id,
                Some(filter),
                platform_version,
                None,
            )
            .await
        } else if version.contains('$') {
            let version = version
                .replace("${mcver}", &self.0.mc_version)
                .replace("${mcversion}", &self.0.mc_version);
            get_project_version(
                &self.0.http_client,
                id,
                Some(filter),
                platform_version,
                Some(&version),
            )
            .await
        } else {
            fetch_project_version(&self.0.http_client, id, version).await
        }
    }

    pub async fn resolve_source(&self, id: &str, version: &str) -> Result<ResolvedFile> {
        let version = self
            .fetch_hangar_version(id, version)
            .await
            .context("Fetching Hangar project version")?;

        let platform = Self::platform();
        let download = version
            .downloads
            .get(&platform)
            .ok_or_else(|| anyhow!("Platform PAPER not available for Hangar project '{id}'"))?;

        let file_info = download.get_file_info();
        let cached_file_path = format!("{id}/{}/{}", version.name, file_info.name);

        Ok(ResolvedFile {
            url: download.get_url(),
            filename: file_info.name,
            cache: CacheStrategy::File {
                namespace: "hangar".to_owned(),
                path: cached_file_path,
            },
            size: Some(file_info.size_bytes),
            hashes: BTreeMap::from([("sha256".to_owned(), file_info.sha256_hash)]),
        })
    }
}
