//! Glue between the pure [`Flow`] state machine, the UI and the [`ExamService`].
//!
//! Everything here runs on the UI thread except the effect jobs and scans, which run on worker
//! threads and post their results back with [`post`]. While a job runs, its progress steps
//! animate; the result is applied once both the work and a minimum display time are done, so
//! fast work still reads as deliberate and slow work never appears finished early.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use examsafe_core::advanced::{AdvancedRow, InventorySnapshot, advanced_rows};
use examsafe_core::flow::{Effect, Event, Flow, Phase as FlowPhase};
use examsafe_core::ports::InventoryError;
use examsafe_core::service::{EffectOutput, ExamService, Findings, run_effect};
use examsafe_core::steps::{CHECK_STEPS, FIX_STEPS, RESTORE_STEPS, VERIFY_STEPS, step_states};
use slint::{CloseRequestResponse, ComponentHandle, ModelRc, SharedString, Timer, VecModel};

use crate::tray::{Tray, TrayCommand, TrayMenuState, TrayStatus};
use crate::{AppWindow, Phase, PlanItem, StepItem};

/// Pace of the progress steps.
const STEP_DURATION: Duration = Duration::from_millis(450);
/// Pause on "all steps done" before moving on, so the last checkmark is seen.
const SETTLE_DURATION: Duration = Duration::from_millis(300);

thread_local! {
    static CURRENT: RefCell<Option<Rc<Controller>>> = const { RefCell::new(None) };
}

/// Makes the controller reachable from tray handlers and background threads.
pub fn install(controller: &Rc<Controller>) {
    CURRENT.with(|current| *current.borrow_mut() = Some(Rc::clone(controller)));
    controller.tray.install_handlers(on_tray_command);
}

/// Runs `job` on the UI thread with the controller. Safe to call from any thread.
fn post(job: impl FnOnce(&Rc<Controller>) + Send + 'static) {
    let result = slint::invoke_from_event_loop(move || {
        CURRENT.with(|current| {
            if let Some(controller) = current.borrow().as_ref() {
                job(controller);
            }
        });
    });
    if let Err(error) = result {
        eprintln!("examsafe: could not reach the UI thread: {error}");
    }
}

fn on_tray_command(command: TrayCommand) {
    post(move |controller| controller.handle_tray(command));
}

/// Result of a batch of effects executed on the worker thread.
struct JobResult {
    findings: Option<Findings>,
    events: Vec<Event>,
}

struct Job {
    generation: u64,
    animation_done: bool,
    result: Option<JobResult>,
}

pub struct Controller {
    ui: slint::Weak<AppWindow>,
    service: Arc<ExamService>,
    flow: RefCell<Flow>,
    tray: Tray,
    /// Last scan: catalog apps running, catalog services and tasks as they are.
    findings: RefCell<Findings>,
    /// Startup items, ports and adapters for Advanced (read-only).
    inventory: RefCell<InventorySnapshot>,
    /// Whether any scan has completed yet (so the UI can tell "none running" from "unknown").
    scanned: Cell<bool>,
    /// Keys of items the user switched off in Advanced (remembered in preferences).
    disabled: RefCell<Vec<String>>,
    /// The Advanced rows behind `plan`, in the same order.
    rows: RefCell<Vec<AdvancedRow>>,
    steps: Rc<VecModel<StepItem>>,
    plan: Rc<VecModel<PlanItem>>,
    app_names: Rc<VecModel<SharedString>>,
    job: RefCell<Option<Job>>,
    /// Bumped for every job, so results or timers from an older job are ignored.
    generation: Cell<u64>,
}

