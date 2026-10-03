//! Application service: the real work behind each flow effect, expressed over the ports.
//! Used by both the window and the command line, so they cannot drift apart.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use crate::apps::{
    AppFinding, Catalog, CloseOutcome, ClosedApp, RestoreOutcome, close_apps, match_apps,
    pick_launch, protected_pids, restore_apps,
};
use crate::exam_mode::ExamModeRecord;
use crate::flow::{Effect, Event};
use crate::ports::{
    ExamModeRepository, PreferencesRepository, ProcessControl, ProcessError, StoreError,
};
use crate::preferences::Preferences;

/// How long apps get to close by themselves before they are force-closed.
pub const DEFAULT_GRACE: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ServiceError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("could not list running programs: {0}")]
    Process(#[from] ProcessError),
}

pub struct ExamService {
    control: Arc<dyn ProcessControl>,
    store: Arc<dyn ExamModeRepository>,
    preferences: Arc<dyn PreferencesRepository>,
    catalog: Arc<Catalog>,
    self_pid: u32,
    grace: Duration,
}

impl ExamService {
    pub fn new(
        control: Arc<dyn ProcessControl>,
        store: Arc<dyn ExamModeRepository>,
        preferences: Arc<dyn PreferencesRepository>,
        catalog: Arc<Catalog>,
        self_pid: u32,
        grace: Duration,
    ) -> Self {
        Self {
            control,
            store,
            preferences,
            catalog,
            self_pid,
            grace,
        }
    }

    /// Catalog apps running right now, with how to reopen each one.
    pub fn scan(&self) -> Result<Vec<AppFinding>, ServiceError> {
        let processes = self.control.list()?;
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
                    .map(|&pid| self.control.describe(pid))
                    .collect();
                finding.launch = pick_launch(&details);
            }
        }
        Ok(findings)
    }

    /// Catalog ids the user chose to leave open.
    pub fn left_open(&self) -> Result<Vec<String>, ServiceError> {
        Ok(self.preferences.load()?.left_open)
    }

    /// Remembers whether an app should be left open next time too.
    pub fn set_left_open(&self, entry_id: &str, left_open: bool) -> Result<(), ServiceError> {
        let mut preferences = self
            .preferences
            .load()
            .unwrap_or_else(|_| Preferences::default());
        preferences.set_left_open(entry_id, left_open);
        Ok(self.preferences.save(&preferences)?)
    }

    pub fn load_record(&self) -> Result<Option<ExamModeRecord>, ServiceError> {
        Ok(self.store.load()?)
    }

    /// Journal first: records the apps about to be closed. Keeps earlier entries on a retry.
    pub fn begin(&self, apps: &[AppFinding], now_unix: u64) -> Result<(), ServiceError> {
        let mut record = self
            .store
            .load()?
            .unwrap_or_else(|| ExamModeRecord::new(now_unix));
        for app in apps {
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
            record.closed_apps.push(ClosedApp {
                entry_id: app.entry_id.clone(),
                name: app.name.clone(),
                processes,
                launch: app.launch.clone(),
            });
        }
        self.store.save(&record)?;
        Ok(())
    }

    /// Re-scans (pids change when apps restart) and closes the selected apps.
    pub fn close(&self, entry_ids: &[String]) -> Result<CloseOutcome, ServiceError> {
        let targets = self.running_selection(entry_ids)?;
        Ok(close_apps(self.control.as_ref(), &targets, self.grace))
    }

    /// Names of selected apps that are still (or again) running.
    pub fn verify(&self, entry_ids: &[String]) -> Result<Vec<String>, ServiceError> {
        Ok(self
            .running_selection(entry_ids)?
            .into_iter()
            .map(|app| app.name)
            .collect())
    }

    pub fn restore(&self) -> Result<RestoreOutcome, ServiceError> {
        let Some(record) = self.store.load()? else {
            return Ok(RestoreOutcome::default());
        };
        let running = self.control.list()?;
        Ok(restore_apps(
            self.control.as_ref(),
            &record.closed_apps,
            &running,
        ))
    }

    pub fn finish(&self) -> Result<(), ServiceError> {
        Ok(self.store.clear()?)
    }

    fn running_selection(&self, entry_ids: &[String]) -> Result<Vec<AppFinding>, ServiceError> {
        let wanted: HashSet<&str> = entry_ids.iter().map(String::as_str).collect();
        Ok(self
            .scan()?
            .into_iter()
            .filter(|app| wanted.contains(app.entry_id.as_str()))
            .collect())
    }
}

/// What executing one effect produced.
#[derive(Debug, Default)]
pub struct EffectOutput {
    /// The event to feed back into the flow, if any.
    pub event: Option<Event>,
    /// A fresh scan (from `RunChecks`).
    pub findings: Option<Vec<AppFinding>>,
}

