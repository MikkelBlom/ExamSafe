//! Apps: the catalog of apps an exam does not allow, matching it against running processes, and
//! the rules for closing and reopening them. Pure logic over the [`ProcessControl`] port.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::ports::{ProcessControl, ProcessError};
use crate::system::ServiceTarget;

/// How long an app gets to exit after being force-closed before we call it a failure.
pub const TERMINATE_WAIT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Catalog {
    pub version: u32,
    pub entries: Vec<CatalogEntry>,
    /// Words that mark a VPN or virtual network adapter (matched in its name or description).
    #[serde(default)]
    pub virtual_adapters: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CatalogEntry {
    pub id: String,
    pub name: String,
    pub category: String,
    /// Executable names without `.exe`; case-insensitive; a trailing `*` matches a prefix.
    #[serde(default)]
    pub processes: Vec<String>,
    /// Windows service names (not display names); same pattern rules.
    #[serde(default)]
    pub services: Vec<String>,
    /// Full scheduled-task paths, e.g. `\PowerToys\Autorun for *`; same pattern rules.
    #[serde(default)]
    pub tasks: Vec<String>,
    /// What this entry's services are set to for the exam (default: disabled).
    #[serde(default)]
    pub service_start: ServiceTarget,
    /// Reopen this app when restoring.
    #[serde(default = "default_true")]
    pub relaunch: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CatalogError {
    #[error("the app catalog is not valid JSON: {0}")]
    Json(String),
    #[error("app catalog entry '{id}' is invalid: {reason}")]
    Entry { id: String, reason: String },
}

impl Catalog {
    pub fn from_json(json: &str) -> Result<Self, CatalogError> {
        let catalog: Self =
            serde_json::from_str(json).map_err(|error| CatalogError::Json(error.to_string()))?;
        let mut seen = HashSet::new();
        for entry in &catalog.entries {
            let invalid = |reason: &str| {
                Err(CatalogError::Entry {
                    id: entry.id.clone(),
                    reason: reason.to_owned(),
                })
            };
            if entry.id.trim().is_empty() || !seen.insert(entry.id.as_str()) {
                return invalid("id is empty or duplicated");
            }
            if entry.name.trim().is_empty() {
                return invalid("name is empty");
            }
            let footprints = || {
                entry
                    .processes
                    .iter()
                    .chain(&entry.services)
                    .chain(&entry.tasks)
            };
            if footprints().next().is_none() || footprints().any(|p| p.trim().is_empty()) {
                return invalid("needs at least one non-empty process, service or task");
            }
        }
        Ok(catalog)
    }

    pub fn entry(&self, id: &str) -> Option<&CatalogEntry> {
        self.entries.iter().find(|entry| entry.id == id)
    }
}

/// One running process, as reported by the OS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningProcess {
    pub pid: u32,
    pub parent_pid: u32,
    pub exe_name: String,
}

/// Extra details about a process, fetched only for the few processes that need them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProcessDetails {
    pub path: Option<String>,
    /// Store app id (AUMID) for packaged apps such as Claude; these can't be started by path.
    pub app_id: Option<String>,
    /// The full command line, program included, exactly as the process was started.
    pub command_line: Option<String>,
    /// The process's current directory.
    pub working_dir: Option<String>,
}

/// What to start on restore.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum LaunchTarget {
    Path(String),
    AppId(String),
}

/// How to start an app again so it comes back the way it was: same program, same arguments,
/// same folder, and in the background if it had no window open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSpec {
    pub target: LaunchTarget,
    /// Arguments after the program, as one string (Windows passes them that way).
    pub arguments: Option<String>,
    pub working_dir: Option<String>,
    /// It had no visible window (e.g. it lived in the tray): start it without showing one.
    pub background: bool,
}

impl LaunchSpec {
    pub fn new(target: LaunchTarget) -> Self {
        Self {
            target,
            arguments: None,
            working_dir: None,
            background: false,
        }
    }
}

