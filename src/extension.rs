//! Reading per-profile extension membership and changing it.
//!
//! Membership is read directly from the relevant `extensions.json`. To add an
//! extension we prefer the shared pool: if it is already installed in the
//! editor's extensions directory, we copy its catalog entry straight into the
//! profile's membership list (no marketplace needed). Only when it is absent do
//! we shell out to the editor CLI to fetch it. Removal edits the membership list
//! directly and never deletes shared files on disk.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::Value;

use crate::config::normalize_id;
use crate::editor::Editor;
use crate::editor::profiles::Profile;
use crate::safety;

/// Full pool catalog entries, keyed by normalized extension id.
pub type Catalog = BTreeMap<String, Value>;

/// An installed extension as recorded in a profile's `extensions.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledExt {
    /// The installed version (e.g. `1.31.7`).
    pub version: String,
    /// Whether the editor has the version frozen (`metadata.pinned`).
    pub pinned: bool,
}

/// Per-profile membership: normalized id -> installed state.
pub type Membership = BTreeMap<String, InstalledExt>;

/// How an extension was added to a profile.
pub enum AddMethod {
    /// Copied from the shared pool catalog (already installed on disk).
    Pool,
    /// Restored from a vendored copy in the repo.
    Vendor,
    /// Fetched and installed via the editor CLI.
    Cli,
}

#[derive(Debug, Deserialize)]
struct RawEntry {
    identifier: RawIdentifier,
}

#[derive(Debug, Deserialize)]
struct RawIdentifier {
    id: String,
}

/// Read the installed extensions (normalized id -> state) for a profile.
pub fn read_membership(editor: &Editor, profile: &Profile) -> Result<Membership> {
    read_membership_file(&profile.extensions_path(editor))
}

/// Read installed extensions from an `extensions.json` file.
pub fn read_membership_file(path: &Path) -> Result<Membership> {
    let mut out = Membership::new();
    for entry in read_entries(path)? {
        if let Some(id) = entry_id(&entry) {
            out.entry(id).or_insert_with(|| InstalledExt {
                version: entry_version(&entry),
                pinned: entry_pinned(&entry),
            });
        }
    }
    Ok(out)
}

/// The shared extensions pool catalog (full entries with metadata/location).
pub fn pool_catalog(editor: &Editor) -> Result<Catalog> {
    let path = editor.extensions_dir.join("extensions.json");
    let mut catalog = Catalog::new();
    for entry in read_entries(&path)? {
        if let Some(id) = entry_id(&entry) {
            catalog.entry(id).or_insert(entry);
        }
    }
    Ok(catalog)
}

/// Ensure `id` is a member of `profile` at the desired version. `pin` requests an
/// exact version (held via `metadata.pinned`); `None` floats to whatever a source
/// provides. Tries, in order: the shared pool (if it already has the right
/// version), a vendored copy in the repo, then the editor CLI.
pub fn add_member(
    editor: &Editor,
    profile: &Profile,
    id: &str,
    pin: Option<&str>,
    catalog: &Catalog,
    vendor_dir: &Path,
    backup_dir: &Path,
) -> Result<AddMethod> {
    if add_from_catalog(editor, profile, id, pin, catalog, backup_dir)? {
        return Ok(AddMethod::Pool);
    }
    if add_from_vendor(editor, profile, id, pin, vendor_dir, backup_dir)? {
        return Ok(AddMethod::Vendor);
    }
    // The editor CLI accepts `id@version` to fetch a specific build.
    let spec = pin.map_or_else(|| id.to_owned(), |version| format!("{id}@{version}"));
    run_cli(
        editor,
        profile.cli_profile(),
        &["--install-extension", &spec, "--force"],
    )
    .with_context(|| format!("installing extension {spec}"))?;
    // A pinned install must be frozen so the editor won't auto-update it; the CLI
    // does not always set this, so assert it on the freshly written entry.
    if pin.is_some() {
        set_membership_pinned(editor, profile, id, backup_dir)?;
    }
    Ok(AddMethod::Cli)
}

/// Copy local (VSIX-source) extensions referenced by `ids` from the pool into
/// `vendor_dir` so the config is portable to machines without them installed.
/// Returns the number of extensions vendored.
pub fn vendor_local(
    editor: &Editor,
    catalog: &Catalog,
    ids: &BTreeSet<String>,
    vendor_dir: &Path,
    dry_run: bool,
) -> Result<usize> {
    let mut count = 0_usize;
    for id in ids {
        let Some(entry) = catalog.get(id) else {
            continue;
        };
        if entry_source(entry).as_deref() != Some("vsix") {
            continue;
        }
        let Some(rel) = relative_location(entry) else {
            continue;
        };
        let source = editor.extensions_dir.join(&rel);
        if !source.is_dir() {
            continue;
        }
        count = count.saturating_add(1);
        if dry_run {
            continue;
        }
        let dest = vendor_dir.join(&rel);
        if !dest.is_dir() {
            copy_dir(&source, &dest)?;
        }
        let sidecar = vendor_dir.join(format!("{rel}.entry.json"));
        let text = serde_json::to_string_pretty(entry).context("serializing vendored entry")?;
        safety::atomic_write(&sidecar, &text)?;
    }
    Ok(count)
}

