// Preso da mcman src/sources/github.rs
// Cambiamenti: App → SourceContext, api_token rimosso (aggiungibile in futuro),
// api_url hardcodato, cache ETag mantenuta (GitHub ha rate limit 60 req/h senza auth).

use std::{
    collections::BTreeMap,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{anyhow, Context, Result};
use reqwest::{
    header::{HeaderMap, HeaderValue},
    StatusCode,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::time::sleep;

use crate::{
    cache::Cache,
    downloadable::{CacheStrategy, ResolvedFile, SourceContext},
};

static CACHE_NAMESPACE: &str = "github";
static GITHUB_API_BASE: &str = "https://api.github.com";
static GITHUB_API_VERSION: &str = "2022-11-28";

// ─── Tipi API GitHub (da mcman) ───────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct CachedData<T: Serialize> {
    pub data: T,
    pub etag: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GithubRelease {
    pub tag_name: String,
    pub name: String,
    pub assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GithubAsset {
    pub url: String,
    pub name: String,
    pub size: u64,
}

// ─── Rate limit (da mcman) ────────────────────────────────────────────────────

pub trait GithubWaitRatelimit<T> {
    async fn wait_ratelimit(self) -> Result<T>;
}

impl GithubWaitRatelimit<reqwest::Response> for reqwest::Response {
    async fn wait_ratelimit(self) -> Result<Self> {
        Ok(match self.headers().get("x-ratelimit-remaining") {
            Some(h) => {
                if String::from_utf8_lossy(h.as_bytes()) == "1" {
                    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
                    let reset = String::from_utf8_lossy(
                        self.headers()["x-ratelimit-reset"].as_bytes(),
                    )
                    .parse::<u64>()?;
                    let wait = reset.saturating_sub(now);
                    println!(" (!) GitHub ratelimit — waiting {wait}s...");
                    sleep(Duration::from_secs(wait)).await;
                }
                self
            }
            None => self.error_for_status()?,
        })
    }
}

// ─── GithubAPI ────────────────────────────────────────────────────────────────

pub struct GithubAPI<'a>(pub &'a SourceContext);

impl GithubAPI<'_> {
    /// Fetch con ETag cache (da mcman) — evita di bruciare le 60 req/h senza token.
    async fn fetch_api<T: DeserializeOwned + Clone + Serialize>(
        &self,
        path: &str,
        cache_path: &str,
    ) -> Result<T> {
        let cached_data = Cache::get(CACHE_NAMESPACE)
            .and_then(|c| c.try_get_json::<CachedData<T>>(cache_path).ok().flatten());

        let mut headers = HeaderMap::new();
        if let Some(ref cached) = cached_data {
            if let Ok(v) = HeaderValue::from_str(&cached.etag) {
                headers.insert("if-none-match", v);
            }
        }
        headers.insert(
            "X-GitHub-Api-Version",
            HeaderValue::from_str(GITHUB_API_VERSION)?,
        );

        let response = self
            .0
            .http_client
            .get(format!("{GITHUB_API_BASE}/{path}"))
            .headers(headers)
            .send()
            .await?;

        if response.status() == StatusCode::NOT_MODIFIED {
            return Ok(cached_data.unwrap().data);
        }

        let etag = response.headers().get("etag").cloned();
        let json: T = response
            .error_for_status()?
            .wait_ratelimit()
            .await?
            .json()
            .await?;

        if let (Some(etag), Some(cache)) = (etag, Cache::get(CACHE_NAMESPACE)) {
            let _ = cache.write_json(
                cache_path,
                &CachedData {
                    etag: etag.to_str().unwrap_or_default().to_owned(),
                    data: json.clone(),
                },
            );
        }

        Ok(json)
    }

    async fn fetch_releases(&self, repo: &str) -> Result<Vec<GithubRelease>> {
        self.fetch_api::<Vec<GithubRelease>>(
            &format!("repos/{repo}/releases"),
            &format!("{repo}/releases.json"),
        )
        .await
    }

    async fn fetch_release(&self, repo: &str, tag: &str) -> Result<GithubRelease> {
        let releases = self.fetch_releases(repo).await?;

        let tag_resolved = tag
            .replace("${mcver}", &self.0.mc_version)
            .replace("${mcversion}", &self.0.mc_version);

        let release = match tag_resolved.as_str() {
            "latest" => releases.first(),
            t => releases
                .iter()
                .find(|r| r.tag_name == t)
                .or_else(|| releases.iter().find(|r| r.tag_name.contains(t))),
        }
        .ok_or_else(|| anyhow!("Release '{tag}' not found on {repo}"))?;

        Ok(release.clone())
    }

    async fn fetch_asset(
        &self,
        repo: &str,
        tag: &str,
        asset_name: &str,
    ) -> Result<(GithubRelease, GithubAsset)> {
        let release = self.fetch_release(repo, tag).await?;

        let asset = match asset_name {
            "" | "first" | "any" => release.assets.first(),
            name => {
                let name = if name.contains('$') {
                    name.replace("${version}", &release.tag_name)
                        .replace("${tag}", &release.tag_name)
                        .replace("${mcver}", &self.0.mc_version)
                } else {
                    name.to_owned()
                };
                release
                    .assets
                    .iter()
                    .find(|a| a.name == name)
                    .or_else(|| release.assets.iter().find(|a| a.name.contains(&name)))
            }
        }
        .ok_or_else(|| {
            anyhow!(
                "Asset '{asset_name}' not found in release '{}' of {repo}",
                release.tag_name
            )
        })?
        .clone();

        Ok((release, asset))
    }

    pub async fn resolve_source(
        &self,
        repo: &str,
        tag: &str,
        asset_name: &str,
    ) -> Result<ResolvedFile> {
        let (release, asset) = self
            .fetch_asset(repo, tag, asset_name)
            .await
            .context("Fetching GitHub release asset")?;

        let cached_file_path = format!("{repo}/releases/{}/{}", release.tag_name, asset.name);

        Ok(ResolvedFile {
            url: format!(
                "https://github.com/{repo}/releases/download/{}/{}",
                release.tag_name, asset.name
            ),
            filename: asset.name,
            cache: CacheStrategy::File {
                namespace: CACHE_NAMESPACE.to_owned(),
                path: cached_file_path,
            },
            size: Some(asset.size),
            hashes: BTreeMap::new(), // GitHub non espone hash nell'API releases
        })
    }
}