/// The arguments part of a Windows command line: everything after the program, which is either
/// quoted or ends at the first space. `None` when there are no arguments.
pub fn command_line_arguments(command_line: &str) -> Option<String> {
    let line = command_line.trim_start();
    let rest = match line.strip_prefix('"') {
        Some(quoted) => quoted.find('"').map_or("", |end| &quoted[end + 1..]),
        None => line
            .find(char::is_whitespace)
            .map_or("", |end| &line[end..]),
    };
    let arguments = rest.trim();
    (!arguments.is_empty()).then(|| arguments.to_owned())
}

/// A catalog app that is running right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppFinding {
    pub entry_id: String,
    pub name: String,
    pub category: String,
    pub pids: Vec<u32>,
    /// Processes of this app whose parent is not part of the app (the "main" processes).
    pub root_pids: Vec<u32>,
    pub launch: Option<LaunchSpec>,
}

pub fn process_matches(pattern: &str, exe_name: &str) -> bool {
    name_matches(strip_exe(pattern), strip_exe(exe_name))
}

/// Case-insensitive match; a trailing `*` in the pattern matches any suffix.
pub fn name_matches(pattern: &str, name: &str) -> bool {
    let name = name.to_lowercase();
    let pattern = pattern.to_lowercase();
    match pattern.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => name == pattern,
    }
}

fn strip_exe(name: &str) -> &str {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".exe") {
        &name[..name.len() - 4]
    } else {
        name
    }
}

/// Our own process and the chain of processes that started us. These are never closed.
pub fn protected_pids(self_pid: u32, processes: &[RunningProcess]) -> Vec<u32> {
    let parents: HashMap<u32, u32> = processes.iter().map(|p| (p.pid, p.parent_pid)).collect();
    let mut protected = vec![self_pid];
    let mut current = self_pid;
    while let Some(&parent) = parents.get(&current) {
        if parent <= 4 || protected.contains(&parent) || protected.len() > 32 {
            break;
        }
        protected.push(parent);
        current = parent;
    }
    protected
}

/// Matches running processes against the catalog. Each process belongs to at most one app (the
/// first entry that matches); protected processes are never matched. `launch` is left empty.
pub fn match_apps(
    catalog: &Catalog,
    processes: &[RunningProcess],
    protected: &[u32],
) -> Vec<AppFinding> {
    let mut claimed = HashSet::new();
    let mut findings = Vec::new();
    for entry in &catalog.entries {
        let matched: Vec<&RunningProcess> = processes
            .iter()
            .filter(|p| !protected.contains(&p.pid) && !claimed.contains(&p.pid))
            .filter(|p| {
                entry
                    .processes
                    .iter()
                    .any(|pattern| process_matches(pattern, &p.exe_name))
            })
            .collect();
        if matched.is_empty() {
            continue;
        }
        let pids: Vec<u32> = matched.iter().map(|p| p.pid).collect();
        claimed.extend(pids.iter().copied());
        let root_pids = matched
            .iter()
            .filter(|p| !pids.contains(&p.parent_pid))
            .map(|p| p.pid)
            .collect();
        findings.push(AppFinding {
            entry_id: entry.id.clone(),
            name: entry.name.clone(),
            category: entry.category.clone(),
            pids,
            root_pids,
            launch: None,
        });
    }
    findings
}