/// Remove `id` from a profile's membership list (never deletes shared files).
/// Returns whether an entry was removed.
pub fn remove_member(
    editor: &Editor,
    profile: &Profile,
    id: &str,
    backup_dir: &Path,
) -> Result<bool> {
    let path = profile.extensions_path(editor);
    let mut entries = read_entries(&path)?;
    let before = entries.len();
    entries.retain(|e| entry_id(e).as_deref() != Some(id));
    if entries.len() == before {
        return Ok(false);
    }
    write_entries(&path, &entries, backup_dir)?;
    Ok(true)
}

/// Add the pool's catalog entry for `id` to a profile's membership list. Returns
/// `false` when the extension is not in the pool (caller falls back to the CLI).
fn add_from_catalog(
    editor: &Editor,
    profile: &Profile,
    id: &str,
    pin: Option<&str>,
    catalog: &Catalog,
    backup_dir: &Path,
) -> Result<bool> {
    let Some(entry) = catalog.get(id) else {
        return Ok(false);
    };
    // The pool holds a single installed version; it can only satisfy a pin that
    // matches it, otherwise fall through to a vendored copy or the CLI.
    if let Some(version) = pin
        && entry_version(entry) != version
    {
        return Ok(false);
    }
    let path = profile.extensions_path(editor);
    let mut entries = read_entries(&path)?;
    if let Some(existing) = entries.iter().find(|e| entry_id(e).as_deref() == Some(id)) {
        let satisfied = pin.is_none() || entry_pinned(existing);
        if satisfied {
            return Ok(true);
        }
    }
    let mut entry = entry.clone();
    if pin.is_some() {
        set_pinned(&mut entry, true);
    }
    // Replace any stale entry for this id (a version-drift or pin reinstall).
    entries.retain(|e| entry_id(e).as_deref() != Some(id));
    entries.push(entry);
    write_entries(&path, &entries, backup_dir)?;
    Ok(true)
}

/// Restore a vendored extension: copy its folder into the pool (if missing),
/// fix its on-disk location, and add it to the profile's membership list.
/// Returns `false` when no vendored copy of `id` exists.
fn add_from_vendor(
    editor: &Editor,
    profile: &Profile,
    id: &str,
    pin: Option<&str>,
    vendor_dir: &Path,
    backup_dir: &Path,
) -> Result<bool> {
    let Some((mut entry, rel)) = find_vendored(vendor_dir, id)? else {
        return Ok(false);
    };
    // A pin is only satisfiable by a vendored copy of that exact version.
    if let Some(version) = pin
        && entry_version(&entry) != version
    {
        return Ok(false);
    }
    let vendored = vendor_dir.join(&rel);
    if !vendored.is_dir() {
        return Ok(false);
    }
    let pool_folder = editor.extensions_dir.join(&rel);
    if !pool_folder.is_dir() {
        copy_dir(&vendored, &pool_folder)?;
    }
    // Point the entry at this machine's pool location.
    if let Value::Object(map) = &mut entry {
        map.insert(
            "location".to_owned(),
            serde_json::json!({
                "$mid": 1,
                "path": pool_folder.to_string_lossy(),
                "scheme": "file",
            }),
        );
    }
    if pin.is_some() {
        set_pinned(&mut entry, true);
    }

    let path = profile.extensions_path(editor);
    let mut entries = read_entries(&path)?;
    if let Some(existing) = entries.iter().find(|e| entry_id(e).as_deref() == Some(id))
        && (pin.is_none() || entry_pinned(existing))
    {
        return Ok(true);
    }
    entries.retain(|e| entry_id(e).as_deref() != Some(id));
    entries.push(entry);
    write_entries(&path, &entries, backup_dir)?;
    Ok(true)
}

/// Find a vendored extension by id, returning its catalog entry and relative
/// location.
fn find_vendored(vendor_dir: &Path, id: &str) -> Result<Option<(Value, String)>> {
    if !vendor_dir.is_dir() {
        return Ok(None);
    }
    for dir_entry in
        fs::read_dir(vendor_dir).with_context(|| format!("reading {}", vendor_dir.display()))?
    {
        let path = dir_entry?.path();
        let is_sidecar = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".entry.json"));
        if !is_sidecar {
            continue;
        }
        let raw =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let entry: Value =
            serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
        if entry_id(&entry).as_deref() == Some(id)
            && let Some(rel) = relative_location(&entry)
        {
            return Ok(Some((entry, rel)));
        }
    }
    Ok(None)
}

