//! Application service: the real work behind each flow effect, expressed over the ports.
//! Used by both the window and the command line, so they cannot drift apart.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use crate::apps::{
    AppFinding, Catalog, ClosedApp, RestoreOutcome, close_apps, match_apps, pick_launch,
    protected_pids, restore_apps,
};
use crate::exam_mode::ExamModeRecord;
use crate::flow::{Effect, Event};
use crate::ports::{
    ExamModeRepository, HelperError, InventoryError, PreferencesRepository, PrivilegedExecutor,
    ProcessControl, ProcessError, StoreError, SystemInventory,
};
use crate::preferences::Preferences;
use crate::protocol::HelperAction;
use crate::system::{
    ServiceFinding, ServiceSnapshot, SystemChange, TaskFinding, TaskSnapshot, match_services,
    match_tasks, not_restored, pause_plan, restore_plan, service_key, still_active, task_key,
};

/// How long apps get to close by themselves before they are force-closed.
pub const DEFAULT_GRACE: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ServiceError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("could not list running programs: {0}")]
    Process(#[from] ProcessError),
    #[error(transparent)]
    Inventory(#[from] InventoryError),
}

/// Everything the service talks to.
pub struct Ports {
    pub processes: Arc<dyn ProcessControl>,
    pub inventory: Arc<dyn SystemInventory>,
    pub privileged: Arc<dyn PrivilegedExecutor>,
    pub store: Arc<dyn ExamModeRepository>,
    pub preferences: Arc<dyn PreferencesRepository>,
}

/// What a scan found: catalog apps that are running, and catalog services and scheduled tasks
/// that are installed (whether or not they need changing).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Findings {
    pub apps: Vec<AppFinding>,
    pub services: Vec<ServiceFinding>,
    pub tasks: Vec<TaskFinding>,
}

impl Findings {
    /// What exam mode acts on: running apps, and services and tasks that are not already off,
    /// minus everything the user chose to leave alone (by key, see [`Findings::keys`]).
    pub fn selection(&self, left_alone: &[String]) -> Self {
        let kept = |key: String| !left_alone.contains(&key);
        Self {
            apps: self
                .apps
                .iter()
                .filter(|app| kept(app.entry_id.clone()))
                .cloned()
                .collect(),
            services: self
                .services
                .iter()
                .filter(|service| service.needs_change() && kept(service.key()))
                .cloned()
                .collect(),
            tasks: self
                .tasks
                .iter()
                .filter(|task| task.needs_change() && kept(task.key()))
                .cloned()
                .collect(),
        }
    }