impl Controller {
    pub fn new(ui: &AppWindow, service: Arc<ExamService>, tray: Tray) -> Rc<Self> {
        let steps = Rc::new(VecModel::default());
        let plan = Rc::new(VecModel::default());
        let app_names = Rc::new(VecModel::default());
        ui.set_steps(ModelRc::from(Rc::clone(&steps)));
        ui.set_plan_items(ModelRc::from(Rc::clone(&plan)));
        ui.set_app_names(ModelRc::from(Rc::clone(&app_names)));

        let controller = Rc::new(Self {
            ui: ui.as_weak(),
            service,
            flow: RefCell::new(Flow::new()),
            tray,
            findings: RefCell::new(Findings::default()),
            inventory: RefCell::new(InventorySnapshot::default()),
            scanned: Cell::new(false),
            disabled: RefCell::new(Vec::new()),
            rows: RefCell::new(Vec::new()),
            steps,
            plan,
            app_names,
            job: RefCell::new(None),
            generation: Cell::new(0),
        });

        let weak = Rc::downgrade(&controller);
        ui.on_primary_action(move || {
            if let Some(controller) = weak.upgrade() {
                controller.dispatch(Event::PrimaryPressed);
            }
        });
        let weak = Rc::downgrade(&controller);
        ui.on_secondary_action(move || {
            if let Some(controller) = weak.upgrade() {
                controller.dispatch(Event::SecondaryPressed);
            }
        });
        let weak = Rc::downgrade(&controller);
        ui.on_plan_item_toggled(move |index, enabled| {
            if let Some(controller) = weak.upgrade() {
                controller.toggle_plan_item(index, enabled);
            }
        });
        let weak = Rc::downgrade(&controller);
        ui.window().on_close_requested(move || {
            weak.upgrade()
                .map_or(CloseRequestResponse::HideWindow, |controller| {
                    controller.close_requested()
                })
        });
        controller
    }

    pub fn start(self: &Rc<Self>) {
        match self.service.left_open() {
            Ok(left_open) => *self.disabled.borrow_mut() = left_open,
            // Not fatal: without saved choices everything on the list is turned off (the safe
            // side).
            Err(error) => eprintln!("examsafe: could not read preferences: {error}"),
        }
        let exam_mode_active = match self.service.load_record() {
            Ok(record) => record.is_some(),
            Err(error) => {
                // Unreadable record: assume exam mode so the user is offered a restore rather
                // than silently losing track of what was changed.
                eprintln!("examsafe: {error}");
                true
            }
        };
        self.dispatch(Event::Launched { exam_mode_active });
        if !exam_mode_active {
            self.refresh_scan();
        }
        self.refresh_inventory();
    }

    /// Background scan so Advanced shows what is there before anything is pressed.
    fn refresh_scan(&self) {
        let service = Arc::clone(&self.service);
        std::thread::spawn(move || match service.scan() {
            Ok(findings) => post(move |controller| {
                if controller.job.borrow().is_none() {
                    controller.set_findings(findings);
                }
            }),
            Err(error) => eprintln!("examsafe: background scan failed: {error}"),
        });
    }

    /// Background read of startup items, ports and adapters (read-only, for Advanced).
    fn refresh_inventory(&self) {
        let service = Arc::clone(&self.service);
        std::thread::spawn(move || {
            let inventory = service.inventory();
            let snapshot = InventorySnapshot {
                startup: or_empty(inventory.startup_items()),
                ports: or_empty(inventory.listening_ports()),
                adapters: or_empty(inventory.network_adapters()),
            };
            post(move |controller| {
                *controller.inventory.borrow_mut() = snapshot;
                controller.rebuild_plan();
            });
        });
    }

    fn dispatch(self: &Rc<Self>, event: Event) {
        let result = self.flow.borrow_mut().handle(event);
        let effects = match result {
            Ok(effects) => effects,
            Err(error) => {
                eprintln!("examsafe: ignored: {error}");
                return;
            }
        };
        self.render();
        if effects.contains(&Effect::Quit) {
            quit();
            return;
        }
        if effects.is_empty() {
            return;
        }
        if self.flow.borrow().phase().is_busy() {
            self.start_job(effects);
        } else {
            // Bookkeeping effects outside a work phase (e.g. clearing the record) are instant.
            let result = self.execute(&effects);
            self.apply_result(result);
        }
    }

    fn start_job(self: &Rc<Self>, effects: Vec<Effect>) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        *self.job.borrow_mut() = Some(Job {
            generation,
            animation_done: false,
            result: None,
        });

        let labels = step_labels(self.flow.borrow().phase());
        self.set_steps(labels, 0);
        // Animate all but the last step; the last one completes when the work is done.
        let animated = labels.len().saturating_sub(1);
        for completed in 1..=animated.max(1) {
            let weak = Rc::downgrade(self);
            let delay = STEP_DURATION * u32::try_from(completed).unwrap_or(u32::MAX);
            Timer::single_shot(delay, move || {
                let Some(controller) = weak.upgrade() else {
                    return;
                };
                if controller.generation.get() != generation {
                    return;
                }
                if completed <= animated {
                    controller.set_steps(labels, completed);
                }
                if completed == animated.max(1) {
                    if let Some(job) = controller.job.borrow_mut().as_mut() {
                        job.animation_done = true;
                    }
                    controller.try_finish_job(generation);
                }
            });
        }

