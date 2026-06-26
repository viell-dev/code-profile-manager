//! The declarative TOML config: desired state for an editor's profiles.
//!
//! The config layers `[global]` settings/extensions, reusable `[groups.*]`, and
//! per-`[profiles.*]` overrides into an effective desired state per profile (see
//! [`resolve`]).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Top-level config document.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub editor: EditorRef,
    pub global: Layer,
    pub groups: BTreeMap<String, Layer>,
    pub profiles: BTreeMap<String, ProfileConfig>,
}

/// How the config refers to / overrides the target editor.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EditorRef {
    /// Editor name (`nameShort`/`applicationName`) to match during discovery.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Explicit launcher path, bypassing PATH discovery.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binary: Option<PathBuf>,
    /// Override for the editor's `User/` directory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_dir: Option<PathBuf>,
    /// Override for the shared extensions directory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions_dir: Option<PathBuf>,
}

/// A reusable bundle of settings and extensions (`[global]` and `[groups.*]`).
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Layer {
    /// Settings keys applied to profiles using this layer.
    pub settings: BTreeMap<String, Value>,
    /// Extension IDs (`publisher.name`) applied to profiles using this layer.
    pub extensions: Vec<String>,
}

/// Per-profile configuration.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProfileConfig {
    /// Codicon ID used as the profile icon.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Names of groups this profile includes.
    pub groups: Vec<String>,
    /// Profile-specific settings (highest precedence).
    pub settings: BTreeMap<String, Value>,
    /// Profile-specific extensions.
    pub extensions: Vec<String>,
    /// Extension IDs to drop even if a group/global adds them.
    pub exclude_extensions: Vec<String>,
    /// Resource types this profile inherits from Default (`useDefaultFlags`).
    pub use_default: BTreeMap<String, bool>,
}

/// Effective desired state for one profile after layering.
#[derive(Debug, Default, Clone)]
pub struct Resolved {
    pub settings: BTreeMap<String, Value>,
    pub extensions: BTreeSet<String>,
    pub icon: Option<String>,
    pub use_default: BTreeMap<String, bool>,
}

impl Config {
    /// Load a config from a TOML file.
    pub fn load(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let config: Self =
            toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
        Ok(config)
    }

    /// Serialize the config to a TOML string with a schema directive header.
    pub fn to_toml(&self) -> Result<String> {
        let body = toml::to_string_pretty(self).context("serializing config")?;
        Ok(format!("#:schema ./schema/config.schema.json\n\n{body}"))
    }

    /// Effective desired state per profile (keyed by profile name).
    pub fn resolve(&self) -> BTreeMap<String, Resolved> {
        let mut out = BTreeMap::new();
        for (name, profile) in &self.profiles {
            out.insert(name.clone(), self.resolve_profile(profile));
        }
        out
    }

    fn resolve_profile(&self, profile: &ProfileConfig) -> Resolved {
        let mut settings = self.global.settings.clone();
        let mut extensions: BTreeSet<String> =
            self.global.extensions.iter().map(|id| normalize_id(id)).collect();

        for group_name in &profile.groups {
            if let Some(group) = self.groups.get(group_name) {
                for (key, value) in &group.settings {
                    settings.insert(key.clone(), value.clone());
                }
                extensions.extend(group.extensions.iter().map(|id| normalize_id(id)));
            }
        }

        for (key, value) in &profile.settings {
            settings.insert(key.clone(), value.clone());
        }
        extensions.extend(profile.extensions.iter().map(|id| normalize_id(id)));
        for excluded in &profile.exclude_extensions {
            extensions.remove(&normalize_id(excluded));
        }

        Resolved {
            settings,
            extensions,
            icon: profile.icon.clone(),
            use_default: profile.use_default.clone(),
        }
    }
}

/// Normalize an extension identifier for set membership: drop any `@version`
/// pin and lowercase (`publisher.name` is case-insensitive).
pub fn normalize_id(spec: &str) -> String {
    let id = spec.split('@').next().unwrap_or(spec);
    id.trim().to_lowercase()
}