    pub fn len(&self) -> usize {
        self.apps.len() + self.services.len() + self.tasks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// One readable name per item, apps first.
    pub fn labels(&self) -> Vec<String> {
        self.apps
            .iter()
            .map(|app| app.name.clone())
            .chain(self.services.iter().map(ServiceFinding::label))
            .chain(self.tasks.iter().map(TaskFinding::label))
            .collect()
    }

    /// Item keys: catalog id for apps, `service:<name>`, `task:<path>`.
    pub fn keys(&self) -> HashSet<String> {
        self.apps
            .iter()
            .map(|app| app.entry_id.clone())
            .chain(self.services.iter().map(ServiceFinding::key))
            .chain(self.tasks.iter().map(TaskFinding::key))
            .collect()
    }
}

pub struct ExamService {
    ports: Ports,
    catalog: Arc<Catalog>,
    self_pid: u32,
    grace: Duration,
}

impl ExamService {
    pub fn new(ports: Ports, catalog: Arc<Catalog>, self_pid: u32, grace: Duration) -> Self {
        Self {
            ports,
            catalog,
            self_pid,
            grace,
        }
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    pub fn inventory(&self) -> &dyn SystemInventory {
        self.ports.inventory.as_ref()
    }

    pub fn processes(&self) -> &dyn ProcessControl {
        self.ports.processes.as_ref()
    }

    /// Catalog apps running right now (with how to reopen each one), and catalog services and
    /// scheduled tasks as they are right now.
    pub fn scan(&self) -> Result<Findings, ServiceError> {
        Ok(Findings {
            apps: self.scan_apps()?,
            services: match_services(&self.catalog, &self.ports.inventory.services()?),
            tasks: match_tasks(&self.catalog, &self.ports.inventory.tasks()?),
        })
    }

    fn scan_apps(&self) -> Result<Vec<AppFinding>, ServiceError> {
        let control = self.ports.processes.as_ref();
        let processes = control.list()?;
        let protected = protected_pids(self.self_pid, &processes);
        let mut findings = match_apps(&self.catalog, &processes, &protected);
        for finding in &mut findings {
            let relaunch = self
                .catalog
                .entry(&finding.entry_id)
                .is_some_and(|entry| entry.relaunch);
            if relaunch {
                let details: Vec<_> = finding
                    .root_pids
                    .iter()
                    .map(|&pid| control.describe(pid))
                    .collect();
                let had_window = control.has_visible_window(&finding.pids);
                finding.launch = pick_launch(&details, had_window);
            }
        }
        Ok(findings)
    }

    /// Keys of items the user chose to leave alone (see [`Findings::keys`]).
    pub fn left_open(&self) -> Result<Vec<String>, ServiceError> {
        Ok(self.ports.preferences.load()?.left_open)
    }

    /// Remembers whether an item should be left alone next time too.
    pub fn set_left_open(&self, key: &str, left_open: bool) -> Result<(), ServiceError> {
        let mut preferences = self
            .ports
            .preferences
            .load()
            .unwrap_or_else(|_| Preferences::default());
        preferences.set_left_open(key, left_open);
        Ok(self.ports.preferences.save(&preferences)?)
    }

    pub fn load_record(&self) -> Result<Option<ExamModeRecord>, ServiceError> {
        Ok(self.ports.store.load()?)
    }

    /// Journal first: records the state of everything about to be changed. On a retry, items
    /// already in the journal keep their first (original) state — never the changed one.
    pub fn begin(&self, selection: &Findings, now_unix: u64) -> Result<(), ServiceError> {
        let mut record = self
            .ports
            .store
            .load()?
            .unwrap_or_else(|| ExamModeRecord::new(now_unix));
        for app in &selection.apps {
            if record
                .closed_apps
                .iter()
                .any(|c| c.entry_id == app.entry_id)
            {
                continue;
            }
            let processes = self
                .catalog
                .entry(&app.entry_id)
                .map(|entry| entry.processes.clone())
                .unwrap_or_default();
            record.closed_apps.push(ClosedApp::new(
                &app.entry_id,
                &app.name,
                processes,
                app.launch.as_ref(),
            ));
        }
        for service in &selection.services {
            if !record.services.iter().any(|s| s.name == service.info.name) {
                record.services.push(ServiceSnapshot::of(service));
            }
        }
        for task in &selection.tasks {
            if !record.tasks.iter().any(|t| t.path == task.info.path) {
                record.tasks.push(TaskSnapshot::of(task));
            }
        }
        self.ports.store.save(&record)?;
        Ok(())
    }

    /// Pauses the selected services and tasks (through the elevated helper, in one go), then
    /// closes the selected apps. Only items that are in the journal are ever changed. Returns
    /// "Name: reason" for everything that failed.
    pub fn fix(&self, selection: &Findings) -> Result<Vec<String>, ServiceError> {
        let mut failed = Vec::new();
        let (services, tasks) = self.journaled_selection(selection)?;
        let current_services = self.ports.inventory.services()?;
        let current_tasks = self.ports.inventory.tasks()?;
        let services: Vec<ServiceSnapshot> = services
            .into_iter()
            .filter(|s| {
                !still_active(std::slice::from_ref(s), &[], &current_services, &[]).is_empty()
            })
            .collect();
        let tasks: Vec<TaskSnapshot> = tasks
            .into_iter()
            .filter(|t| !still_active(&[], std::slice::from_ref(t), &[], &current_tasks).is_empty())
            .collect();
        let changes = pause_plan(&services, &tasks);
        if let Err(reason) = self.apply(changes) {
            failed.push(reason);
        }

        let app_ids: HashSet<&str> = selection.apps.iter().map(|a| a.entry_id.as_str()).collect();
        let targets: Vec<AppFinding> = self
            .scan_apps()?
            .into_iter()
            .filter(|app| app_ids.contains(app.entry_id.as_str()))
            .collect();
        failed.extend(close_apps(self.ports.processes.as_ref(), &targets, self.grace).failed);
        Ok(failed)
    }

    /// Names of selected items that are still (or again) running or enabled.
    pub fn verify(&self, selection: &Findings) -> Result<Vec<String>, ServiceError> {
        let app_ids: HashSet<&str> = selection.apps.iter().map(|a| a.entry_id.as_str()).collect();
        let mut remaining: Vec<String> = self
            .scan_apps()?
            .into_iter()
            .filter(|app| app_ids.contains(app.entry_id.as_str()))
            .map(|app| app.name)
            .collect();
        let (services, tasks) = self.journaled_selection(selection)?;
        if !services.is_empty() || !tasks.is_empty() {
            remaining.extend(still_active(
                &services,
                &tasks,
                &self.ports.inventory.services()?,
                &self.ports.inventory.tasks()?,
            ));
        }
        Ok(remaining)
    }

    /// Puts services and tasks back exactly as journaled (one helper run), reopens apps, then
    /// checks that every service and task really is back.
    pub fn restore(&self) -> Result<RestoreOutcome, ServiceError> {
        let Some(record) = self.ports.store.load()? else {
            return Ok(RestoreOutcome::default());
        };
        let mut helper_failure = None;
        let has_system = !record.services.is_empty() || !record.tasks.is_empty();
        if has_system {
            helper_failure = self
                .apply(restore_plan(&record.services, &record.tasks))
                .err();
        }

        let running = self.ports.processes.list()?;
        let mut outcome =
            restore_apps(self.ports.processes.as_ref(), &record.closed_apps, &running);
        if has_system {
            let not_back = not_restored(
                &record.services,
                &record.tasks,
                &self.ports.inventory.services()?,
                &self.ports.inventory.tasks()?,
            );
            outcome.failed.extend(helper_failure);
            outcome.failed.extend(
                not_back
                    .into_iter()
                    .map(|name| format!("{name}: not back as it was")),
            );
        }
        Ok(outcome)
    }

    pub fn finish(&self) -> Result<(), ServiceError> {
        Ok(self.ports.store.clear()?)
    }

    /// The selection's services and tasks as recorded in the journal (the only ones that may be
    /// changed).
    fn journaled_selection(
        &self,
        selection: &Findings,
    ) -> Result<(Vec<ServiceSnapshot>, Vec<TaskSnapshot>), ServiceError> {
        let Some(record) = self.ports.store.load()? else {
            return Ok((Vec::new(), Vec::new()));
        };
        let keys = selection.keys();
        let services = record
            .services
            .into_iter()
            .filter(|s| keys.contains(&service_key(&s.name)))
            .collect();
        let tasks = record
            .tasks
            .into_iter()
            .filter(|t| keys.contains(&task_key(&t.path)))
            .collect();
        Ok((services, tasks))
    }

    /// Runs the changes through the elevated helper (one UAC prompt). Nothing to do: no prompt.
    fn apply(&self, changes: Vec<SystemChange>) -> Result<(), String> {
        if changes.is_empty() {
            return Ok(());
        }
        match self
            .ports
            .privileged
            .execute(HelperAction::ApplyChanges { changes })
        {
            Ok(_) => Ok(()),
            Err(HelperError::Failed(message)) => Err(message),
            Err(error) => Err(format!("services and scheduled tasks: {error}")),
        }
    }
}

/// What executing one effect produced.
#[derive(Debug, Default)]
pub struct EffectOutput {
    /// The event to feed back into the flow, if any.
    pub event: Option<Event>,
    /// A fresh scan (from `RunChecks`).
    pub findings: Option<Findings>,
}

/// Executes one flow effect. `selection` is what the user agreed to change (see
/// [`Findings::selection`]); `left_alone` are the keys the user switched off. `Err` means "stop
/// executing the remaining effects". `Effect::Quit` is not handled here — quitting is the
/// caller's job.
pub fn run_effect(
    service: &ExamService,
    effect: Effect,
    left_alone: &[String],
    selection: &Findings,
    now_unix: u64,
) -> Result<EffectOutput, String> {
    let output = match effect {
        Effect::RunChecks => {
            let findings = service.scan().map_err(|error| error.to_string())?;
            let issues = findings.selection(left_alone).len();
            EffectOutput {
                event: Some(Event::ChecksCompleted { issues }),
                findings: Some(findings),
            }
        }
        Effect::BeginExamMode => {
            service
                .begin(selection, now_unix)
                .map_err(|error| format!("Couldn't save the restore record: {error}"))?;
            EffectOutput::default()
        }
        Effect::RunFix => {
            let event = match service.fix(selection) {
                Ok(failed) if failed.is_empty() => Event::FixCompleted,
                Ok(failed) => Event::FixFailed {
                    reason: format!("Couldn't turn off {}.", failed.join("; ")),
                },
                Err(error) => Event::FixFailed {
                    reason: error.to_string(),
                },
            };
            EffectOutput {
                event: Some(event),
                findings: None,
            }
        }
        Effect::RunVerify => {
            let remaining = service
                .verify(selection)
                .map_err(|error| error.to_string())?;
            EffectOutput {
                event: Some(Event::VerifyCompleted { remaining }),
                findings: None,
            }
        }
        Effect::RunRestore => {
            let event = match service.restore() {
                Ok(outcome) if outcome.failed.is_empty() => Event::RestoreCompleted,
                Ok(outcome) => Event::RestoreFailed {
                    reason: format!("Couldn't put back {}.", outcome.failed.join("; ")),
                },
                Err(error) => Event::RestoreFailed {
                    reason: error.to_string(),
                },
            };
            EffectOutput {
                event: Some(event),
                findings: None,
            }
        }
        Effect::EndExamMode => {
            service.finish().map_err(|error| error.to_string())?;
            EffectOutput::default()
        }
        Effect::Quit => EffectOutput::default(),
    };
    Ok(output)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::apps::tests::{FakeControl, catalog, process};
    use crate::apps::{LaunchSpec, LaunchTarget, ProcessDetails};
    use crate::flow::{Flow, Phase};
    use crate::inventory::{ListeningPort, NetworkAdapter, StartupItem};
    use crate::protocol::{HelperResponse, PROTOCOL_VERSION};
    use crate::system::{ServiceInfo, ServiceState, StartType, TaskInfo};
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemoryStore(Mutex<Option<ExamModeRecord>>);

    impl ExamModeRepository for MemoryStore {
        fn load(&self) -> Result<Option<ExamModeRecord>, StoreError> {
            Ok(self.0.lock().unwrap().clone())
        }
        fn save(&self, record: &ExamModeRecord) -> Result<(), StoreError> {
            *self.0.lock().unwrap() = Some(record.clone());
            Ok(())
        }
        fn clear(&self) -> Result<(), StoreError> {
            *self.0.lock().unwrap() = None;
            Ok(())
        }
    }

    #[derive(Default)]
    struct MemoryPreferences(Mutex<Preferences>);

    impl PreferencesRepository for MemoryPreferences {
        fn load(&self) -> Result<Preferences, StoreError> {
            Ok(self.0.lock().unwrap().clone())
        }
        fn save(&self, preferences: &Preferences) -> Result<(), StoreError> {
            *self.0.lock().unwrap() = preferences.clone();
            Ok(())
        }
    }

    /// Services and tasks held in memory; the fake helper changes them.
    #[derive(Default)]
    pub struct FakeInventory {
        pub services: Mutex<Vec<ServiceInfo>>,
        pub tasks: Mutex<Vec<TaskInfo>>,
    }

    impl SystemInventory for FakeInventory {
        fn services(&self) -> Result<Vec<ServiceInfo>, InventoryError> {
            Ok(self.services.lock().unwrap().clone())
        }
        fn tasks(&self) -> Result<Vec<TaskInfo>, InventoryError> {
            Ok(self.tasks.lock().unwrap().clone())
        }
        fn startup_items(&self) -> Result<Vec<StartupItem>, InventoryError> {
            Ok(Vec::new())
        }
        fn listening_ports(&self) -> Result<Vec<ListeningPort>, InventoryError> {
            Ok(Vec::new())
        }
        fn network_adapters(&self) -> Result<Vec<NetworkAdapter>, InventoryError> {
            Ok(Vec::new())
        }
    }

    /// Applies changes to the fake inventory, like the real helper does to Windows.
    pub struct FakeHelper {
        pub inventory: Arc<FakeInventory>,
        pub calls: Mutex<Vec<Vec<SystemChange>>>,
        pub decline: Mutex<bool>,
    }

    impl PrivilegedExecutor for FakeHelper {
        fn execute(&self, action: HelperAction) -> Result<HelperResponse, HelperError> {
            if *self.decline.lock().unwrap() {
                return Err(HelperError::Declined);
            }
            let HelperAction::ApplyChanges { changes } = action else {
                return Err(HelperError::Failed("unexpected action".into()));
            };
            for change in &changes {
                match change {
                    SystemChange::Service {
                        name,
                        start_type,
                        running,
                    } => {
                        for service in self.inventory.services.lock().unwrap().iter_mut() {
                            if &service.name == name {
                                service.start_type = *start_type;
                                service.state = if *running {
                                    ServiceState::Running
                                } else {
                                    ServiceState::Stopped
                                };
                            }
                        }
                    }
                    SystemChange::Task { path, enabled } => {
                        for task in self.inventory.tasks.lock().unwrap().iter_mut() {
                            if &task.path == path {
                                task.enabled = *enabled;
                                task.running = false;
                            }
                        }
                    }
                }
            }
            self.calls.lock().unwrap().push(changes);
            Ok(HelperResponse {
                protocol: PROTOCOL_VERSION,
                id: String::new(),
                ok: true,
                elevated: true,
                message: String::new(),
                failures: Vec::new(),
            })
        }
    }

    const SELF_PID: u32 = 500;

    struct Setup {
        control: Arc<FakeControl>,
        inventory: Arc<FakeInventory>,
        helper: Arc<FakeHelper>,
        store: Arc<MemoryStore>,
        service: ExamService,
    }

    fn setup(control: FakeControl) -> Setup {
        setup_with(control, FakeInventory::default())
    }

    fn setup_with(control: FakeControl, inventory: FakeInventory) -> Setup {
        let control = Arc::new(control);
        let inventory = Arc::new(inventory);
        let helper = Arc::new(FakeHelper {
            inventory: inventory.clone(),
            calls: Mutex::new(Vec::new()),
            decline: Mutex::new(false),
        });
        let store = Arc::new(MemoryStore::default());
        let service = ExamService::new(
            Ports {
                processes: control.clone(),
                inventory: inventory.clone(),
                privileged: helper.clone(),
                store: store.clone(),
                preferences: Arc::new(MemoryPreferences::default()),
            },
            Arc::new(system_catalog()),
            SELF_PID,
            Duration::ZERO,
        );
        Setup {
            control,
            inventory,
            helper,
            store,
            service,
        }
    }

    /// The app catalog from `apps::tests` plus a service + task entry.
    fn system_catalog() -> Catalog {
        let mut catalog = catalog();
        let extra = Catalog::from_json(
            r#"{"version":1,"entries":[
                {"id":"tv","name":"TeamViewer","category":"Remote access","processes":["TeamViewer"],
                 "services":["TeamViewer"],"tasks":["\\TeamViewer\\*"]},
                {"id":"sql","name":"SQL Server","category":"Work","services":["MSSQLSERVER"],
                 "service_start":"manual"}
            ]}"#,
        )
        .unwrap();
        catalog.entries.extend(extra.entries);
        catalog
    }

