//! User preferences that survive restarts: currently which catalog apps the user chose to leave
//! open (switched off in Advanced). Both the window and the CLI honour them.

use serde::{Deserialize, Serialize};

use crate::ports::StoreError;

pub const PREFERENCES_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preferences {
    pub version: u32,
    /// Catalog ids of apps ExamSafe should leave open.
    #[serde(default)]
    pub left_open: Vec<String>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: PREFERENCES_VERSION,
            left_open: Vec::new(),
        }
    }
}

impl Preferences {
    pub fn to_json(&self) -> Result<String, StoreError> {
        serde_json::to_string_pretty(self).map_err(|error| StoreError::Corrupt(error.to_string()))
    }

    pub fn from_json(json: &str) -> Result<Self, StoreError> {
        serde_json::from_str(json).map_err(|error| StoreError::Corrupt(error.to_string()))
    }

    /// Sets whether `entry_id` is left open. Keeps the list sorted and free of duplicates so the
    /// file stays stable and diff-friendly.
    pub fn set_left_open(&mut self, entry_id: &str, left_open: bool) {
        self.left_open.retain(|id| id != entry_id);
        if left_open {
            self.left_open.push(entry_id.to_owned());
            self.left_open.sort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggling_adds_and_removes_without_duplicates() {
        let mut prefs = Preferences::default();
        prefs.set_left_open("slack", true);
        prefs.set_left_open("discord", true);
        prefs.set_left_open("slack", true);
        assert_eq!(prefs.left_open, vec!["discord", "slack"]);
        prefs.set_left_open("slack", false);
        assert_eq!(prefs.left_open, vec!["discord"]);
    }

    #[test]
    fn round_trips_and_tolerates_missing_fields() {
        let mut prefs = Preferences::default();
        prefs.set_left_open("slack", true);
        assert_eq!(
            Preferences::from_json(&prefs.to_json().unwrap()).unwrap(),
            prefs
        );
        let minimal = Preferences::from_json(r#"{"version":1}"#).unwrap();
        assert!(minimal.left_open.is_empty());
        assert!(Preferences::from_json("nope").is_err());
    }
}
