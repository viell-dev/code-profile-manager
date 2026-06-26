//! Reading VS Code's JSONC files (comments + trailing commas allowed) and
//! writing them back as plain, pretty-printed JSON.
//!
//! Comment/formatting preservation is a deliberate non-goal for now (see
//! `PLAN.md` §5): we parse to a value, operate on it, and re-serialize.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::{Map, Value};

/// Parse a JSONC file into a JSON value. A missing or empty file yields an empty
/// object, which is the natural identity for settings/extension files.
pub fn read_object(path: &Path) -> Result<Map<String, Value>> {
    if !path.is_file() {
        return Ok(Map::new());
    }
    let raw = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let value = parse(&raw).with_context(|| format!("parsing {}", path.display()))?;
    match value {
        Value::Object(map) => Ok(map),
        Value::Null => Ok(Map::new()),
        _ => anyhow::bail!("{} is not a JSON object", path.display()),
    }
}

/// Parse a JSONC string into a JSON value.
pub fn parse(raw: &str) -> Result<Value> {
    let options = jsonc_parser::ParseOptions::default();
    // Empty/whitespace input deserializes as `null`.
    let parsed =
        jsonc_parser::parse_to_serde_value::<Value>(raw, &options).context("invalid JSONC")?;
    Ok(parsed)
}

/// Serialize a JSON value to a pretty string with a trailing newline.
pub fn to_pretty(value: &Value) -> Result<String> {
    let mut text = serde_json::to_string_pretty(value).context("serializing JSON")?;
    text.push('\n');
    Ok(text)
}
