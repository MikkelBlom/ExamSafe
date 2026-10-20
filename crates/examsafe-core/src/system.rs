//! Services and scheduled tasks: matching them against the catalog, planning the changes that
//! pause them for an exam, journaling their previous state, and planning the exact restore.
//! Pure logic; reading goes through [`crate::ports::SystemInventory`], changing through the
//! elevated helper ([`crate::ports::PrivilegedExecutor`]).

use serde::{Deserialize, Serialize};

use crate::apps::{Catalog, name_matches};

/// How Windows starts a service. `AutomaticDelayed` is "Automatic (Delayed Start)".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartType {
    Boot,
    System,
    Automatic,
    AutomaticDelayed,
    Manual,
    Disabled,
}

impl StartType {
    pub fn label(self) -> &'static str {
        match self {
            Self::Boot => "Boot",
            Self::System => "System",
            Self::Automatic => "Automatic",
            Self::AutomaticDelayed => "Automatic (delayed)",
            Self::Manual => "Manual",
            Self::Disabled => "Disabled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceState {
    Stopped,
    Running,
    /// Starting, stopping, pausing or continuing.
    Pending,
    Paused,
}

impl ServiceState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Stopped => "Stopped",
            Self::Running => "Running",
            Self::Pending => "Changing state",
            Self::Paused => "Paused",
        }
    }
}

/// One installed service, as reported by the OS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceInfo {
    pub name: String,
    pub display_name: String,
    pub start_type: StartType,
    pub state: ServiceState,
    /// Process id while running (0 when stopped); several services can share one process.
    pub pid: u32,
}

/// One registered scheduled task, as reported by the OS. `path` is the full task path, e.g.
/// `\PowerToys\Autorun for mikke`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskInfo {
    pub path: String,
    pub enabled: bool,
    pub running: bool,
}

/// What a catalog entry's services are set to for the exam.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceTarget {
    /// Cannot be started by anything until restored. The strongest reversible lever.
    #[default]
    Disabled,
    /// Can still be started on demand by Windows or another program.
    Manual,
}

impl ServiceTarget {
    pub fn start_type(self) -> StartType {
        match self {
            Self::Disabled => StartType::Disabled,
            Self::Manual => StartType::Manual,
        }
    }
}

/// A catalog service that is installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceFinding {
    pub entry_id: String,
    pub app_name: String,
    pub category: String,
    pub info: ServiceInfo,
    pub target: StartType,
}

impl ServiceFinding {
    /// Stable id for preferences and selection: `service:<name>`.
    pub fn key(&self) -> String {
        service_key(&self.info.name)
    }

    /// Something to do: it is not yet stopped with the target start type.
    pub fn needs_change(&self) -> bool {
        self.info.state != ServiceState::Stopped || self.info.start_type != self.target
    }

    pub fn label(&self) -> String {
        format!("{} (service)", self.info.display_name)
    }
}

/// A catalog scheduled task that is registered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskFinding {
    pub entry_id: String,
    pub app_name: String,
    pub category: String,
    pub info: TaskInfo,
}

impl TaskFinding {
    /// Stable id for preferences and selection: `task:<path>`.
    pub fn key(&self) -> String {
        task_key(&self.info.path)
    }

    pub fn needs_change(&self) -> bool {
        self.info.enabled || self.info.running
    }

    pub fn label(&self) -> String {
        format!("{} (scheduled task)", task_name(&self.info.path))
    }
}

pub fn service_key(name: &str) -> String {
    format!("service:{name}")
}

pub fn task_key(path: &str) -> String {
    format!("task:{path}")
}

/// The last part of a task path: `\A\B\Name` -> `Name`.
pub fn task_name(path: &str) -> &str {
    path.rsplit('\\').next().unwrap_or(path)
}

// ----- Never touched ---------------------------------------------------------------------------

