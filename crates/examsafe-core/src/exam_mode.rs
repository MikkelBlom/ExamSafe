//! The persisted exam-mode journal. Its presence on disk is what makes the next launch show
//! "Restore my PC" — even after a crash or reboot — and it lists exactly what to put back.

use serde::{Deserialize, Serialize};

use crate::apps::ClosedApp;
use crate::ports::StoreError;
use crate::system::{ServiceSnapshot, TaskSnapshot};

/// Bump when the format changes; older records must stay readable by newer versions.
/// v1 (prototype) had no app list; it reads as "nothing to reopen".
/// v3 added each app's arguments, folder and window state, and services and scheduled tasks.
pub const RECORD_VERSION: u32 = 3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExamModeRecord {
    pub version: u32,
    pub started_unix: u64,
    #[serde(default)]
    pub closed_apps: Vec<ClosedApp>,
    /// Services as they were before exam mode changed them.
    #[serde(default)]
    pub services: Vec<ServiceSnapshot>,
    /// Scheduled tasks as they were before exam mode changed them.
    #[serde(default)]
    pub tasks: Vec<TaskSnapshot>,
}

impl ExamModeRecord {
    pub fn new(started_unix: u64) -> Self {
        Self {
            version: RECORD_VERSION,
            started_unix,
            closed_apps: Vec::new(),
            services: Vec::new(),
            tasks: Vec::new(),
        }
    }

    /// Whether this record holds anything that restore must put back.
    pub fn is_empty(&self) -> bool {
        self.closed_apps.is_empty() && self.services.is_empty() && self.tasks.is_empty()
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
    use crate::apps::{LaunchSpec, LaunchTarget};

    #[test]
    fn round_trips_through_json() {
        let mut record = ExamModeRecord::new(1_790_000_000);
        let spec = LaunchSpec {
            target: LaunchTarget::AppId("Pkg!App".into()),
            arguments: Some("--background \"a b\"".into()),
            working_dir: Some("C:\\Work".into()),
            background: true,
        };
        record.closed_apps.push(ClosedApp::new(
            "slack",
            "Slack",
            vec!["slack".into()],
            Some(&spec),
        ));
        record.services.push(ServiceSnapshot {
            name: "TeamViewer".into(),
            display_name: "TeamViewer".into(),
            start_type: crate::system::StartType::AutomaticDelayed,
            was_running: true,
            target: crate::system::StartType::Disabled,
        });
        record.tasks.push(TaskSnapshot {
            path: "\\PowerToys\\Autorun".into(),
            enabled: true,
        });
        let json = record.to_json().unwrap();
        let read = ExamModeRecord::from_json(&json).unwrap();
        assert_eq!(read, record);
        assert_eq!(read.closed_apps[0].launch_spec(), Some(spec));
        assert!(json.contains("\"automatic_delayed\""));
    }

    #[test]
    fn reads_v2_records_as_plain_windowed_launches() {
        let json = r#"{"version":2,"started_unix":5,"closed_apps":[
            {"entry_id":"slack","name":"Slack","processes":["slack"],
             "launch":{"kind":"path","value":"C:\\slack.exe"}}]}"#;
        let record = ExamModeRecord::from_json(json).unwrap();
        assert_eq!(
            record.closed_apps[0].launch_spec(),
            Some(LaunchSpec::new(LaunchTarget::Path("C:\\slack.exe".into())))
        );
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