        let service = Arc::clone(&self.service);
        let disabled = self.disabled.borrow().clone();
        let selection = self.selection();
        std::thread::spawn(move || {
            let result = execute_effects(&service, &effects, &disabled, &selection);
            post(move |controller| {
                if let Some(job) = controller.job.borrow_mut().as_mut()
                    && job.generation == generation
                {
                    job.result = Some(result);
                }
                controller.try_finish_job(generation);
            });
        });
    }

    fn try_finish_job(self: &Rc<Self>, generation: u64) {
        let ready = self.job.borrow().as_ref().is_some_and(|job| {
            job.generation == generation && job.animation_done && job.result.is_some()
        });
        if !ready {
            return;
        }
        let labels = step_labels(self.flow.borrow().phase());
        self.set_steps(labels, labels.len());
        let weak = Rc::downgrade(self);
        Timer::single_shot(SETTLE_DURATION, move || {
            let Some(controller) = weak.upgrade() else {
                return;
            };
            if controller.generation.get() != generation {
                return;
            }
            let result = controller
                .job
                .borrow_mut()
                .take()
                .and_then(|job| job.result);
            if let Some(result) = result {
                controller.apply_result(result);
            }
        });
    }

    fn execute(&self, effects: &[Effect]) -> JobResult {
        execute_effects(
            &self.service,
            effects,
            &self.disabled.borrow(),
            &self.selection(),
        )
    }

    fn apply_result(self: &Rc<Self>, result: JobResult) {
        if let Some(findings) = result.findings {
            self.set_findings(findings);
        }
        for event in result.events {
            self.dispatch(event);
        }
        let phase = self.flow.borrow().phase();
        if matches!(phase, FlowPhase::Restored | FlowPhase::Ready) {
            self.refresh_scan();
            self.refresh_inventory();
        }
    }

    /// What the user agreed to change: everything active on the list minus what they switched
    /// off.
    fn selection(&self) -> Findings {
        self.findings.borrow().selection(&self.disabled.borrow())
    }

    fn set_findings(&self, findings: Findings) {
        *self.findings.borrow_mut() = findings;
        self.scanned.set(true);
        self.rebuild_plan();
        self.render();
    }

    fn rebuild_plan(&self) {
        let rows = advanced_rows(
            self.service.catalog(),
            &self.findings.borrow(),
            &self.inventory.borrow(),
            &self.disabled.borrow(),
        );
        let items: Vec<PlanItem> = rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let first_in_section = index == 0 || rows[index - 1].section != row.section;
                PlanItem {
                    section: if first_in_section {
                        row.section.title().into()
                    } else {
                        SharedString::new()
                    },
                    name: row.name.as_str().into(),
                    detail: row.detail.as_str().into(),
                    state: row.state.as_str().into(),
                    action: row.action.as_str().into(),
                    toggleable: row.key.is_some(),
                    enabled: row.enabled,
                }
            })
            .collect();
        *self.rows.borrow_mut() = rows;
        self.plan.set_vec(items);
    }

    fn toggle_plan_item(&self, index: i32, enabled: bool) {
        let key = usize::try_from(index)
            .ok()
            .and_then(|row| self.rows.borrow().get(row).and_then(|r| r.key.clone()));
        let Some(key) = key else {
            return;
        };
        {
            let mut disabled = self.disabled.borrow_mut();
            disabled.retain(|id| id != &key);
            if !enabled {
                disabled.push(key.clone());
            }
        }
        if let Err(error) = self.service.set_left_open(&key, !enabled) {
            // The choice still applies to this run; it just is not remembered.
            eprintln!("examsafe: could not save preferences: {error}");
        }
        // Rows linked to the same item (e.g. its ports) change with it.
        self.rebuild_plan();
        self.render();
    }

    fn set_steps(&self, labels: &[&str], completed: usize) {
        let items: Vec<StepItem> = labels
            .iter()
            .zip(step_states(labels.len(), completed))
            .map(|(label, state)| StepItem {
                label: SharedString::from(*label),
                state: state as i32,
            })
            .collect();
        self.steps.set_vec(items);
    }

    /// Names for the confirm screen (will be turned off) or exam mode (will be put back).
    fn app_names_for(&self, phase: FlowPhase) -> Vec<SharedString> {
        match phase {
            FlowPhase::Confirm => self
                .selection()
                .labels()
                .into_iter()
                .map(SharedString::from)
                .collect(),
            FlowPhase::ExamMode => match self.service.load_record() {
                Ok(Some(record)) => record
                    .closed_apps
                    .iter()
                    .filter(|app| app.launch.is_some())
                    .map(|app| app.name.clone())
                    .chain(
                        record
                            .services
                            .iter()
                            .map(|s| format!("{} (service)", s.display_name)),
                    )
                    .chain(record.tasks.iter().map(|t| {
                        format!(
                            "{} (scheduled task)",
                            examsafe_core::system::task_name(&t.path)
                        )
                    }))
                    .map(SharedString::from)
                    .collect(),
                Ok(None) => Vec::new(),
                Err(error) => {
                    eprintln!("examsafe: {error}");
                    Vec::new()
                }
            },
            _ => Vec::new(),
        }
    }

    fn show_window(&self) {
        if let Some(ui) = self.ui.upgrade()
            && let Err(error) = ui.show()
        {
            eprintln!("examsafe: could not show the window: {error}");
        }
    }

    fn handle_tray(self: &Rc<Self>, command: TrayCommand) {
        let (phase, exam_mode_active) = {
            let flow = self.flow.borrow();
            (flow.phase(), flow.exam_mode_active())
        };
        match command {
            TrayCommand::Open => self.show_window(),
            // Same as pressing the big button: it checks, then asks before closing anything.
            TrayCommand::MakeSafe => {
                self.show_window();
                if matches!(phase, FlowPhase::Idle | FlowPhase::Restored) {
                    if phase == FlowPhase::Restored {
                        self.dispatch(Event::PrimaryPressed);
                    }
                    self.dispatch(Event::PrimaryPressed);
                }
            }
            TrayCommand::Restore => {
                self.show_window();
                match phase {
                    FlowPhase::ExamMode => self.dispatch(Event::PrimaryPressed),
                    FlowPhase::Failed if exam_mode_active => {
                        self.dispatch(Event::SecondaryPressed);
                    }
                    _ => {}
                }
            }
            TrayCommand::Quit => quit(),
        }
    }

    /// The window's X hides it to the tray. Exception: once the PC is ready for the exam,
    /// closing the window closes ExamSafe completely — it must not keep running in the tray
    /// during the exam.
    fn close_requested(&self) -> CloseRequestResponse {
        if self.flow.borrow().phase() == FlowPhase::Ready {
            quit();
        }
        CloseRequestResponse::HideWindow
    }

    fn render(&self) {
        let (phase, issues, failure, exam_mode_active) = {
            let flow = self.flow.borrow();
            (
                flow.phase(),
                flow.issues(),
                flow.failure().unwrap_or_default().to_owned(),
                flow.exam_mode_active(),
            )
        };
        self.app_names.set_vec(self.app_names_for(phase));
        let running = self.scanned.get().then(|| self.selection().len());
        if let Some(ui) = self.ui.upgrade() {
            ui.set_phase(to_ui_phase(phase));
            ui.set_issue_count(i32::try_from(issues).unwrap_or(i32::MAX));
            ui.set_failure_reason(failure.into());
            ui.set_exam_mode_active(exam_mode_active);
            ui.set_running_count(running.map_or(-1, |n| i32::try_from(n).unwrap_or(i32::MAX)));
        }
        let (status, tooltip) = tray_status(phase, running);
        self.tray.set_status(status, &tooltip);
        self.tray.set_menu_state(TrayMenuState {
            make_safe: matches!(phase, FlowPhase::Idle | FlowPhase::Restored),
            restore: phase == FlowPhase::ExamMode
                || (phase == FlowPhase::Failed && exam_mode_active),
        });
    }
}

