//! Glue between the pure [`Flow`] state machine, the UI and the platform adapters.
//!
//! Everything here runs on the UI thread. The only background work is the privileged helper
//! call, whose result is posted back with [`post`].

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use examsafe_core::exam_mode::ExamModeRecord;
use examsafe_core::flow::{Effect, Event, Flow, Phase as FlowPhase};
use examsafe_core::ports::{ExamModeRepository, PrivilegedExecutor};
use examsafe_core::protocol::{HelperAction, HelperResponse};
use examsafe_core::steps::{CHECK_STEPS, FIX_STEPS, RESTORE_STEPS, VERIFY_STEPS, step_states};
use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, VecModel};

use crate::sample_plan::SAMPLE_PLAN;
use crate::tray::{Tray, TrayCommand, TrayStatus};
use crate::{AppWindow, Phase, PlanItem, StepItem};

/// Pace of the simulated steps. Long enough to read, short enough not to be tedious.
const STEP_DURATION: Duration = Duration::from_millis(550);

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

pub struct Controller {
    ui: slint::Weak<AppWindow>,
    flow: RefCell<Flow>,
    store: Box<dyn ExamModeRepository>,
    executor: Arc<dyn PrivilegedExecutor>,
    tray: Tray,
    steps: Rc<VecModel<StepItem>>,
    plan: Rc<VecModel<PlanItem>>,
    /// Bumped whenever a new step sequence starts, so timers from an old one are ignored.
    generation: Cell<u64>,
}

impl Controller {
    pub fn new(
        ui: &AppWindow,
        store: Box<dyn ExamModeRepository>,
        executor: Arc<dyn PrivilegedExecutor>,
        tray: Tray,
    ) -> Rc<Self> {
        let steps = Rc::new(VecModel::default());
        let plan = Rc::new(VecModel::from(
            SAMPLE_PLAN
                .iter()
                .map(|item| PlanItem {
                    category: item.category.into(),
                    name: item.name.into(),
                    method: item.method.into(),
                    enabled: true,
                })
                .collect::<Vec<_>>(),
        ));
        ui.set_steps(ModelRc::from(Rc::clone(&steps)));
        ui.set_plan_items(ModelRc::from(Rc::clone(&plan)));

        let controller = Rc::new(Self {
            ui: ui.as_weak(),
            flow: RefCell::new(Flow::new()),
            store,
            executor,
            tray,
            steps,
            plan,
            generation: Cell::new(0),
        });

        let weak = Rc::downgrade(&controller);
        ui.on_primary_action(move || {
            if let Some(controller) = weak.upgrade() {
                controller.dispatch(Event::PrimaryPressed);
            }
        });
        let weak = Rc::downgrade(&controller);
        ui.on_plan_item_toggled(move |index, enabled| {
            if let Some(controller) = weak.upgrade() {
                controller.set_plan_item(index, enabled);
            }
        });
        controller
    }

    pub fn start(self: &Rc<Self>) {
        let exam_mode_active = match self.store.load() {
            Ok(record) => record.is_some(),
            Err(error) => {
                // Unreadable record: assume exam mode so the user is offered a restore rather
                // than silently losing track of changed items.
                eprintln!("examsafe: {error}");
                true
            }
        };
        self.dispatch(Event::Launched { exam_mode_active });
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
        for effect in effects {
            if let Err(reason) = self.apply(effect) {
                eprintln!("examsafe: {effect:?} failed: {reason}");
                // Never continue to later effects (e.g. RunFix) when an earlier one failed
                // (e.g. persisting exam mode): changes must only happen after they are recorded.
                if self.flow.borrow().phase() == FlowPhase::Fixing {
                    self.dispatch(Event::FixFailed { reason });
                }
                return;
            }
        }
    }

    fn apply(self: &Rc<Self>, effect: Effect) -> Result<(), String> {
        match effect {
            Effect::RunChecks => {
                // Prototype: simulated scan with a fixed number of findings.
                let issues = self.enabled_plan_items();
                self.run_steps(CHECK_STEPS, 0, Event::ChecksCompleted { issues });
            }
            Effect::BeginExamMode => {
                let record = ExamModeRecord::new(unix_now(), self.flow.borrow().issues());
                self.store
                    .save(&record)
                    .map_err(|error| error.to_string())?;
            }
            Effect::RunFix => self.run_fix(),
            Effect::RunVerify => {
                self.run_steps(VERIFY_STEPS, 0, Event::VerifyCompleted { clean: true });
            }
            Effect::RunRestore => self.run_steps(RESTORE_STEPS, 0, Event::RestoreCompleted),
            Effect::EndExamMode => self.store.clear().map_err(|error| error.to_string())?,
            Effect::Quit => slint::quit_event_loop().map_err(|error| error.to_string())?,
        }
        Ok(())
    }