/// Security software (never disabled — invariant 6) and exam software (never touched — invariant
/// 4). Prefix patterns (trailing `*`) or exact names, case-insensitive.
const PROTECTED_SERVICES: &[&str] = &[
    // Windows security
    "WinDefend",
    "WdNisSvc",
    "WdBoot",
    "WdFilter",
    "Sense",
    "MDCoreSvc",
    "SecurityHealthService",
    "wscsvc",
    "mpssvc",
    "BFE",
    "EventLog",
    "RpcSs",
    // Bitdefender (incl. its VPN tunnel)
    "BD*",
    "bdredline*",
    "bdvpnservice",
    "VSSERV",
    "UPDATESRV",
    "ProductAgentService",
    "WireGuardTunnel$bdvpnservice*",
    "EPSecurityService",
    "EPProtectedService",
    "EPIntegrationService",
    "EPUpdateService",
    "EPRedline",
    // Other antivirus / endpoint protection
    "avast*",
    "avg*",
    "Avira*",
    "McAfee*",
    "mfe*",
    "Norton*",
    "ekrn",
    "Sophos*",
    "CSFalconService",
    "SentinelAgent",
    "AVP*",
    "klnagent",
    "MBAMService",
    "WRSVC",
];

const PROTECTED_TASK_PREFIXES: &[&str] = &[
    "\\Microsoft\\Windows\\Windows Defender\\*",
    "\\Microsoft\\Windows\\ExploitGuard\\*",
    "\\Bitdefender*",
];

/// Words that mark exam software, matched anywhere in a name or path.
const EXAM_SOFTWARE_WORDS: &[&str] = &[
    "exammonitor",
    "digitalexam",
    "digiexam",
    "safeexambrowser",
    "safe exam browser",
    "proctor",
    "respondus",
    "lockdownbrowser",
    "wiseflow",
    "flowlock",
    "examcookie",
];

fn is_exam_software(name: &str) -> bool {
    let lower = name.to_lowercase();
    EXAM_SOFTWARE_WORDS.iter().any(|word| lower.contains(word))
}

/// Services ExamSafe never changes, whatever the catalog or a request says.
pub fn is_protected_service(name: &str) -> bool {
    is_exam_software(name)
        || PROTECTED_SERVICES
            .iter()
            .any(|pattern| name_matches(pattern, name))
}

/// Scheduled tasks ExamSafe never changes, whatever the catalog or a request says.
pub fn is_protected_task(path: &str) -> bool {
    is_exam_software(path)
        || PROTECTED_TASK_PREFIXES
            .iter()
            .any(|pattern| name_matches(pattern, path))
}

// ----- Matching --------------------------------------------------------------------------------

/// Installed services that belong to a catalog entry. Each service belongs to the first entry
/// that lists it; protected services are never matched.
pub fn match_services(catalog: &Catalog, services: &[ServiceInfo]) -> Vec<ServiceFinding> {
    let mut findings: Vec<ServiceFinding> = Vec::new();
    for entry in &catalog.entries {
        for service in services {
            let listed = entry
                .services
                .iter()
                .any(|pattern| name_matches(pattern, &service.name));
            let claimed = findings.iter().any(|f| f.info.name == service.name);
            if listed && !claimed && !is_protected_service(&service.name) {
                findings.push(ServiceFinding {
                    entry_id: entry.id.clone(),
                    app_name: entry.name.clone(),
                    category: entry.category.clone(),
                    info: service.clone(),
                    target: entry.service_start.start_type(),
                });
            }
        }
    }
    findings
}

/// Registered tasks that belong to a catalog entry; protected tasks are never matched.
pub fn match_tasks(catalog: &Catalog, tasks: &[TaskInfo]) -> Vec<TaskFinding> {
    let mut findings: Vec<TaskFinding> = Vec::new();
    for entry in &catalog.entries {
        for task in tasks {
            let listed = entry
                .tasks
                .iter()
                .any(|pattern| name_matches(pattern, &task.path));
            let claimed = findings.iter().any(|f| f.info.path == task.path);
            if listed && !claimed && !is_protected_task(&task.path) {
                findings.push(TaskFinding {
                    entry_id: entry.id.clone(),
                    app_name: entry.name.clone(),
                    category: entry.category.clone(),
                    info: task.clone(),
                });
            }
        }
    }
    findings
}

// ----- Journal ---------------------------------------------------------------------------------

/// A service's state before exam mode changed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceSnapshot {
    pub name: String,
    pub display_name: String,
    pub start_type: StartType,
    pub was_running: bool,
    /// What exam mode sets it to.
    pub target: StartType,
}