/// Advanced is informational for these: a failed read shows an empty section, not an error.
fn or_empty<T>(result: Result<Vec<T>, InventoryError>) -> Vec<T> {
    result.unwrap_or_else(|error| {
        eprintln!("examsafe: {error}");
        Vec::new()
    })
}

fn quit() {
    if let Err(error) = slint::quit_event_loop() {
        eprintln!("examsafe: could not quit: {error}");
    }
}

fn execute_effects(
    service: &ExamService,
    effects: &[Effect],
    disabled: &[String],
    selection: &Findings,
) -> JobResult {
    let mut result = JobResult {
        findings: None,
        events: Vec::new(),
    };
    for &effect in effects {
        match run_effect(service, effect, disabled, selection, unix_now()) {
            Ok(EffectOutput { event, findings }) => {
                if findings.is_some() {
                    result.findings = findings;
                }
                result.events.extend(event);
            }
            Err(reason) => {
                // Stop here: later effects (e.g. closing apps) must not run when an earlier one
                // (e.g. saving the restore record) failed.
                result.events.push(Event::Failed { reason });
                break;
            }
        }
    }
    result
}

fn step_labels(phase: FlowPhase) -> &'static [&'static str] {
    match phase {
        FlowPhase::Checking => CHECK_STEPS,
        FlowPhase::Fixing => FIX_STEPS,
        FlowPhase::Verifying => VERIFY_STEPS,
        FlowPhase::Restoring => RESTORE_STEPS,
        _ => &[],
    }
}