    fn service(name: &str, start_type: StartType, state: ServiceState) -> ServiceInfo {
        ServiceInfo {
            name: name.to_owned(),
            display_name: name.to_owned(),
            start_type,
            state,
            pid: 0,
        }
    }

    fn task(path: &str, enabled: bool) -> TaskInfo {
        TaskInfo {
            path: path.to_owned(),
            enabled,
            running: false,
        }
    }

    /// Drives the flow the way the UI and CLI do, answering the confirm step with "yes".
    fn drive(service: &ExamService, flow: &mut Flow, first: Event) {
        drive_leaving(service, flow, first, &[]);
    }

    fn drive_leaving(service: &ExamService, flow: &mut Flow, first: Event, left_alone: &[String]) {
        let mut queue = std::collections::VecDeque::from([first]);
        let mut selection = Findings::default();
        while let Some(event) = queue.pop_front() {
            let effects = flow.handle(event).unwrap();
            for effect in effects {
                match run_effect(service, effect, left_alone, &selection, 1) {
                    Ok(output) => {
                        if let Some(found) = output.findings {
                            selection = found.selection(left_alone);
                        }
                        queue.extend(output.event);
                    }
                    Err(reason) => {
                        queue.push_back(Event::Failed { reason });
                        break;
                    }
                }
            }
            if flow.phase() == Phase::Confirm {
                queue.push_back(Event::PrimaryPressed);
            }
        }
    }

