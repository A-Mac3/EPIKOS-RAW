//! Imported 3D LUTs (`.cube`). Importing copies the file into the app's LUT folder
//! (validated first), so edits keep working if the original moves; a sidecar names a
//! LUT by its file name there.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError};

use epikos_core::{Error, Result};
use epikos_pipeline::Lut3d;
use serde::Serialize;

use crate::Engine;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LutInfo {
    /// File name in the LUT folder without `.cube`; what a sidecar stores.
    pub name: String,
    /// The file's `TITLE`, if any.
    pub title: Option<String>,
    /// Grid size (e.g. 33 for a 33³ LUT).
    pub size: usize,
}

/// `$EPIKOS_LUTS_DIR`, else `luts/` in the app's data folder (the same one the desktop
/// app uses, so the CLI sees imported LUTs too).
pub fn default_lut_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("EPIKOS_LUTS_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let base = if cfg!(target_os = "macos") {
        home.join("Library/Application Support")
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or(home)
    } else {
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share"))
    };
    base.join("com.epikos.raw").join("luts")
}

impl Engine {
    /// Keep imported LUTs in `dir`.
    pub fn with_lut_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.lut_dir = dir.into();
        self
    }

    pub fn lut_dir(&self) -> &Path {
        &self.lut_dir
    }

    /// Imported LUTs, by name. Files that no longer parse are left out.
    pub fn list_luts(&self) -> Result<Vec<LutInfo>> {
        let Ok(entries) = fs::read_dir(&self.lut_dir) else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for entry in entries {
            let path = entry?.path();
            if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("cube")) {
                let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                if let Some(lut) = self.lut(&name) {
                    out.push(LutInfo { name, title: lut.title.clone(), size: lut.size });
                }
            }
        }
        out.sort_by_key(|l| l.name.to_lowercase());
        Ok(out)
    }

    /// Validate `src` as a 3D `.cube` LUT and copy it into the LUT folder. A different
    /// LUT with the same name gets a number (" 2"); re-importing the same file is a no-op.
    pub fn import_lut(&self, src: &Path) -> Result<LutInfo> {
        let text = fs::read_to_string(src)?;
        let lut = Lut3d::parse_cube(&text)?;
        fs::create_dir_all(&self.lut_dir)?;
        let stem = sanitize(&src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
        let stem = if stem.is_empty() { "LUT".to_string() } else { stem };
        let mut name = stem.clone();
        for n in 2.. {
            let dest = self.lut_dir.join(format!("{name}.cube"));
            match fs::read_to_string(&dest) {
                Ok(existing) if existing == text => break,
                Ok(_) => name = format!("{stem} {n}"),
                Err(_) => {
                    fs::write(&dest, &text)?;
                    break;
                }
            }
        }
        self.luts.lock().unwrap_or_else(PoisonError::into_inner).retain(|(n, _)| *n != name);
        Ok(LutInfo { name, title: lut.title, size: lut.size })
    }

    /// Delete an imported LUT (the app's copy; the original file is untouched).
    pub fn remove_lut(&self, name: &str) -> Result<()> {
        let path = self.lut_path(name)?;
        fs::remove_file(path)?;
        self.luts.lock().unwrap_or_else(PoisonError::into_inner).retain(|(n, _)| n != name);
        Ok(())
    }

    /// The LUT named `name`, parsed once and cached. `None` if it isn't installed.
    pub(crate) fn lut(&self, name: &str) -> Option<Arc<Lut3d>> {
        if name.is_empty() {
            return None;
        }
        let mut cache = self.luts.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, l)) = cache.iter().find(|(n, _)| n == name) {
            return Some(l.clone());
        }
        let text = fs::read_to_string(self.lut_path(name).ok()?).ok()?;
        let lut = Arc::new(Lut3d::parse_cube(&text).inspect_err(|e| eprintln!("LUT {name}: {e}")).ok()?);
        cache.push((name.to_string(), lut.clone()));
        Some(lut)
    }

    fn lut_path(&self, name: &str) -> Result<PathBuf> {
        if name.is_empty() || sanitize(name) != name {
            return Err(Error::InvalidImage { reason: format!("not a LUT name: {name:?}") });
        }
        Ok(self.lut_dir.join(format!("{name}.cube")))
    }
}

/// A file-name-safe LUT name: letters, digits, spaces, `-`, `_`, `.`, `&`, `(`, `)`.
fn sanitize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric() || " -_.&()".contains(*c))
        .collect::<String>()
        .trim_matches(|c: char| c == '.' || c.is_whitespace())
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(n: usize) -> String {
        let mut s = format!("TITLE \"Neutral\"\nLUT_3D_SIZE {n}\n");
        for b in 0..n {
            for g in 0..n {
                for r in 0..n {
                    let f = |v: usize| v as f32 / (n - 1) as f32;
                    s += &format!("{} {} {}\n", f(r), f(g), f(b));
                }
            }
        }
        s
    }

    #[test]
    fn import_list_rename_and_remove() {
        let dir = std::env::temp_dir().join(format!("epikos-luts-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let src = dir.join("My Look.cube");
        fs::write(&src, identity(5)).unwrap();
        let engine = Engine::default().with_lut_dir(dir.join("luts"));

        let a = engine.import_lut(&src).unwrap();
        assert_eq!((a.name.as_str(), a.title.as_deref(), a.size), ("My Look", Some("Neutral"), 5));
        // Same file again: same entry. A different LUT of the same name: numbered.
        assert_eq!(engine.import_lut(&src).unwrap().name, "My Look");
        fs::write(&src, identity(3)).unwrap();
        assert_eq!(engine.import_lut(&src).unwrap().name, "My Look 2");
        let names: Vec<_> = engine.list_luts().unwrap().into_iter().map(|l| l.name).collect();
        assert_eq!(names, ["My Look", "My Look 2"]);
        assert!(engine.lut("My Look").is_some());

        // Not a LUT: refused, nothing copied.
        let bad = dir.join("bad.cube");
        fs::write(&bad, "LUT_1D_SIZE 2\n0 0 0\n1 1 1\n").unwrap();
        assert!(engine.import_lut(&bad).is_err());
        assert!(engine.remove_lut("../escape").is_err());

        engine.remove_lut("My Look").unwrap();
        assert!(engine.lut("My Look").is_none());
        assert_eq!(engine.list_luts().unwrap().len(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }
}
