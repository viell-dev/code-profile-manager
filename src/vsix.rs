//! Reading `.vsix` packages (zip archives) and discovering vendored ones.
//!
//! A `.vsix` is a zip whose `extension/package.json` carries the extension's
//! identity (`publisher`.`name`), `version`, and `engines.vscode`; an optional
//! `extension.vsixmanifest` declares a `TargetPlatform` for native-binary builds.
//! Vendored `.vsix` files are the portable, preferred source for non-marketplace
//! extensions (see PLAN.md §4.1).

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

/// Platform-agnostic builds use this `targetPlatform`.
pub const UNIVERSAL: &str = "universal";

/// Identity metadata read from a `.vsix` package.
///
/// We deliberately do **not** read or evaluate `engines.vscode`: the editor's own
/// installer validates API compatibility and install failures are reported and
/// skipped (see PLAN.md §4.1), so there is nothing for us to gate on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VsixInfo {
    /// Normalized `publisher.name`.
    pub id: String,
    pub version: String,
    /// `targetPlatform` (e.g. `linux-x64`), or [`UNIVERSAL`] when unspecified.
    pub target_platform: String,
}

/// A discovered vendored `.vsix` and its location.
#[derive(Debug, Clone)]
pub struct Vendored {
    pub info: VsixInfo,
    pub path: PathBuf,
}

#[derive(Deserialize)]
struct PackageJson {
    publisher: String,
    name: String,
    version: String,
}

/// Read identity + compatibility metadata from a `.vsix` file.
pub fn read_info(path: &Path) -> Result<VsixInfo> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut archive =
        zip::ZipArchive::new(file).with_context(|| format!("reading vsix {}", path.display()))?;

    let pkg: PackageJson = {
        let mut entry = archive
            .by_name("extension/package.json")
            .with_context(|| format!("{} is missing extension/package.json", path.display()))?;
        let mut buf = String::new();
        entry.read_to_string(&mut buf)?;
        serde_json::from_str(&buf)
            .with_context(|| format!("parsing package.json in {}", path.display()))?
    };
    let target_platform =
        read_target_platform(&mut archive).unwrap_or_else(|| UNIVERSAL.to_owned());

    Ok(VsixInfo {
        id: format!("{}.{}", pkg.publisher, pkg.name).to_lowercase(),
        version: pkg.version,
        target_platform,
    })
}

/// Pull a `TargetPlatform="…"` attribute out of `extension.vsixmanifest`, if any.
fn read_target_platform(archive: &mut zip::ZipArchive<File>) -> Option<String> {
    let mut entry = archive.by_name("extension.vsixmanifest").ok()?;
    let mut buf = String::new();
    entry.read_to_string(&mut buf).ok()?;
    let after = buf.split("TargetPlatform=\"").nth(1)?;
    let value = after.split('"').next()?;
    (!value.is_empty()).then(|| value.to_owned())
}

/// Discover vendored `.vsix` files keyed by normalized id. Multiple versions and
/// target platforms of the same id can coexist, so each id maps to all of its
/// candidates. Unreadable files are skipped.
pub fn discover(vsix_dir: &Path) -> Result<BTreeMap<String, Vec<Vendored>>> {
    let mut out: BTreeMap<String, Vec<Vendored>> = BTreeMap::new();
    if !vsix_dir.is_dir() {
        return Ok(out);
    }
    for entry in
        fs::read_dir(vsix_dir).with_context(|| format!("reading {}", vsix_dir.display()))?
    {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("vsix") {
            continue;
        }
        if let Ok(info) = read_info(&path) {
            out.entry(info.id.clone())
                .or_default()
                .push(Vendored { info, path });
        }
    }
    Ok(out)
}

/// Choose the best candidate for `id`: an exact `pin` match when pinned, else the
/// highest version; ties prefer a build matching `platform` over [`UNIVERSAL`].
pub fn select<'c>(
    candidates: &'c [Vendored],
    pin: Option<&str>,
    platform: &str,
) -> Option<&'c Vendored> {
    let suitable = |v: &Vendored| {
        let platform_ok = v.info.target_platform == platform || v.info.target_platform == UNIVERSAL;
        let version_ok = pin.is_none_or(|want| v.info.version == want);
        platform_ok && version_ok
    };
    candidates.iter().filter(|v| suitable(v)).max_by(|a, b| {
        version_key(&a.info.version)
            .cmp(&version_key(&b.info.version))
            .then_with(|| {
                platform_rank(&a.info.target_platform, platform)
                    .cmp(&platform_rank(&b.info.target_platform, platform))
            })
    })
}

