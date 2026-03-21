use std::{collections::BTreeMap, fmt};

use anyhow::Result;
use serde::{Deserialize, Serialize};

// ─── Contesto passato ai resolver ────────────────────────────────────────────

#[derive(Clone)]
pub struct SourceContext {
    pub http_client: reqwest::Client,
    pub mc_version: String,
}

impl SourceContext {
    pub fn new(mc_version: impl Into<String>) -> Result<Self> {
        Ok(Self {
            http_client: reqwest::Client::builder()
                .user_agent(concat!("minipot-cli/", env!("CARGO_PKG_VERSION")))
                .build()?,
            mc_version: mc_version.into(),
        })
    }
}

// ─── ResolvedFile + CacheStrategy ────────────────────────────────────────────

/// File risolto da una sorgente: URL, filename, hash, strategia di cache.
/// Struttura analoga a mcman — mantiene compatibilità concettuale.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedFile {
    pub url: String,
    pub filename: String,
    pub size: Option<u64>,
    pub hashes: BTreeMap<String, String>,
    pub cache: CacheStrategy,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(tag = "type")]
pub enum CacheStrategy {
    File {
        namespace: String,
        path: String,
    },
    #[default]
    None,
}

// ─── Trait Resolvable ─────────────────────────────────────────────────────────

pub trait Resolvable {
    async fn resolve_source(&self, ctx: &SourceContext) -> Result<ResolvedFile>;
}

// ─── Downloadable ─────────────────────────────────────────────────────────────

fn latest() -> String {
    "latest".to_owned()
}

fn first() -> String {
    "first".to_owned()
}

/// Sorgente dichiarata per una dipendenza in minipot.yml.
/// Struttura modellata su mcman — stesse varianti, stesso schema di serializzazione.
#[derive(Debug, Deserialize, Serialize, Clone, Hash, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Downloadable {
    /// URL diretto. Nessuna chiamata API, nessuna cache.
    Url {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
    },

    /// Modrinth — il registro mod/plugin più diffuso.
    #[serde(alias = "mr")]
    Modrinth {
        id: String,
        #[serde(default = "latest")]
        version: String,
    },

    /// Hangar — registro ufficiale PaperMC.
    Hangar {
        id: String,
        #[serde(default = "latest")]
        version: String,
    },

    /// GitHub Releases.
    #[serde(rename = "ghrel")]
    GithubRelease {
        repo: String,
        #[serde(default = "latest")]
        tag: String,
        #[serde(default = "first")]
        asset: String,
    },
}

impl fmt::Display for Downloadable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Url { url, .. } => write!(f, "{url}"),
            Self::Modrinth { id, version } => write!(f, "modrinth:{id}@{version}"),
            Self::Hangar { id, version } => write!(f, "hangar:{id}@{version}"),
            Self::GithubRelease { repo, tag, asset } => {
                write!(f, "github:{repo}@{tag}/{asset}")
            }
        }
    }
}

impl Resolvable for Downloadable {
    async fn resolve_source(&self, ctx: &SourceContext) -> Result<ResolvedFile> {
        match self {
            Self::Url { url, filename } => Ok(ResolvedFile {
                url: url.clone(),
                filename: if let Some(f) = filename {
                    f.clone()
                } else {
                    let clean = url.split('?').next().unwrap_or(url);
                    clean
                        .split('/')
                        .next_back()
                        .unwrap_or("plugin.jar")
                        .to_string()
                },
                cache: CacheStrategy::None,
                size: None,
                hashes: BTreeMap::new(),
            }),
            Self::Modrinth { id, version } => {
                crate::sources::modrinth::ModrinthAPI(ctx)
                    .resolve_source(id, version)
                    .await
            }
            Self::Hangar { id, version } => {
                crate::sources::hangar::HangarAPI(ctx)
                    .resolve_source(id, version)
                    .await
            }
            Self::GithubRelease { repo, tag, asset } => {
                crate::sources::github::GithubAPI(ctx)
                    .resolve_source(repo, tag, asset)
                    .await
            }
        }
    }
}