    fn launched(exam_mode_active: bool) -> Flow {
        let mut flow = Flow::new();
        flow.handle(Event::Launched { exam_mode_active }).unwrap();
        flow
    }

    #[test]
    fn end_to_end_make_safe_then_restore() {
        let mut control = FakeControl::with(vec![
            process(1, 900, "slack.exe"),
            process(2, 1, "slack.exe"),
            process(3, 900, "notepad.exe"),
            process(SELF_PID, 900, "examsafe.exe"),
        ]);
        control.details.insert(
            1,
            ProcessDetails {
                path: Some("C:\\slack.exe".into()),
                command_line: Some("\"C:\\slack.exe\" --startup".into()),
                working_dir: Some("C:\\Users\\me".into()),
                ..ProcessDetails::default()
            },
        );
        // Slack lived in the tray: no visible window.
        let s = setup(control);

        let mut flow = launched(false);
        drive(&s.service, &mut flow, Event::PrimaryPressed);

        assert_eq!(flow.phase(), Phase::Ready);
        assert_eq!(
            s.control.running(),
            vec![3, SELF_PID],
            "Slack closed, others untouched"
        );
        let record = s.store.load().unwrap().unwrap();
        assert_eq!(record.closed_apps.len(), 1);
        let recorded = &record.closed_apps[0];
        assert_eq!(
            recorded.launch,
            Some(LaunchTarget::Path("C:\\slack.exe".into()))
        );
        assert_eq!(recorded.arguments.as_deref(), Some("--startup"));
        assert_eq!(recorded.working_dir.as_deref(), Some("C:\\Users\\me"));
        assert!(!recorded.had_window);
        assert!(
            s.helper.calls.lock().unwrap().is_empty(),
            "no services or tasks: no admin prompt"
        );

        // Next launch: exam mode, then restore.
        let mut flow = launched(true);
        drive(&s.service, &mut flow, Event::PrimaryPressed);
        assert_eq!(flow.phase(), Phase::Restored);
        assert_eq!(
            *s.control.launched.lock().unwrap(),
            vec![LaunchSpec {
                target: LaunchTarget::Path("C:\\slack.exe".into()),
                arguments: Some("--startup".into()),
                working_dir: Some("C:\\Users\\me".into()),
                background: true,
            }],
            "started exactly as before, in the background"
        );
        assert!(
            s.store.load().unwrap().is_none(),
            "journal cleared after a clean restore"
        );
        assert!(s.helper.calls.lock().unwrap().is_empty());
    }