/// Recursively copy a directory tree.
fn copy_dir(source: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest).with_context(|| format!("creating {}", dest.display()))?;
    for child in fs::read_dir(source).with_context(|| format!("reading {}", source.display()))? {
        let child = child?;
        let from = child.path();
        let to = dest.join(child.file_name());
        if from.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            fs::copy(&from, &to).with_context(|| format!("copying {}", from.display()))?;
        }
    }
    Ok(())
}

fn entry_source(entry: &Value) -> Option<String> {
    entry
        .get("metadata")?
        .get("source")?
        .as_str()
        .map(str::to_owned)
}

/// The installed version of an entry (empty when absent).
fn entry_version(entry: &Value) -> String {
    entry
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// Whether an entry is frozen (`metadata.pinned == true`).
fn entry_pinned(entry: &Value) -> bool {
    entry
        .get("metadata")
        .and_then(|m| m.get("pinned"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Set `metadata.pinned` on an entry, creating `metadata` if needed.
fn set_pinned(entry: &mut Value, pinned: bool) {
    if let Value::Object(map) = entry {
        let metadata = map
            .entry("metadata".to_owned())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        if let Value::Object(metadata) = metadata {
            metadata.insert("pinned".to_owned(), Value::Bool(pinned));
        }
    }
}

/// Ensure the profile's membership entry for `id` is frozen (`metadata.pinned`),
/// rewriting the file only when it isn't already.
fn set_membership_pinned(
    editor: &Editor,
    profile: &Profile,
    id: &str,
    backup_dir: &Path,
) -> Result<()> {
    let path = profile.extensions_path(editor);
    let mut entries = read_entries(&path)?;
    let mut changed = false;
    for entry in &mut entries {
        if entry_id(entry).as_deref() == Some(id) && !entry_pinned(entry) {
            set_pinned(entry, true);
            changed = true;
        }
    }
    if changed {
        write_entries(&path, &entries, backup_dir)?;
    }
    Ok(())
}

fn relative_location(entry: &Value) -> Option<String> {
    entry.get("relativeLocation")?.as_str().map(str::to_owned)
}

/// Read the raw entry list from an `extensions.json` file (empty if missing).
fn read_entries(path: &Path) -> Result<Vec<Value>> {
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))
}

/// The normalized id of an `extensions.json` entry.
fn entry_id(entry: &Value) -> Option<String> {
    let raw: RawEntry = serde_json::from_value(entry.clone()).ok()?;
    Some(normalize_id(&raw.identifier.id))
}

fn write_entries(path: &Path, entries: &[Value], backup_dir: &Path) -> Result<()> {
    safety::backup_file(path, backup_dir)?;
    let mut text = serde_json::to_string_pretty(entries).context("serializing extensions.json")?;
    text.push('\n');
    safety::atomic_write(path, &text)
}

fn run_cli(editor: &Editor, profile_name: Option<&str>, args: &[&str]) -> Result<()> {
    let mut command = Command::new(&editor.launcher);
    if let Some(name) = profile_name {
        command.arg("--profile").arg(name);
    }
    command.args(args);
    let output = command
        .output()
        .with_context(|| format!("running {}", editor.launcher.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        bail!("editor CLI failed: {}", best_error_line(&stderr, &stdout));
    }
    Ok(())
}

/// Pick the most informative line of CLI output, skipping Electron/Chromium and
/// Node noise and progress chatter, preferring a line that names the failure.
fn best_error_line(stderr: &str, stdout: &str) -> String {
    let is_noise = |line: &&str| {
        line.is_empty()
            || line.starts_with("Warning:")
            || line.contains("DeprecationWarning")
            || line.contains("trace-deprecation")
            || line.starts_with("Installing extensions")
    };
    let lines: Vec<&str> = stderr
        .lines()
        .chain(stdout.lines())
        .map(str::trim)
        .filter(|l| !is_noise(l))
        .collect();
    let chosen = lines
        .iter()
        .rev()
        .find(|line| {
            line.contains("not found")
                || line.starts_with("Failed")
                || line.to_ascii_lowercase().contains("error")
        })
        .or_else(|| lines.last());
    chosen.map_or_else(|| "unknown error".to_owned(), |line| (*line).to_owned())
}

#[cfg(test)]
mod tests {
    use super::best_error_line;

    #[test]
    fn best_error_line_prefers_the_failure_over_noise_and_progress() {
        let stderr = "Warning: 'enable-features' is not in the list of known options\n\
                      (node:36363) [DEP0169] DeprecationWarning: url.parse() ...\n\
                      Extension 'x.y' not found.\n\
                      Make sure you use the full extension ID, including the publisher\n\
                      Failed Installing Extensions: x.y";
        let stdout = "Installing extensions...";
        assert_eq!(
            best_error_line(stderr, stdout),
            "Failed Installing Extensions: x.y"
        );
    }

    #[test]
    fn best_error_line_falls_back_to_last_line() {
        assert_eq!(
            best_error_line("", "something odd happened"),
            "something odd happened"
        );
        assert_eq!(best_error_line("", ""), "unknown error");
    }
}