    /// The real part of the prototype: a round trip to the elevated helper (UAC prompt).
    fn run_fix(self: &Rc<Self>) {
        let generation = self.next_generation();
        self.set_steps(FIX_STEPS, 0);
        self.set_helper_status("Waiting for administrator permission…");
        let executor = Arc::clone(&self.executor);
        std::thread::spawn(move || {
            let started = Instant::now();
            let result = executor.execute(HelperAction::Ping);
            let elapsed = started.elapsed();
            post(move |controller| controller.finish_fix(generation, result, elapsed));
        });
    }

    fn finish_fix(
        self: &Rc<Self>,
        generation: u64,
        result: Result<HelperResponse, examsafe_core::ports::HelperError>,
        elapsed: Duration,
    ) {
        if self.generation.get() != generation {
            return;
        }
        match result {
            Ok(response) => {
                self.set_helper_status(&format!(
                    "{} ({} ms)",
                    response.message,
                    elapsed.as_millis()
                ));
                self.run_steps(FIX_STEPS, 1, Event::FixCompleted);
            }
            Err(error) => {
                self.set_helper_status(&error.to_string());
                self.dispatch(Event::FixFailed {
                    reason: format!("Couldn't make changes: {error}."),
                });
            }
        }
    }

    /// Animates `labels` from `already_done` to completion, then dispatches `done`.
    fn run_steps(
        self: &Rc<Self>,
        labels: &'static [&'static str],
        already_done: usize,
        done: Event,
    ) {
        let generation = self.next_generation();
        self.set_steps(labels, already_done);
        let remaining = labels.len().saturating_sub(already_done).max(1);
        let mut done = Some(done);
        for offset in 1..=remaining {
            let completed = already_done + offset;
            let event = if offset == remaining {
                done.take()
            } else {
                None
            };
            let weak = Rc::downgrade(self);
            let delay = STEP_DURATION * u32::try_from(offset).unwrap_or(u32::MAX);
            Timer::single_shot(delay, move || {
                let Some(controller) = weak.upgrade() else {
                    return;
                };
                if controller.generation.get() != generation {
                    return;
                }
                controller.set_steps(labels, completed);
                if let Some(event) = event {
                    controller.dispatch(event);
                }
            });
        }
    }

    fn next_generation(&self) -> u64 {
        let next = self.generation.get() + 1;
        self.generation.set(next);
        next
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

    fn enabled_plan_items(&self) -> usize {
        self.plan.iter().filter(|item| item.enabled).count()
    }

    fn set_plan_item(&self, index: i32, enabled: bool) {
        let Ok(row) = usize::try_from(index) else {
            return;
        };
        if let Some(mut item) = self.plan.row_data(row) {
            item.enabled = enabled;
            self.plan.set_row_data(row, item);
        }
    }

    fn set_helper_status(&self, text: &str) {
        if let Some(ui) = self.ui.upgrade() {
            ui.set_helper_status(text.into());
        }
    }

    fn handle_tray(self: &Rc<Self>, command: TrayCommand) {
        match command {
            TrayCommand::Open => {
                if let Some(ui) = self.ui.upgrade()
                    && let Err(error) = ui.show()
                {
                    eprintln!("examsafe: could not show the window: {error}");
                }
            }
            TrayCommand::Quit => {
                if let Err(error) = slint::quit_event_loop() {
                    eprintln!("examsafe: could not quit: {error}");
                }
            }
        }
    }

    fn render(&self) {
        let flow = self.flow.borrow();
        if let Some(ui) = self.ui.upgrade() {
            ui.set_phase(to_ui_phase(flow.phase()));
            ui.set_issue_count(i32::try_from(flow.issues()).unwrap_or(i32::MAX));
            ui.set_failure_reason(flow.failure().unwrap_or_default().into());
        }
        let (status, tooltip) = tray_status(flow.phase());
        self.tray.set_status(status, tooltip);
    }
}

fn to_ui_phase(phase: FlowPhase) -> Phase {
    match phase {
        FlowPhase::Idle => Phase::Idle,
        FlowPhase::Checking => Phase::Checking,
        FlowPhase::Fixing => Phase::Fixing,
        FlowPhase::Verifying => Phase::Verifying,
        FlowPhase::Ready => Phase::Ready,
        FlowPhase::ExamMode => Phase::ExamMode,
        FlowPhase::Restoring => Phase::Restoring,
        FlowPhase::Restored => Phase::Restored,
        FlowPhase::Failed => Phase::Failed,
    }
}

fn tray_status(phase: FlowPhase) -> (TrayStatus, &'static str) {
    match phase {
        FlowPhase::Idle => (TrayStatus::Neutral, "ExamSafe: not checked yet"),
        FlowPhase::Ready => (TrayStatus::Safe, "ExamSafe: ready for your exam"),
        FlowPhase::Restored => (TrayStatus::Safe, "ExamSafe: back to normal"),
        FlowPhase::ExamMode => (
            TrayStatus::ExamMode,
            "ExamSafe: exam mode is on, restore when done",
        ),
        FlowPhase::Failed => (TrayStatus::Problem, "ExamSafe: needs attention"),
        FlowPhase::Checking | FlowPhase::Fixing | FlowPhase::Verifying | FlowPhase::Restoring => {
            (TrayStatus::Busy, "ExamSafe: working…")
        }
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}