impl ServiceSnapshot {
    pub fn of(finding: &ServiceFinding) -> Self {
        Self {
            name: finding.info.name.clone(),
            display_name: finding.info.display_name.clone(),
            start_type: finding.info.start_type,
            was_running: matches!(
                finding.info.state,
                ServiceState::Running | ServiceState::Pending
            ),
            target: finding.target,
        }
    }
}

/// A scheduled task's state before exam mode changed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskSnapshot {
    pub path: String,
    pub enabled: bool,
}

impl TaskSnapshot {
    pub fn of(finding: &TaskFinding) -> Self {
        Self {
            path: finding.info.path.clone(),
            enabled: finding.info.enabled,
        }
    }
}

// ----- Changes ---------------------------------------------------------------------------------

/// One change the elevated helper makes. Each states the full desired end state, so applying it
/// twice is harmless (a retried fix or restore never makes things worse).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SystemChange {
    /// Set the start type, then make sure it is running (`true`) or stopped (`false`).
    Service {
        name: String,
        start_type: StartType,
        running: bool,
    },
    /// Enable or disable the task; disabling also stops a running instance.
    Task { path: String, enabled: bool },
}

impl SystemChange {
    pub fn subject(&self) -> &str {
        match self {
            Self::Service { name, .. } => name,
            Self::Task { path, .. } => path,
        }
    }

    pub fn is_protected(&self) -> bool {
        match self {
            Self::Service { name, .. } => is_protected_service(name),
            Self::Task { path, .. } => is_protected_task(path),
        }
    }
}

/// The changes that pause these items, in the safe order: tasks first (so nothing restarts a
/// service), then services (start type first, then stop).
pub fn pause_plan(services: &[ServiceSnapshot], tasks: &[TaskSnapshot]) -> Vec<SystemChange> {
    let tasks = tasks.iter().map(|task| SystemChange::Task {
        path: task.path.clone(),
        enabled: false,
    });
    let services = services.iter().map(|service| SystemChange::Service {
        name: service.name.clone(),
        start_type: service.target,
        running: false,
    });
    tasks.chain(services).collect()
}

/// The changes that put everything back exactly as recorded, in reverse order: services first
/// (their exact start type, delayed start included, and running state), then tasks.
pub fn restore_plan(services: &[ServiceSnapshot], tasks: &[TaskSnapshot]) -> Vec<SystemChange> {
    let services = services.iter().map(|service| SystemChange::Service {
        name: service.name.clone(),
        start_type: service.start_type,
        running: service.was_running,
    });
    let tasks = tasks.iter().map(|task| SystemChange::Task {
        path: task.path.clone(),
        enabled: task.enabled,
    });
    services.chain(tasks).collect()
}

/// Display names of journaled items that are not (yet) paused, judged from a fresh read.
/// A service or task that no longer exists counts as paused (nothing can run it).
pub fn still_active(
    services: &[ServiceSnapshot],
    tasks: &[TaskSnapshot],
    current_services: &[ServiceInfo],
    current_tasks: &[TaskInfo],
) -> Vec<String> {
    let mut active = Vec::new();
    for snapshot in services {
        if let Some(now) = current_services.iter().find(|s| s.name == snapshot.name)
            && (now.state != ServiceState::Stopped || now.start_type != snapshot.target)
        {
            active.push(format!("{} (service)", snapshot.display_name));
        }
    }
    for snapshot in tasks {
        if let Some(now) = current_tasks.iter().find(|t| t.path == snapshot.path)
            && (now.enabled || now.running)
        {
            active.push(format!("{} (scheduled task)", task_name(&snapshot.path)));
        }
    }
    active
}