    fn system_setup() -> Setup {
        let inventory = FakeInventory::default();
        *inventory.services.lock().unwrap() = vec![
            service(
                "TeamViewer",
                StartType::AutomaticDelayed,
                ServiceState::Running,
            ),
            // Already as the exam wants it (manual + stopped): nothing to do.
            service("MSSQLSERVER", StartType::Manual, ServiceState::Stopped),
            service("Spooler", StartType::Automatic, ServiceState::Running),
        ];
        *inventory.tasks.lock().unwrap() = vec![
            task("\\TeamViewer\\Updater", true),
            task("\\Other\\Task", true),
        ];
        setup_with(FakeControl::with(vec![]), inventory)
    }

    #[test]
    fn services_and_tasks_are_journaled_paused_verified_and_restored_exactly() {
        let s = system_setup();
        let mut flow = launched(false);
        drive(&s.service, &mut flow, Event::PrimaryPressed);
        assert_eq!(flow.phase(), Phase::Ready, "{:?}", flow.failure());
        assert_eq!(flow.issues(), 2);

        // Journal holds the original state, including delayed start.
        let record = s.store.load().unwrap().unwrap();
        assert_eq!(record.services.len(), 1);
        assert_eq!(record.services[0].start_type, StartType::AutomaticDelayed);
        assert!(record.services[0].was_running);
        assert_eq!(record.tasks.len(), 1);
        assert!(record.tasks[0].enabled);

        // One helper run, tasks before services.
        let calls = s.helper.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 1);
        assert!(matches!(calls[0][0], SystemChange::Task { .. }));
        assert!(matches!(calls[0][1], SystemChange::Service { .. }));

