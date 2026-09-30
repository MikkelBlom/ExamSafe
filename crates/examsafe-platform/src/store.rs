//! File-backed [`ExamModeRepository`].

use std::path::{Path, PathBuf};

use examsafe_core::exam_mode::ExamModeRecord;
use examsafe_core::ports::{ExamModeRepository, StoreError};

use crate::paths::app_data_dir;

#[derive(Debug, Clone)]
pub struct FileExamModeStore {
    path: PathBuf,
}

impl FileExamModeStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// `%LOCALAPPDATA%\ExamSafe\exam-mode.json`, or `$EXAMSAFE_STATE_DIR\exam-mode.json` when
    /// that variable is set (used by tests so they never touch the real record).
    pub fn in_app_data() -> Self {
        let dir = std::env::var_os("EXAMSAFE_STATE_DIR").map_or_else(app_data_dir, PathBuf::from);
        Self::new(dir.join("exam-mode.json"))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn io_error(context: &str, error: &std::io::Error) -> StoreError {
    StoreError::Io(format!("{context}: {error}"))
}

impl ExamModeRepository for FileExamModeStore {
    fn load(&self) -> Result<Option<ExamModeRecord>, StoreError> {
        match std::fs::read_to_string(&self.path) {
            Ok(json) => ExamModeRecord::from_json(&json).map(Some),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(io_error("read", &error)),
        }
    }

    fn save(&self, record: &ExamModeRecord) -> Result<(), StoreError> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|error| io_error("create folder", &error))?;
        }
        // Write-then-rename so a crash never leaves a half-written record.
        let temp = self.path.with_extension("json.tmp");
        std::fs::write(&temp, record.to_json()?).map_err(|error| io_error("write", &error))?;
        std::fs::rename(&temp, &self.path).map_err(|error| io_error("replace", &error))
    }

    fn clear(&self) -> Result<(), StoreError> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io_error("delete", &error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_store() -> FileExamModeStore {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("examsafe-test-{}-{nanos}", std::process::id()));
        FileExamModeStore::new(dir.join("exam-mode.json"))
    }

    #[test]
    fn missing_file_means_not_in_exam_mode() {
        assert_eq!(temp_store().load().unwrap(), None);
    }

    #[test]
    fn save_load_clear_round_trip() {
        let store = temp_store();
        let record = ExamModeRecord::new(1_790_000_000);
        store.save(&record).unwrap();
        assert_eq!(store.load().unwrap(), Some(record));
        store.clear().unwrap();
        assert_eq!(store.load().unwrap(), None);
        store.clear().unwrap();
        if let Some(dir) = store.path().parent() {
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn corrupt_file_is_reported_not_ignored() {
        let store = temp_store();
        let dir = store.path().parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(store.path(), "{ broken").unwrap();
        assert!(matches!(store.load(), Err(StoreError::Corrupt(_))));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