/// Executes one flow effect. `selection` is the apps the user agreed to close (after any the
/// user switched off in Advanced). `Err` means "stop executing the remaining effects".
/// `Effect::Quit` is not handled here — quitting is the caller's job.
pub fn run_effect(
    service: &ExamService,
    effect: Effect,
    disabled_ids: &[String],
    selection: &[AppFinding],
    now_unix: u64,
) -> Result<EffectOutput, String> {
    let ids: Vec<String> = selection.iter().map(|app| app.entry_id.clone()).collect();
    let output = match effect {
        Effect::RunChecks => {
            let findings = service.scan().map_err(|error| error.to_string())?;
            let issues = findings
                .iter()
                .filter(|app| !disabled_ids.contains(&app.entry_id))
                .count();
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
            let event = match service.close(&ids) {
                Ok(outcome) if outcome.failed.is_empty() => Event::FixCompleted,
                Ok(outcome) => Event::FixFailed {
                    reason: format!("Couldn't close {}.", outcome.failed.join("; ")),
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
            let remaining = service.verify(&ids).map_err(|error| error.to_string())?;
            EffectOutput {
                event: Some(Event::VerifyCompleted { remaining }),
                findings: None,
            }
        }
        Effect::RunRestore => {
            let event = match service.restore() {
                Ok(outcome) if outcome.failed.is_empty() => Event::RestoreCompleted,
                Ok(outcome) => Event::RestoreFailed {
                    reason: format!("Couldn't reopen {}.", outcome.failed.join("; ")),
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
mod tests {
    use super::*;
    use crate::apps::LaunchTarget;
    use crate::apps::tests::{FakeControl, catalog, process};
    use crate::flow::{Flow, Phase};
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

    const SELF_PID: u32 = 500;

    fn setup(control: FakeControl) -> (Arc<FakeControl>, Arc<MemoryStore>, ExamService) {
        let control = Arc::new(control);
        let store = Arc::new(MemoryStore::default());
        let service = ExamService::new(
            control.clone(),
            store.clone(),
            Arc::new(MemoryPreferences::default()),
            Arc::new(catalog()),
            SELF_PID,
            Duration::ZERO,
        );
        (control, store, service)
    }

    /// Drives the flow the way the UI and CLI do, answering the confirm step with "yes".
    fn drive(service: &ExamService, flow: &mut Flow, first: Event) {
        let mut queue = std::collections::VecDeque::from([first]);
        let mut findings: Vec<AppFinding> = Vec::new();
        while let Some(event) = queue.pop_front() {
            let effects = flow.handle(event).unwrap();
            for effect in effects {
                match run_effect(service, effect, &[], &findings, 1) {
                    Ok(output) => {
                        if let Some(new) = output.findings {
                            findings = new;
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
            crate::apps::ProcessDetails {
                path: Some("C:\\slack.exe".into()),
                app_id: None,
            },
        );
        let (control, store, service) = setup(control);

        let mut flow = Flow::new();
        flow.handle(Event::Launched {
            exam_mode_active: false,
        })
        .unwrap();
        drive(&service, &mut flow, Event::PrimaryPressed);

        assert_eq!(flow.phase(), Phase::Ready);
        assert_eq!(
            control.running(),
            vec![3, SELF_PID],
            "Slack closed, others untouched"
        );
        let record = store.load().unwrap().unwrap();
        assert_eq!(record.closed_apps.len(), 1);
        assert_eq!(
            record.closed_apps[0].launch,
            Some(LaunchTarget::Path("C:\\slack.exe".into()))
        );

        // Next launch: exam mode, then restore.
        let mut flow = Flow::new();
        flow.handle(Event::Launched {
            exam_mode_active: true,
        })
        .unwrap();
        drive(&service, &mut flow, Event::PrimaryPressed);
        assert_eq!(flow.phase(), Phase::Restored);
        assert_eq!(
            *control.launched.lock().unwrap(),
            vec![LaunchTarget::Path("C:\\slack.exe".into())]
        );
        assert!(
            store.load().unwrap().is_none(),
            "journal cleared after a clean restore"
        );
    }

    #[test]
    fn app_the_os_will_not_close_ends_in_failure_with_its_name() {
        let mut control = FakeControl::with(vec![process(1, 900, "slack.exe")]);
        control.stubborn.insert(1);
        control.protected_by_os.insert(1);
        let (_control, store, service) = setup(control);

        let mut flow = Flow::new();
        flow.handle(Event::Launched {
            exam_mode_active: false,
        })
        .unwrap();
        drive(&service, &mut flow, Event::PrimaryPressed);

        assert_eq!(flow.phase(), Phase::Failed);
        assert!(flow.failure().unwrap().contains("Slack"));
        assert!(
            store.load().unwrap().is_some(),
            "journal kept so the user can still restore"
        );
    }

    #[test]
    fn nothing_running_goes_straight_to_ready_without_a_journal() {
        let (_control, store, service) =
            setup(FakeControl::with(vec![process(3, 900, "notepad.exe")]));
        let mut flow = Flow::new();
        flow.handle(Event::Launched {
            exam_mode_active: false,
        })
        .unwrap();
        drive(&service, &mut flow, Event::PrimaryPressed);
        assert_eq!(flow.phase(), Phase::Ready);
        assert!(store.load().unwrap().is_none());
    }

    #[test]
    fn begin_keeps_earlier_entries_on_retry() {
        let (_control, store, service) =
            setup(FakeControl::with(vec![process(1, 900, "slack.exe")]));
        let first = service.scan().unwrap();
        service.begin(&first, 1).unwrap();
        service.begin(&first, 2).unwrap();
        let record = store.load().unwrap().unwrap();
        assert_eq!(record.closed_apps.len(), 1);
        assert_eq!(record.started_unix, 1);
    }

    #[test]
    fn left_open_choices_are_remembered() {
        let (_control, _store, service) = setup(FakeControl::with(vec![]));
        service.set_left_open("slack", true).unwrap();
        service.set_left_open("ollama", true).unwrap();
        service.set_left_open("slack", false).unwrap();
        assert_eq!(service.left_open().unwrap(), vec!["ollama"]);
    }

    #[test]
    fn disabled_apps_are_not_counted_or_closed() {
        let (control, _store, service) =
            setup(FakeControl::with(vec![process(1, 900, "slack.exe")]));
        let output =
            run_effect(&service, Effect::RunChecks, &["slack".to_owned()], &[], 1).unwrap();
        assert_eq!(output.event, Some(Event::ChecksCompleted { issues: 0 }));
        run_effect(&service, Effect::RunFix, &[], &[], 1).unwrap();
        assert_eq!(control.running(), vec![1]);
    }
}