/// Display names of journaled items that are not back in their recorded state. One that no
/// longer exists (uninstalled meanwhile) has nothing to put back and is not reported.
pub fn not_restored(
    services: &[ServiceSnapshot],
    tasks: &[TaskSnapshot],
    current_services: &[ServiceInfo],
    current_tasks: &[TaskInfo],
) -> Vec<String> {
    let mut wrong = Vec::new();
    for snapshot in services {
        let back = current_services
            .iter()
            .find(|s| s.name == snapshot.name)
            .is_none_or(|now| {
                now.start_type == snapshot.start_type
                    && (!snapshot.was_running || now.state == ServiceState::Running)
            });
        if !back {
            wrong.push(format!("{} (service)", snapshot.display_name));
        }
    }
    for snapshot in tasks {
        let back = current_tasks
            .iter()
            .find(|t| t.path == snapshot.path)
            .is_none_or(|now| now.enabled == snapshot.enabled);
        if !back {
            wrong.push(format!("{} (scheduled task)", task_name(&snapshot.path)));
        }
    }
    wrong
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn service(name: &str, start_type: StartType, state: ServiceState) -> ServiceInfo {
        ServiceInfo {
            name: name.to_owned(),
            display_name: format!("{name} display"),
            start_type,
            state,
            pid: 0,
        }
    }

    pub fn task(path: &str, enabled: bool) -> TaskInfo {
        TaskInfo {
            path: path.to_owned(),
            enabled,
            running: false,
        }
    }

    fn catalog() -> Catalog {
        Catalog::from_json(
            r#"{"version":1,"entries":[
                {"id":"tv","name":"TeamViewer","category":"Remote access","processes":["TeamViewer"],
                 "services":["TeamViewer"]},
                {"id":"sql","name":"SQL Server","category":"Work","services":["MSSQL$*","MSSQLSERVER"],
                 "service_start":"manual"},
                {"id":"toys","name":"PowerToys","category":"Overlay","tasks":["\\PowerToys\\Autorun for *"]},
                {"id":"greedy","name":"Greedy","category":"x","services":["BD*","Sense"],
                 "tasks":["\\Microsoft\\Windows\\Windows Defender\\*"]}
            ]}"#,
        )
        .unwrap()
    }

    #[test]
    fn matches_services_with_patterns_and_their_target() {
        let services = vec![
            service("TeamViewer", StartType::Automatic, ServiceState::Running),
            service("MSSQL$SQLEXPRESS", StartType::Manual, ServiceState::Stopped),
            service(
                "MSSQLSERVER",
                StartType::AutomaticDelayed,
                ServiceState::Running,
            ),
            service("Spooler", StartType::Automatic, ServiceState::Running),
        ];
        let found = match_services(&catalog(), &services);
        let names: Vec<_> = found.iter().map(|f| f.info.name.as_str()).collect();
        assert_eq!(names, vec!["TeamViewer", "MSSQL$SQLEXPRESS", "MSSQLSERVER"]);
        assert_eq!(found[0].target, StartType::Disabled);
        assert_eq!(found[1].target, StartType::Manual);
        assert!(
            !found[1].needs_change(),
            "already manual + stopped: nothing to do"
        );
        assert!(found[2].needs_change());
        assert_eq!(found[0].key(), "service:TeamViewer");
    }

    #[test]
    fn security_and_exam_software_are_never_matched_even_if_listed() {
        let services = vec![
            service("BDProtSrv", StartType::Automatic, ServiceState::Running),
            service("Sense", StartType::Automatic, ServiceState::Running),
        ];
        assert!(match_services(&catalog(), &services).is_empty());
        let tasks = vec![task(
            "\\Microsoft\\Windows\\Windows Defender\\Windows Defender Scheduled Scan",
            true,
        )];
        assert!(match_tasks(&catalog(), &tasks).is_empty());
        assert!(is_protected_service("ExamMonitorService"));
        assert!(is_protected_task(
            "\\SDU\\Digital Exam\\DigitalExam Updater"
        ));
        assert!(is_protected_service("WireGuardTunnel$bdvpnservice_2"));
        assert!(!is_protected_service("TeamViewer"));
    }

    #[test]
    fn matches_tasks_by_full_path() {
        let tasks = vec![
            task("\\PowerToys\\Autorun for mikke", true),
            task("\\PowerToys\\Other", true),
        ];
        let found = match_tasks(&catalog(), &tasks);
        assert_eq!(found.len(), 1);
        assert!(found[0].needs_change());
        assert_eq!(found[0].label(), "Autorun for mikke (scheduled task)");
        assert_eq!(found[0].key(), "task:\\PowerToys\\Autorun for mikke");
    }

    fn snapshots() -> (Vec<ServiceSnapshot>, Vec<TaskSnapshot>) {
        let services = vec![ServiceSnapshot {
            name: "TeamViewer".into(),
            display_name: "TeamViewer".into(),
            start_type: StartType::AutomaticDelayed,
            was_running: true,
            target: StartType::Disabled,
        }];
        let tasks = vec![TaskSnapshot {
            path: "\\PowerToys\\Autorun for mikke".into(),
            enabled: true,
        }];
        (services, tasks)
    }

    #[test]
    fn pause_disables_tasks_before_services() {
        let (services, tasks) = snapshots();
        assert_eq!(
            pause_plan(&services, &tasks),
            vec![
                SystemChange::Task {
                    path: "\\PowerToys\\Autorun for mikke".into(),
                    enabled: false
                },
                SystemChange::Service {
                    name: "TeamViewer".into(),
                    start_type: StartType::Disabled,
                    running: false
                },
            ]
        );
    }

    #[test]
    fn restore_puts_back_exact_start_type_and_state_services_first() {
        let (services, tasks) = snapshots();
        assert_eq!(
            restore_plan(&services, &tasks),
            vec![
                SystemChange::Service {
                    name: "TeamViewer".into(),
                    start_type: StartType::AutomaticDelayed,
                    running: true
                },
                SystemChange::Task {
                    path: "\\PowerToys\\Autorun for mikke".into(),
                    enabled: true
                },
            ]
        );
    }

    #[test]
    fn snapshot_records_delayed_start_and_running_state() {
        let finding = ServiceFinding {
            entry_id: "tv".into(),
            app_name: "TeamViewer".into(),
            category: "Remote access".into(),
            info: service(
                "TeamViewer",
                StartType::AutomaticDelayed,
                ServiceState::Running,
            ),
            target: StartType::Disabled,
        };
        let snapshot = ServiceSnapshot::of(&finding);
        assert_eq!(snapshot.start_type, StartType::AutomaticDelayed);
        assert!(snapshot.was_running);
        assert_eq!(snapshot.target, StartType::Disabled);
    }

    #[test]
    fn verification_names_what_is_still_active_or_not_back() {
        let (services, tasks) = snapshots();
        let running = vec![service(
            "TeamViewer",
            StartType::Disabled,
            ServiceState::Running,
        )];
        let enabled = vec![task("\\PowerToys\\Autorun for mikke", true)];
        assert_eq!(
            still_active(&services, &tasks, &running, &enabled),
            vec!["TeamViewer (service)", "Autorun for mikke (scheduled task)"]
        );
        let paused = vec![service(
            "TeamViewer",
            StartType::Disabled,
            ServiceState::Stopped,
        )];
        let disabled = vec![task("\\PowerToys\\Autorun for mikke", false)];
        assert!(still_active(&services, &tasks, &paused, &disabled).is_empty());
        // Gone entirely: nothing can run it.
        assert!(still_active(&services, &tasks, &[], &[]).is_empty());

        assert_eq!(not_restored(&services, &tasks, &paused, &disabled).len(), 2);
        let wrong_type = vec![service(
            "TeamViewer",
            StartType::Automatic,
            ServiceState::Running,
        )];
        assert_eq!(
            not_restored(&services, &tasks, &wrong_type, &enabled),
            vec!["TeamViewer (service)"],
            "delayed start must come back as delayed"
        );
        let back = vec![service(
            "TeamViewer",
            StartType::AutomaticDelayed,
            ServiceState::Running,
        )];
        assert!(not_restored(&services, &tasks, &back, &enabled).is_empty());
        assert!(
            not_restored(&services, &tasks, &[], &[]).is_empty(),
            "uninstalled meanwhile: nothing to put back"
        );
    }

    #[test]
    fn changes_round_trip_through_json_and_know_what_is_protected() {
        let change = SystemChange::Service {
            name: "WinDefend".into(),
            start_type: StartType::Disabled,
            running: false,
        };
        let json = serde_json::to_string(&change).unwrap();
        assert_eq!(serde_json::from_str::<SystemChange>(&json).unwrap(), change);
        assert!(change.is_protected());
        assert!(
            !SystemChange::Task {
                path: "\\PowerToys\\x".into(),
                enabled: false
            }
            .is_protected()
        );
    }
}
