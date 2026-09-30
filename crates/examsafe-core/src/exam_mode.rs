//! The persisted exam-mode journal. Its presence on disk is what makes the next launch show
//! "Restore my PC" — even after a crash or reboot — and it lists exactly what to put back.

use serde::{Deserialize, Serialize};

use crate::apps::ClosedApp;
use crate::ports::StoreError;

/// Bump when the format changes; older records must stay readable by newer versions.
/// v1 (prototype) had no app list; it reads as "nothing to reopen".
pub const RECORD_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExamModeRecord {
    pub version: u32,
    pub started_unix: u64,
    #[serde(default)]
    pub closed_apps: Vec<ClosedApp>,
}

impl ExamModeRecord {
    pub fn new(started_unix: u64) -> Self {
        Self {
            version: RECORD_VERSION,
            started_unix,
            closed_apps: Vec::new(),
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
    use crate::apps::LaunchTarget;

    #[test]
    fn round_trips_through_json() {
        let mut record = ExamModeRecord::new(1_790_000_000);
        record.closed_apps.push(ClosedApp {
            entry_id: "slack".into(),
            name: "Slack".into(),
            processes: vec!["slack".into()],
            launch: Some(LaunchTarget::AppId("Pkg!App".into())),
        });
        let json = record.to_json().unwrap();
        assert_eq!(ExamModeRecord::from_json(&json).unwrap(), record);
    }

    #[test]
    fn reads_prototype_v1_records() {
        let json = r#"{"version":1,"started_unix":5,"items_changed":16}"#;
        let record = ExamModeRecord::from_json(json).unwrap();
        assert_eq!(record.started_unix, 5);
        assert!(record.closed_apps.is_empty());
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
        let json = r#"{"version":99,"started_unix":1}"#;
        assert!(matches!(
            ExamModeRecord::from_json(json),
            Err(StoreError::Corrupt(_))
        ));
    }
}
