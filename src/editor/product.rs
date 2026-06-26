//! The subset of an editor's `product.json` we use to identify it.
//!
//! `product.json` is the authoritative identity of a VS Code OSS build: it tells
//! us the user-data directory name (`nameShort`), the extensions directory name
//! (`dataFolderName`), and the CLI application name (`applicationName`). Relying
//! on it means forks work without a hardcoded table.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

/// Identity fields read from an editor's `product.json`.
#[derive(Debug, Clone, Deserialize)]
pub struct Product {
    /// Short product name, e.g. `Code - OSS`, `VSCodium`, `Code`. Determines the
    /// user-data directory name (`~/.config/<nameShort>` on Linux).
    #[serde(rename = "nameShort")]
    pub name_short: String,

    /// Long product name, e.g. `Code - OSS`, `Visual Studio Code`.
    #[serde(rename = "nameLong", default)]
    pub name_long: String,

    /// CLI application name, e.g. `code-oss`, `codium`, `code`.
    #[serde(rename = "applicationName")]
    pub application_name: String,

    /// Data folder name under `$HOME`, e.g. `.vscode-oss`, `.vscode`. The default
    /// extensions directory is `$HOME/<dataFolderName>/extensions`.
    #[serde(rename = "dataFolderName")]
    pub data_folder_name: String,

    /// Release channel, e.g. `stable` or `insider`.
    #[serde(default)]
    pub quality: Option<String>,

    /// Build commit hash, when present.
    #[serde(default)]
    pub commit: Option<String>,
}

impl Product {
    /// Parse a `product.json` file.
    pub fn from_file(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("reading product.json at {}", path.display()))?;
        let product: Self = serde_json::from_str(&raw)
            .with_context(|| format!("parsing product.json at {}", path.display()))?;
        Ok(product)
    }
}