/// Picks how to reopen the app from its main (root) processes: a Store app id wins (can't be
/// started by path), then a path. Arguments and folder come from that same process, so the app
/// is started exactly as it was. `had_window`: whether any of its processes showed a window.
pub fn pick_launch(details: &[ProcessDetails], had_window: bool) -> Option<LaunchSpec> {
    let (source, target) = details
        .iter()
        .find_map(|d| d.app_id.clone().map(|id| (d, LaunchTarget::AppId(id))))
        .or_else(|| {
            details
                .iter()
                .find_map(|d| d.path.clone().map(|path| (d, LaunchTarget::Path(path))))
        })?;
    Some(LaunchSpec {
        target,
        arguments: source
            .command_line
            .as_deref()
            .and_then(command_line_arguments),
        working_dir: source.working_dir.clone(),
        background: !had_window,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CloseOutcome {
    pub closed: Vec<String>,
    /// "Name: reason" for apps that are still running.
    pub failed: Vec<String>,
}

/// Asks every app to close, waits `grace`, force-closes what is left, and reports per app.
pub fn close_apps(
    control: &dyn ProcessControl,
    apps: &[AppFinding],
    grace: Duration,
) -> CloseOutcome {
    let all: Vec<u32> = apps
        .iter()
        .flat_map(|app| app.pids.iter().copied())
        .collect();
    if all.is_empty() {
        return CloseOutcome::default();
    }
    control.request_close(&all);
    let after_grace = control.wait_for_exit(&all, grace);

    let mut errors: HashMap<u32, ProcessError> = HashMap::new();
    for &pid in &after_grace {
        if let Err(error) = control.terminate(pid) {
            errors.insert(pid, error);
        }
    }
    let still_running: HashSet<u32> = control
        .wait_for_exit(&after_grace, TERMINATE_WAIT)
        .into_iter()
        .collect();

    let mut outcome = CloseOutcome::default();
    for app in apps {
        match app.pids.iter().find(|pid| still_running.contains(pid)) {
            None => outcome.closed.push(app.name.clone()),
            Some(pid) => {
                let reason = errors
                    .get(pid)
                    .map_or_else(|| "did not close".to_owned(), ToString::to_string);
                outcome.failed.push(format!("{}: {reason}", app.name));
            }
        }
    }
    outcome
}

/// An app that exam mode closed, as recorded in the journal. The fields after `launch` were
/// added in record v3; older records read them as "no arguments, had a window" (the old
/// behaviour).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClosedApp {
    pub entry_id: String,
    pub name: String,
    pub processes: Vec<String>,
    pub launch: Option<LaunchTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    #[serde(default = "default_true")]
    pub had_window: bool,
}

impl ClosedApp {
    pub fn new(
        entry_id: &str,
        name: &str,
        processes: Vec<String>,
        launch: Option<&LaunchSpec>,
    ) -> Self {
        Self {
            entry_id: entry_id.to_owned(),
            name: name.to_owned(),
            processes,
            launch: launch.map(|spec| spec.target.clone()),
            arguments: launch.and_then(|spec| spec.arguments.clone()),
            working_dir: launch.and_then(|spec| spec.working_dir.clone()),
            had_window: launch.is_none_or(|spec| !spec.background),
        }
    }

    pub fn launch_spec(&self) -> Option<LaunchSpec> {
        self.launch.clone().map(|target| LaunchSpec {
            target,
            arguments: self.arguments.clone(),
            working_dir: self.working_dir.clone(),
            background: !self.had_window,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RestoreOutcome {
    pub reopened: Vec<String>,
    /// Already running, or not meant to be reopened automatically.
    pub skipped: Vec<String>,
    /// "Name: reason".
    pub failed: Vec<String>,
}

pub fn restore_apps(
    control: &dyn ProcessControl,
    closed: &[ClosedApp],
    running: &[RunningProcess],
) -> RestoreOutcome {
    let mut outcome = RestoreOutcome::default();
    for app in closed {
        let already_running = running.iter().any(|p| {
            app.processes
                .iter()
                .any(|pattern| process_matches(pattern, &p.exe_name))
        });
        match (app.launch_spec(), already_running) {
            (_, true) | (None, false) => outcome.skipped.push(app.name.clone()),
            (Some(spec), false) => match control.launch(&spec) {
                Ok(()) => outcome.reopened.push(app.name.clone()),
                Err(error) => outcome.failed.push(format!("{}: {error}", app.name)),
            },
        }
    }
    outcome
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Mutex;

    /// In-memory fake of the OS. Processes listed in `stubborn` ignore close requests;
    /// processes in `protected_by_os` cannot be terminated either.
    #[derive(Default)]
    pub struct FakeControl {
        pub processes: Mutex<Vec<RunningProcess>>,
        pub details: HashMap<u32, ProcessDetails>,
        pub stubborn: HashSet<u32>,
        pub protected_by_os: HashSet<u32>,
        pub launched: Mutex<Vec<LaunchSpec>>,
        pub close_requests: Mutex<Vec<u32>>,
        /// Processes that show a visible window.
        pub windowed: HashSet<u32>,
    }

    impl FakeControl {
        pub fn with(processes: Vec<RunningProcess>) -> Self {
            Self {
                processes: Mutex::new(processes),
                ..Self::default()
            }
        }

        pub fn running(&self) -> Vec<u32> {
            self.processes
                .lock()
                .unwrap()
                .iter()
                .map(|p| p.pid)
                .collect()
        }

        fn kill(&self, pid: u32) {
            self.processes.lock().unwrap().retain(|p| p.pid != pid);
        }
    }

    impl ProcessControl for FakeControl {
        fn list(&self) -> Result<Vec<RunningProcess>, ProcessError> {
            Ok(self.processes.lock().unwrap().clone())
        }
        fn describe(&self, pid: u32) -> ProcessDetails {
            self.details.get(&pid).cloned().unwrap_or_default()
        }
        fn has_visible_window(&self, pids: &[u32]) -> bool {
            pids.iter().any(|pid| self.windowed.contains(pid))
        }
        fn request_close(&self, pids: &[u32]) {
            self.close_requests.lock().unwrap().extend_from_slice(pids);
            for &pid in pids {
                if !self.stubborn.contains(&pid) {
                    self.kill(pid);
                }
            }
        }
        fn wait_for_exit(&self, pids: &[u32], _timeout: Duration) -> Vec<u32> {
            let running = self.running();
            pids.iter()
                .copied()
                .filter(|pid| running.contains(pid))
                .collect()
        }
        fn terminate(&self, pid: u32) -> Result<(), ProcessError> {
            if self.protected_by_os.contains(&pid) {
                return Err(ProcessError::AccessDenied);
            }
            self.kill(pid);
            Ok(())
        }
        fn launch(&self, spec: &LaunchSpec) -> Result<(), ProcessError> {
            self.launched.lock().unwrap().push(spec.clone());
            Ok(())
        }
    }

    pub fn process(pid: u32, parent_pid: u32, exe_name: &str) -> RunningProcess {
        RunningProcess {
            pid,
            parent_pid,
            exe_name: exe_name.to_owned(),
        }
    }

    pub fn catalog() -> Catalog {
        Catalog::from_json(
            r#"{"version":1,"entries":[
                {"id":"slack","name":"Slack","category":"Chat","processes":["slack"]},
                {"id":"ollama","name":"Ollama","category":"AI","processes":["ollama app","ollama"]},
                {"id":"toys","name":"PowerToys","category":"Overlay","processes":["PowerToys.*"],"relaunch":false}
            ]}"#,
        )
        .unwrap()
    }

    #[test]
    fn shipped_catalog_is_valid() {
        let catalog = Catalog::from_json(include_str!("../../../catalog/apps.json")).unwrap();
        assert!(catalog.entries.len() > 20);
        assert!(catalog.entry("antigravity").is_some());
    }

    #[test]
    fn catalog_rejects_duplicates_and_empty_process_lists() {
        let duplicate = r#"{"version":1,"entries":[
            {"id":"a","name":"A","category":"x","processes":["a"]},
            {"id":"a","name":"B","category":"x","processes":["b"]}]}"#;
        assert!(matches!(
            Catalog::from_json(duplicate),
            Err(CatalogError::Entry { .. })
        ));
        let empty =
            r#"{"version":1,"entries":[{"id":"a","name":"A","category":"x","processes":[]}]}"#;
        assert!(matches!(
            Catalog::from_json(empty),
            Err(CatalogError::Entry { .. })
        ));
        let blank_service =
            r#"{"version":1,"entries":[{"id":"a","name":"A","category":"x","services":[" "]}]}"#;
        assert!(matches!(
            Catalog::from_json(blank_service),
            Err(CatalogError::Entry { .. })
        ));
        let service_only =
            r#"{"version":1,"entries":[{"id":"a","name":"A","category":"x","services":["A"]}]}"#;
        assert!(Catalog::from_json(service_only).is_ok());
        assert!(matches!(
            Catalog::from_json("nope"),
            Err(CatalogError::Json(_))
        ));
    }

    #[test]
    fn matching_ignores_case_and_exe_suffix_and_supports_prefixes() {
        assert!(process_matches("slack", "Slack.exe"));
        assert!(process_matches("Slack.exe", "slack"));
        assert!(!process_matches("slack", "slackware.exe"));
        assert!(process_matches("PowerToys.*", "PowerToys.FancyZones.exe"));
        assert!(!process_matches("PowerToys.*", "PowerToys.exe"));
        assert!(process_matches("Wispr Flow", "Wispr Flow.exe"));
    }

    #[test]
    fn protected_chain_follows_parents() {
        let processes = vec![
            process(10, 4, "explorer.exe"),
            process(20, 10, "pwsh.exe"),
            process(30, 20, "examsafe.exe"),
        ];
        assert_eq!(protected_pids(30, &processes), vec![30, 20, 10]);
    }

    #[test]
    fn finds_apps_with_their_main_processes() {
        let processes = vec![
            process(1, 900, "slack.exe"),
            process(2, 1, "slack.exe"),
            process(3, 900, "ollama app.exe"),
            process(4, 3, "ollama.exe"),
            process(5, 900, "notepad.exe"),
        ];
        let findings = match_apps(&catalog(), &processes, &[]);
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].entry_id, "slack");
        assert_eq!(findings[0].pids, vec![1, 2]);
        assert_eq!(findings[0].root_pids, vec![1]);
        assert_eq!(findings[1].entry_id, "ollama");
        assert_eq!(findings[1].root_pids, vec![3]);
    }

    #[test]
    fn protected_processes_are_never_matched() {
        let processes = vec![process(1, 900, "slack.exe")];
        assert!(match_apps(&catalog(), &processes, &[1]).is_empty());
    }

    #[test]
    fn store_app_id_beats_path_for_relaunch() {
        let path_only = ProcessDetails {
            path: Some("C:\\a.exe".into()),
            ..ProcessDetails::default()
        };
        let packaged = ProcessDetails {
            path: Some("C:\\WindowsApps\\b.exe".into()),
            app_id: Some("Pkg!App".into()),
            command_line: Some("\"C:\\WindowsApps\\b.exe\" --hidden".into()),
            working_dir: None,
        };
        let spec = pick_launch(&[path_only.clone(), packaged], true).unwrap();
        assert_eq!(spec.target, LaunchTarget::AppId("Pkg!App".into()));
        assert_eq!(
            spec.arguments.as_deref(),
            Some("--hidden"),
            "arguments come from the same process as the target"
        );
        assert_eq!(
            pick_launch(&[path_only], true),
            Some(LaunchSpec::new(LaunchTarget::Path("C:\\a.exe".into())))
        );
        assert_eq!(pick_launch(&[ProcessDetails::default()], true), None);
    }

    #[test]
    fn relaunch_keeps_arguments_folder_and_background_state() {
        let tray_app = ProcessDetails {
            path: Some("C:\\Apps\\Discord\\Discord.exe".into()),
            command_line: Some(
                "\"C:\\Apps\\Discord\\Discord.exe\" --processStart Discord.exe --start-minimized"
                    .into(),
            ),
            working_dir: Some("C:\\Apps\\Discord\\".into()),
            app_id: None,
        };
        assert_eq!(
            pick_launch(&[tray_app], false),
            Some(LaunchSpec {
                target: LaunchTarget::Path("C:\\Apps\\Discord\\Discord.exe".into()),
                arguments: Some("--processStart Discord.exe --start-minimized".into()),
                working_dir: Some("C:\\Apps\\Discord\\".into()),
                background: true,
            })
        );
    }

    #[test]
    fn command_line_arguments_follow_windows_quoting() {
        assert_eq!(
            command_line_arguments("\"C:\\Program Files\\A\\a.exe\" --x \"y z\""),
            Some("--x \"y z\"".into())
        );
        assert_eq!(
            command_line_arguments("C:\\a.exe   -s  "),
            Some("-s".into())
        );
        assert_eq!(command_line_arguments("\"C:\\Program Files\\a.exe\""), None);
        assert_eq!(command_line_arguments("C:\\a.exe"), None);
        assert_eq!(command_line_arguments(""), None);
        assert_eq!(command_line_arguments("\"unterminated"), None);
    }

    #[test]
    fn closed_app_journal_entry_round_trips_the_launch_spec() {
        let spec = LaunchSpec {
            target: LaunchTarget::Path("C:\\a.exe".into()),
            arguments: Some("--tray".into()),
            working_dir: Some("C:\\".into()),
            background: true,
        };
        let closed = ClosedApp::new("a", "A", vec!["a".into()], Some(&spec));
        assert!(!closed.had_window);
        assert_eq!(closed.launch_spec(), Some(spec));
        let none = ClosedApp::new("b", "B", vec!["b".into()], None);
        assert!(none.had_window);
        assert_eq!(none.launch_spec(), None);
    }

    #[test]
    fn close_asks_first_then_force_closes_stubborn_apps() {
        let mut control = FakeControl::with(vec![
            process(1, 900, "slack.exe"),
            process(3, 900, "ollama app.exe"),
        ]);
        control.stubborn.insert(3);
        let findings = match_apps(&catalog(), &control.list().unwrap(), &[]);
        let outcome = close_apps(&control, &findings, Duration::ZERO);
        assert_eq!(outcome.closed, vec!["Slack", "Ollama"]);
        assert!(outcome.failed.is_empty());
        assert!(control.running().is_empty());
        assert_eq!(*control.close_requests.lock().unwrap(), vec![1, 3]);
    }

    #[test]
    fn close_reports_apps_the_os_refuses_to_close() {
        let mut control = FakeControl::with(vec![process(1, 900, "slack.exe")]);
        control.stubborn.insert(1);
        control.protected_by_os.insert(1);
        let findings = match_apps(&catalog(), &control.list().unwrap(), &[]);
        let outcome = close_apps(&control, &findings, Duration::ZERO);
        assert!(outcome.closed.is_empty());
        assert_eq!(outcome.failed.len(), 1);
        assert!(outcome.failed[0].starts_with("Slack: "));
    }

    #[test]
    fn restore_reopens_only_what_is_not_running_and_has_a_target() {
        let control = FakeControl::with(vec![]);
        let slack = LaunchSpec {
            target: LaunchTarget::Path("C:\\slack.exe".into()),
            arguments: Some("--startup".into()),
            working_dir: None,
            background: true,
        };
        let ollama = LaunchSpec::new(LaunchTarget::Path("C:\\ollama.exe".into()));
        let closed = vec![
            ClosedApp::new("slack", "Slack", vec!["slack".into()], Some(&slack)),
            ClosedApp::new("ollama", "Ollama", vec!["ollama".into()], Some(&ollama)),
            ClosedApp::new("toys", "PowerToys", vec!["PowerToys.*".into()], None),
        ];
        let running = vec![process(9, 900, "ollama.exe")];
        let outcome = restore_apps(&control, &closed, &running);
        assert_eq!(outcome.reopened, vec!["Slack"]);
        assert_eq!(outcome.skipped, vec!["Ollama", "PowerToys"]);
        assert_eq!(*control.launched.lock().unwrap(), vec![slack]);
    }
}
