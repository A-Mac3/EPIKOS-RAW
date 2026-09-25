use std::fs;
use std::path::{Path, PathBuf};

use epikos_core::{Error, Result};

use crate::document::DevelopDocument;

pub fn sidecar_json_path(raw_path: &Path) -> PathBuf {
    let mut p = raw_path.as_os_str().to_os_string();
    p.push(".epikos.json");
    PathBuf::from(p)
}

pub fn save_json(path: &Path, doc: &DevelopDocument) -> Result<()> {
    let text = serde_json::to_string_pretty(doc).map_err(|e| Error::Sidecar(e.to_string()))?;
    fs::write(path, text)?;
    Ok(())
}

pub fn load_json(path: &Path) -> Result<DevelopDocument> {
    let text = fs::read_to_string(path)?;
    serde_json::from_str(&text).map_err(|e| Error::Sidecar(e.to_string()))
}

#[cfg(test)]
mod tests {
    use crate::document::{DevelopDocument, SourceRef};

    #[test]
    fn json_roundtrip_is_bit_identical_struct() {
        let doc = DevelopDocument::new(SourceRef {
            path: "a.ARW".into(),
            sha256: "abc".into(),
            format: "Sony ARW".into(),
            make: "Sony".into(),
            model: "ILCE-7M4".into(),
        });
        let encoded = serde_json::to_string(&doc).unwrap();
        let decoded: DevelopDocument = serde_json::from_str(&encoded).unwrap();
        assert_eq!(doc, decoded);
    }

    #[test]
    fn older_sidecar_without_new_fields_still_loads() {
        let v1 = r#"{"version":1,"source":{"path":"a.ARW","sha256":"","format":"Sony ARW",
            "make":"Sony","model":"ILCE-7M4"},"adjustments":{"highlightRecovery":false}}"#;
        let doc: DevelopDocument = serde_json::from_str(v1).unwrap();
        assert!(!doc.adjustments.highlight_recovery);
        assert_eq!(doc.adjustments.exposure, 0.0);
    }
}