fn to_ui_phase(phase: FlowPhase) -> Phase {
    match phase {
        FlowPhase::Idle => Phase::Idle,
        FlowPhase::Checking => Phase::Checking,
        FlowPhase::Confirm => Phase::Confirm,
        FlowPhase::Fixing => Phase::Fixing,
        FlowPhase::Verifying => Phase::Verifying,
        FlowPhase::Ready => Phase::Ready,
        FlowPhase::ExamMode => Phase::ExamMode,
        FlowPhase::Restoring => Phase::Restoring,
        FlowPhase::Restored => Phase::Restored,
        FlowPhase::Failed => Phase::Failed,
    }
}

/// Tray colour and hover text. `active` is how many listed items are on right now (once known).
pub fn tray_status(phase: FlowPhase, active: Option<usize>) -> (TrayStatus, String) {
    let text = |s: &str| s.to_owned();
    match phase {
        FlowPhase::Idle => match active {
            Some(0) => (
                TrayStatus::Neutral,
                text("ExamSafe: nothing on the exam list is on"),
            ),
            Some(1) => (
                TrayStatus::Neutral,
                text("ExamSafe: 1 item on the exam list is on"),
            ),
            Some(n) => (
                TrayStatus::Neutral,
                format!("ExamSafe: {n} items on the exam list are on"),
            ),
            None => (TrayStatus::Neutral, text("ExamSafe: not ready yet")),
        },
        FlowPhase::Confirm => (TrayStatus::Neutral, text("ExamSafe: waiting for your OK")),
        FlowPhase::Ready => (
            TrayStatus::Safe,
            text("ExamSafe: ready for the exam. Close ExamSafe now"),
        ),
        FlowPhase::Restored => (TrayStatus::Safe, text("ExamSafe: back to normal")),
        FlowPhase::ExamMode => (
            TrayStatus::ExamMode,
            text("ExamSafe: exam mode is on, restore when done"),
        ),
        FlowPhase::Failed => (TrayStatus::Problem, text("ExamSafe: needs attention")),
        FlowPhase::Checking => (TrayStatus::Busy, text("ExamSafe: checking…")),
        FlowPhase::Fixing => (TrayStatus::Busy, text("ExamSafe: turning things off…")),
        FlowPhase::Verifying => (TrayStatus::Busy, text("ExamSafe: double-checking…")),
        FlowPhase::Restoring => (TrayStatus::Busy, text("ExamSafe: putting things back…")),
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_tooltip_states_the_count_and_stays_short() {
        assert_eq!(
            tray_status(FlowPhase::Idle, Some(3)).1,
            "ExamSafe: 3 items on the exam list are on"
        );
        assert_eq!(
            tray_status(FlowPhase::Idle, Some(1)).1,
            "ExamSafe: 1 item on the exam list is on"
        );
        assert_eq!(
            tray_status(FlowPhase::Idle, None).1,
            "ExamSafe: not ready yet"
        );
        for phase in [
            FlowPhase::Idle,
            FlowPhase::Checking,
            FlowPhase::Confirm,
            FlowPhase::Fixing,
            FlowPhase::Verifying,
            FlowPhase::Ready,
            FlowPhase::ExamMode,
            FlowPhase::Restoring,
            FlowPhase::Restored,
            FlowPhase::Failed,
        ] {
            // Windows cuts tray tooltips at 127 characters.
            assert!(tray_status(phase, Some(999)).1.chars().count() < 128);
        }
        assert_eq!(
            tray_status(FlowPhase::ExamMode, None).0,
            TrayStatus::ExamMode
        );
    }
}