/// The `targetPlatform` for the machine the tool runs on (e.g. `linux-x64`).
pub fn current_platform() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "x86" => "ia32",
        other => other,
    };
    format!("{os}-{arch}")
}

/// A platform-specific build outranks a universal one for the current platform.
fn platform_rank(target: &str, platform: &str) -> u8 {
    if target == platform {
        2
    } else {
        u8::from(target == UNIVERSAL)
    }
}

/// Split a version into numeric components for ordering (`1.31.10` > `1.31.2`).
/// Non-numeric parts sort as 0, which is sufficient for picking a latest build.
fn version_key(version: &str) -> Vec<u64> {
    version
        .split(['.', '-', '+'])
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect()
}

#[cfg(test)]
mod tests {
    #![expect(clippy::unwrap_used, reason = "unit tests")]

    use std::io::Write;

    use super::*;

    /// Write a minimal `.vsix` (zip) with a `package.json` and optional manifest.
    fn write_vsix(
        dir: &Path,
        file: &str,
        publisher: &str,
        name: &str,
        version: &str,
        platform: Option<&str>,
    ) -> PathBuf {
        let path = dir.join(file);
        let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
        zip.start_file("extension/package.json", opts).unwrap();
        let pkg = format!(r#"{{"publisher":"{publisher}","name":"{name}","version":"{version}"}}"#);
        zip.write_all(pkg.as_bytes()).unwrap();
        if let Some(platform) = platform {
            zip.start_file("extension.vsixmanifest", opts).unwrap();
            zip.write_all(format!(r#"<Identity TargetPlatform="{platform}" />"#).as_bytes())
                .unwrap();
        }
        zip.finish().unwrap();
        path
    }

    #[test]
    fn read_info_extracts_identity_version_and_platform() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_vsix(
            dir.path(),
            "git-graph-2-1.31.7.vsix",
            "Hansu",
            "git-graph-2",
            "1.31.7",
            Some("linux-x64"),
        );
        let info = read_info(&path).unwrap();
        assert_eq!(info.id, "hansu.git-graph-2");
        assert_eq!(info.version, "1.31.7");
        assert_eq!(info.target_platform, "linux-x64");
    }

    #[test]
    fn discover_groups_versions_by_id_and_defaults_universal() {
        let dir = tempfile::tempdir().unwrap();
        write_vsix(dir.path(), "a-1.0.0.vsix", "pub", "a", "1.0.0", None);
        write_vsix(dir.path(), "a-2.0.0.vsix", "pub", "a", "2.0.0", None);
        write_vsix(dir.path(), "ignore.txt", "pub", "x", "1.0.0", None); // not a .vsix name
        let found = discover(dir.path()).unwrap();
        let versions: Vec<&str> = found
            .get("pub.a")
            .into_iter()
            .flatten()
            .map(|v| v.info.version.as_str())
            .collect();
        assert_eq!(versions.len(), 2, "both versions of pub.a kept");
        assert!(
            found
                .values()
                .flatten()
                .all(|v| v.info.target_platform == UNIVERSAL)
        );
    }

    fn vendored(version: &str, platform: &str) -> Vendored {
        Vendored {
            info: VsixInfo {
                id: "pub.name".to_owned(),
                version: version.to_owned(),
                target_platform: platform.to_owned(),
            },
            path: PathBuf::from(format!("pub.name-{version}-{platform}.vsix")),
        }
    }

    #[test]
    fn select_prefers_exact_pin() {
        let cands = vec![vendored("1.31.2", UNIVERSAL), vendored("1.31.7", UNIVERSAL)];
        let chosen = select(&cands, Some("1.31.2"), "linux-x64");
        assert_eq!(chosen.map(|c| c.info.version.as_str()), Some("1.31.2"));
    }

    #[test]
    fn select_floats_to_highest_version_numerically() {
        let cands = vec![
            vendored("1.31.2", UNIVERSAL),
            vendored("1.31.10", UNIVERSAL),
        ];
        let chosen = select(&cands, None, "linux-x64");
        assert_eq!(chosen.map(|c| c.info.version.as_str()), Some("1.31.10"));
    }

    #[test]
    fn select_prefers_matching_platform_over_universal() {
        let cands = vec![vendored("1.0.0", UNIVERSAL), vendored("1.0.0", "linux-x64")];
        let chosen = select(&cands, None, "linux-x64");
        assert_eq!(
            chosen.map(|c| c.info.target_platform.as_str()),
            Some("linux-x64")
        );
    }

    #[test]
    fn select_skips_incompatible_platform() {
        let cands = vec![vendored("1.0.0", "win32-x64")];
        assert!(select(&cands, None, "linux-x64").is_none());
    }
}
