//! The persisted "exam mode is on" record. Its presence on disk is what makes the next launch
//! show "Restore my PC" — even after a crash or reboot.

use serde::{Deserialize, Serialize};

use crate::ports::StoreError;

/// Bump when the format changes; older records must stay readable by newer versions.
pub const RECORD_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExamModeRecord {
    pub version: u32,
    pub started_unix: u64,
    pub items_changed: usize,
}

impl ExamModeRecord {
    pub fn new(started_unix: u64, items_changed: usize) -> Self {
        Self {
            version: RECORD_VERSION,
            started_unix,
            items_changed,
        }
    }

    pub fn to_json(&self) -> Result<String, StoreError> {
        serde_json::to_string_pretty(self).map_err(|error| StoreError::Corrupt(error.to_string()))
    }

    pub fn from_json(json: &str) -> Result<Self, StoreError> {
        let record: Self =
            serde_json::from_str(json).map_err(|error| StoreError::Corrupt(error.to_string()))?;
        if record.version > RECORD_VERSION {
            return Err(StoreError::Corrupt(format!(
                "record version {} is newer than this app understands ({RECORD_VERSION})",
                record.version
            )));
        }
        Ok(record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json() {
        let record = ExamModeRecord::new(1_790_000_000, 12);
        let json = record.to_json().unwrap();
        assert_eq!(ExamModeRecord::from_json(&json).unwrap(), record);
    }

    #[test]
    fn rejects_garbage() {
        assert!(matches!(
            ExamModeRecord::from_json("not json"),
            Err(StoreError::Corrupt(_))
        ));
    }

    #[test]
    fn rejects_newer_versions() {
        let json = r#"{"version":99,"started_unix":1,"items_changed":0}"#;
        assert!(matches!(
            ExamModeRecord::from_json(json),
            Err(StoreError::Corrupt(_))
        ));
    }
}
