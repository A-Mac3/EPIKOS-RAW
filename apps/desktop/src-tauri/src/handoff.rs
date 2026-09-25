//! Step 8 external handoff: find the photo editors installed on this Mac and open an
//! export in one of them.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HandoffApp {
    /// Display name, e.g. "Adobe Photoshop 2026".
    pub name: String,
    /// The `.app` bundle.
    pub path: String,
}

/// Editors we hand off to, by bundle-name fragment, in menu order.
const KNOWN: &[&str] = &[
    "Photoshop",
    "Lightroom Classic",
    "Lightroom",
    "Capture One",
    "PhotoLab",
    "Affinity Photo",
    "Pixelmator Pro",
];

/// Installed editors, from /Applications and ~/Applications (including Adobe's
/// "Adobe <App> <Year>" folders one level down).
pub fn installed() -> Vec<HandoffApp> {
    if !cfg!(target_os = "macos") {
        return Vec::new();
    }
    let mut roots = vec![PathBuf::from("/Applications")];
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(Path::new(&home).join("Applications"));
    }
    let mut bundles = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(&root) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if is_app(&path) {
                bundles.push(path);
            } else if path.is_dir() && file_name(&path).starts_with("Adobe") {
                if let Ok(inner) = std::fs::read_dir(&path) {
                    bundles.extend(inner.flatten().map(|e| e.path()).filter(|p| is_app(p)));
                }
            }
        }
    }
    let rank = |name: &str| {
        let squashed = name.replace(' ', "").to_lowercase();
        KNOWN
            .iter()
            .position(|k| squashed.contains(&k.replace(' ', "").to_lowercase()))
    };
    let mut apps: Vec<(usize, HandoffApp)> = bundles
        .into_iter()
        .filter_map(|p| {
            let name = file_name(&p).trim_end_matches(".app").to_string();
            // Skip helpers and uninstallers that live next to the real app.
            if name.to_lowercase().contains("uninstall") {
                return None;
            }
            Some((rank(&name)?, HandoffApp { name: display_name(&name), path: p.to_string_lossy().into_owned() }))
        })
        .collect();
    apps.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)));
    apps.dedup_by(|a, b| a.1.path == b.1.path);
    apps.into_iter().map(|(_, a)| a).collect()
}

/// Open `file` (an exported TIFF) in `app`, which must be one of [`installed`].
pub fn open_in(app: &str, file: &Path) -> Result<(), String> {
    if !installed().iter().any(|a| a.path == app) {
        return Err(format!("{app} is not a known photo editor on this computer"));
    }
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if !matches!(ext.as_str(), "tif" | "tiff") || !file.is_file() {
        return Err(format!("{} is not an exported TIFF", file.display()));
    }
    let status = Command::new("open")
        .arg("-a")
        .arg(app)
        .arg(file)
        .status()
        .map_err(|e| format!("could not launch {app}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{app} could not open {}", file.display()))
    }
}

/// DxO ships its bundle as "DXOPhotoLab9"; show "DxO PhotoLab 9".
fn display_name(bundle: &str) -> String {
    if let Some(version) = bundle.strip_prefix("DXOPhotoLab") {
        return format!("DxO PhotoLab {version}").trim_end().to_string();
    }
    bundle.to_string()
}

fn is_app(p: &Path) -> bool {
    p.extension().is_some_and(|e| e == "app") && p.is_dir()
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_in_refuses_unknown_apps_and_non_tiffs() {
        let err = open_in("/Applications/Calculator.app", Path::new("/tmp/x.tif")).unwrap_err();
        assert!(err.contains("not a known photo editor"), "{err}");
        let apps = installed();
        eprintln!("installed editors: {apps:?}");
        if let Some(app) = apps.first() {
            let err = open_in(&app.path, Path::new("/etc/hosts")).unwrap_err();
            assert!(err.contains("not an exported TIFF"), "{err}");
        }
    }
}