        let services = s.inventory.services.lock().unwrap().clone();
        assert_eq!(
            services[0],
            service("TeamViewer", StartType::Disabled, ServiceState::Stopped)
        );
        assert_eq!(
            services[1],
            service("MSSQLSERVER", StartType::Manual, ServiceState::Stopped),
            "already off: untouched"
        );
        assert_eq!(
            services[2].state,
            ServiceState::Running,
            "not in the catalog: untouched"
        );
        assert!(!s.inventory.tasks.lock().unwrap()[0].enabled);
        assert!(s.inventory.tasks.lock().unwrap()[1].enabled);

        let mut flow = launched(true);
        drive(&s.service, &mut flow, Event::PrimaryPressed);
        assert_eq!(flow.phase(), Phase::Restored, "{:?}", flow.failure());
        assert_eq!(
            s.inventory.services.lock().unwrap()[0],
            service(
                "TeamViewer",
                StartType::AutomaticDelayed,
                ServiceState::Running
            ),
            "exact start type (delayed) and state are back"
        );
        assert!(s.inventory.tasks.lock().unwrap()[0].enabled);
        assert!(s.store.load().unwrap().is_none());
    }

    #[test]
    fn declined_admin_prompt_fails_with_the_journal_kept_for_restore() {
        let s = system_setup();
        *s.helper.decline.lock().unwrap() = true;
        let mut flow = launched(false);
        drive(&s.service, &mut flow, Event::PrimaryPressed);
        assert_eq!(flow.phase(), Phase::Failed);
        assert!(flow.failure().unwrap().contains("declined"));
        assert_eq!(s.store.load().unwrap().unwrap().services.len(), 1);
        assert_eq!(
            s.inventory.services.lock().unwrap()[0].state,
            ServiceState::Running,
            "nothing changed"
        );

        // Restore (from the failure screen) also needs the helper.
        *s.helper.decline.lock().unwrap() = false;
        flow.handle(Event::SecondaryPressed).unwrap();
        let output =
            run_effect(&s.service, Effect::RunRestore, &[], &Findings::default(), 1).unwrap();
        assert_eq!(output.event, Some(Event::RestoreCompleted));
    }

    #[test]
    fn retry_keeps_the_original_state_in_the_journal() {
        let s = system_setup();
        let first = s.service.scan().unwrap().selection(&[]);
        s.service.begin(&first, 1).unwrap();
        // Paused meanwhile (e.g. a first fix attempt got this far), then begin runs again.
        s.inventory.services.lock().unwrap()[0] =
            service("TeamViewer", StartType::Disabled, ServiceState::Stopped);
        let second = s.service.scan().unwrap().selection(&[]);
        s.service.begin(&second, 2).unwrap();
        let record = s.store.load().unwrap().unwrap();
        assert_eq!(record.services.len(), 1);
        assert_eq!(record.services[0].start_type, StartType::AutomaticDelayed);
    }

    #[test]
    fn items_left_alone_are_not_journaled_or_changed() {
        let s = system_setup();
        let left_alone = vec!["service:TeamViewer".to_owned()];
        let mut flow = launched(false);
        drive_leaving(&s.service, &mut flow, Event::PrimaryPressed, &left_alone);
        assert_eq!(flow.phase(), Phase::Ready);
        let record = s.store.load().unwrap().unwrap();
        assert!(record.services.is_empty());
        assert_eq!(record.tasks.len(), 1);
        assert_eq!(
            s.inventory.services.lock().unwrap()[0].state,
            ServiceState::Running
        );
    }

    #[test]
    fn a_service_that_restarts_is_paused_again_then_reported() {
        let s = system_setup();
        let selection = s.service.scan().unwrap().selection(&[]);
        s.service.begin(&selection, 1).unwrap();
        assert!(s.service.fix(&selection).unwrap().is_empty());
        assert!(s.service.verify(&selection).unwrap().is_empty());
        // Something starts it again.
        s.inventory.services.lock().unwrap()[0].state = ServiceState::Running;
        assert_eq!(
            s.service.verify(&selection).unwrap(),
            vec!["TeamViewer (service)"]
        );
        assert!(s.service.fix(&selection).unwrap().is_empty());
        assert_eq!(s.helper.calls.lock().unwrap().last().unwrap().len(), 1);
    }

    #[test]
    fn nothing_is_changed_that_is_not_in_the_journal() {
        let s = system_setup();
        let selection = s.service.scan().unwrap().selection(&[]);
        // No begin(): no journal, so fix must not touch services or tasks.
        assert!(s.service.fix(&selection).unwrap().is_empty());
        assert!(s.helper.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn app_with_a_window_is_recorded_to_come_back_with_its_window() {
        let mut control = FakeControl::with(vec![
            process(1, 900, "slack.exe"),
            process(2, 1, "slack.exe"),
        ]);
        control.details.insert(
            1,
            ProcessDetails {
                path: Some("C:\\slack.exe".into()),
                ..ProcessDetails::default()
            },
        );
        // The window belongs to a child process, as with most Electron apps.
        control.windowed.insert(2);
        let s = setup(control);
        let spec = s.service.scan().unwrap().apps[0].launch.clone().unwrap();
        assert!(!spec.background);
        assert_eq!(spec.arguments, None);
    }

    #[test]
    fn restore_skips_apps_that_are_already_running_again() {
        let mut control = FakeControl::with(vec![process(1, 900, "slack.exe")]);
        control.details.insert(
            1,
            ProcessDetails {
                path: Some("C:\\slack.exe".into()),
                ..ProcessDetails::default()
            },
        );
        let s = setup(control);
        let findings = s.service.scan().unwrap();
        s.service.begin(&findings, 1).unwrap();
        // The user started Slack again by hand before restoring.
        let outcome = s.service.restore().unwrap();
        assert_eq!(outcome.skipped, vec!["Slack"]);
        assert!(s.control.launched.lock().unwrap().is_empty());
    }

    #[test]
    fn app_the_os_will_not_close_ends_in_failure_with_its_name() {
        let mut control = FakeControl::with(vec![process(1, 900, "slack.exe")]);
        control.stubborn.insert(1);
        control.protected_by_os.insert(1);
        let s = setup(control);

        let mut flow = launched(false);
        drive(&s.service, &mut flow, Event::PrimaryPressed);

        assert_eq!(flow.phase(), Phase::Failed);
        assert!(flow.failure().unwrap().contains("Slack"));
        assert!(
            s.store.load().unwrap().is_some(),
            "journal kept so the user can still restore"
        );
    }

    #[test]
    fn nothing_running_goes_straight_to_ready_without_a_journal() {
        let s = setup(FakeControl::with(vec![process(3, 900, "notepad.exe")]));
        let mut flow = launched(false);
        drive(&s.service, &mut flow, Event::PrimaryPressed);
        assert_eq!(flow.phase(), Phase::Ready);
        assert!(s.store.load().unwrap().is_none());
    }

    #[test]
    fn begin_keeps_earlier_entries_on_retry() {
        let s = setup(FakeControl::with(vec![process(1, 900, "slack.exe")]));
        let first = s.service.scan().unwrap();
        s.service.begin(&first, 1).unwrap();
        s.service.begin(&first, 2).unwrap();
        let record = s.store.load().unwrap().unwrap();
        assert_eq!(record.closed_apps.len(), 1);
        assert_eq!(record.started_unix, 1);
    }

    #[test]
    fn left_open_choices_are_remembered() {
        let s = setup(FakeControl::with(vec![]));
        s.service.set_left_open("slack", true).unwrap();
        s.service.set_left_open("service:TeamViewer", true).unwrap();
        s.service.set_left_open("slack", false).unwrap();
        assert_eq!(s.service.left_open().unwrap(), vec!["service:TeamViewer"]);
    }

    #[test]
    fn disabled_apps_are_not_counted_or_closed() {
        let s = setup(FakeControl::with(vec![process(1, 900, "slack.exe")]));
        let output = run_effect(
            &s.service,
            Effect::RunChecks,
            &["slack".to_owned()],
            &Findings::default(),
            1,
        )
        .unwrap();
        assert_eq!(output.event, Some(Event::ChecksCompleted { issues: 0 }));
        run_effect(&s.service, Effect::RunFix, &[], &Findings::default(), 1).unwrap();
        assert_eq!(s.control.running(), vec![1]);
    }

    #[test]
    fn selection_labels_and_keys_cover_every_kind() {
        let s = system_setup();
        let selection = s.service.scan().unwrap().selection(&[]);
        assert_eq!(
            selection.labels(),
            vec!["TeamViewer (service)", "Updater (scheduled task)"]
        );
        assert!(selection.keys().contains("task:\\TeamViewer\\Updater"));
    }
}
