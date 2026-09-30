//! The exam flow as a pure state machine.
//!
//! The UI feeds it [`Event`]s and executes the [`Effect`]s it returns. Keeping the rules here
//! (instead of in UI callbacks) means every transition — including the failure and retry paths
//! that matter most for trust — is covered by plain unit tests.

/// How many fix → verify rounds we try before telling the user something keeps coming back.
pub const MAX_FIX_ATTEMPTS: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Checking,
    /// Apps were found; waiting for the user to agree to close them (unsaved work!).
    Confirm,
    Fixing,
    Verifying,
    Ready,
    ExamMode,
    Restoring,
    Restored,
    Failed,
}

impl Phase {
    pub fn is_busy(self) -> bool {
        matches!(
            self,
            Self::Checking | Self::Fixing | Self::Verifying | Self::Restoring
        )
    }
}

/// What "Try again" should do from the failed state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retry {
    Check,
    Fix,
    Restore,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// App started; `exam_mode_active` comes from the persisted exam-mode record.
    Launched {
        exam_mode_active: bool,
    },
    /// The single big button was pressed (its meaning depends on the phase).
    PrimaryPressed,
    /// The small secondary action: Cancel on the confirm step, Restore on a failure.
    SecondaryPressed,
    ChecksCompleted {
        issues: usize,
    },
    FixCompleted,
    FixFailed {
        reason: String,
    },
    /// `remaining` names the selected apps that are still (or again) running.
    VerifyCompleted {
        remaining: Vec<String>,
    },
    RestoreCompleted,
    RestoreFailed {
        reason: String,
    },
    /// Any work step failed unexpectedly (e.g. programs could not be listed).
    Failed {
        reason: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    RunChecks,
    /// Persist "exam mode is on" BEFORE any change is made, so a crash can always be restored.
    BeginExamMode,
    RunFix,
    RunVerify,
    RunRestore,
    EndExamMode,
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("event {event} is not valid in phase {phase:?}")]
pub struct FlowError {
    pub phase: Phase,
    pub event: String,
}

#[derive(Debug, Clone)]
pub struct Flow {
    phase: Phase,
    issues: usize,
    fix_attempts: u8,
    exam_mode_active: bool,
    failure: Option<String>,
    retry: Retry,
}

impl Default for Flow {
    fn default() -> Self {
        Self::new()
    }
}

impl Flow {
    pub fn new() -> Self {
        Self {
            phase: Phase::Idle,
            issues: 0,
            fix_attempts: 0,
            exam_mode_active: false,
            failure: None,
            retry: Retry::Check,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn issues(&self) -> usize {
        self.issues
    }

    pub fn exam_mode_active(&self) -> bool {
        self.exam_mode_active
    }

    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    pub fn handle(&mut self, event: Event) -> Result<Vec<Effect>, FlowError> {
        use Phase as P;
        let description = format!("{event:?}");
        let effects = match (self.phase, event) {
            (_, Event::Launched { exam_mode_active }) => {
                *self = Self::new();
                self.exam_mode_active = exam_mode_active;
                self.phase = if exam_mode_active {
                    P::ExamMode
                } else {
                    P::Idle
                };
                Vec::new()
            }
            (P::Idle, Event::PrimaryPressed) => self.start_checks(),
            (P::Checking, Event::ChecksCompleted { issues }) => {
                self.issues = issues;
                if issues == 0 {
                    self.phase = P::Verifying;
                    vec![Effect::RunVerify]
                } else {
                    // Never close apps without asking: they may hold unsaved work.
                    self.phase = P::Confirm;
                    Vec::new()
                }
            }
            (P::Confirm, Event::PrimaryPressed) => self.begin_fix(),
            (P::Confirm, Event::SecondaryPressed) => {
                self.phase = P::Idle;
                Vec::new()
            }
            (P::Fixing, Event::FixCompleted) => {
                self.phase = P::Verifying;
                vec![Effect::RunVerify]
            }
            (P::Fixing, Event::FixFailed { reason }) => self.fail(reason, Retry::Fix),
            (P::Verifying, Event::VerifyCompleted { remaining }) => {
                if remaining.is_empty() {
                    self.phase = P::Ready;
                    Vec::new()
                } else if self.fix_attempts < MAX_FIX_ATTEMPTS {
                    self.begin_fix()
                } else {
                    self.fail(
                        format!("Still running after closing: {}.", remaining.join(", ")),
                        Retry::Fix,
                    )
                }
            }
            // "Close ExamSafe": the app must be gone before the exam starts.
            (P::Ready, Event::PrimaryPressed) => vec![Effect::Quit],
            (P::ExamMode, Event::PrimaryPressed) => self.start_restore(),
            (P::Restoring, Event::RestoreCompleted) => {
                self.exam_mode_active = false;
                self.phase = P::Restored;
                vec![Effect::EndExamMode]
            }
            (P::Restoring, Event::RestoreFailed { reason }) => self.fail(reason, Retry::Restore),
            (P::Restored, Event::PrimaryPressed) => {
                self.phase = P::Idle;
                Vec::new()
            }
            (P::Failed, Event::PrimaryPressed) => match self.retry {
                Retry::Check => self.start_checks(),
                Retry::Fix => {
                    self.fix_attempts = 0;
                    self.begin_fix()
                }
                Retry::Restore => self.start_restore(),
            },
            (P::Checking, Event::Failed { reason }) => self.fail(reason, Retry::Check),
            (P::Fixing | P::Verifying, Event::Failed { reason }) => self.fail(reason, Retry::Fix),
            (P::Restoring, Event::Failed { reason }) => self.fail(reason, Retry::Restore),
            // Give up on the attempt: put back whatever was already changed.
            (P::Failed, Event::SecondaryPressed) if self.exam_mode_active => self.start_restore(),
            (P::Failed, Event::SecondaryPressed) => {
                self.phase = P::Idle;
                Vec::new()
            }
            (phase, _) => {
                return Err(FlowError {
                    phase,
                    event: description,
                });
            }
        };
        Ok(effects)
    }

    fn start_checks(&mut self) -> Vec<Effect> {
        self.phase = Phase::Checking;
        self.issues = 0;
        self.fix_attempts = 0;
        self.failure = None;
        vec![Effect::RunChecks]
    }

    fn begin_fix(&mut self) -> Vec<Effect> {
        self.phase = Phase::Fixing;
        self.failure = None;
        self.fix_attempts += 1;
        let mut effects = Vec::with_capacity(2);
        if !self.exam_mode_active {
            self.exam_mode_active = true;
            effects.push(Effect::BeginExamMode);
        }
        effects.push(Effect::RunFix);
        effects
    }

    fn start_restore(&mut self) -> Vec<Effect> {
        self.phase = Phase::Restoring;
        self.failure = None;
        vec![Effect::RunRestore]
    }

    fn fail(&mut self, reason: String, retry: Retry) -> Vec<Effect> {
        self.phase = Phase::Failed;
        self.failure = Some(reason);
        self.retry = retry;
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flow_in(events: &[Event]) -> Flow {
        let mut flow = Flow::new();
        for event in events {
            flow.handle(event.clone()).unwrap();
        }
        flow
    }

    fn launched(exam_mode_active: bool) -> Event {
        Event::Launched { exam_mode_active }
    }

    #[test]
    fn fresh_launch_is_idle() {
        let flow = flow_in(&[launched(false)]);
        assert_eq!(flow.phase(), Phase::Idle);
        assert!(!flow.exam_mode_active());
    }

    #[test]
    fn launch_with_active_exam_mode_goes_straight_to_restore_screen() {
        let flow = flow_in(&[launched(true)]);
        assert_eq!(flow.phase(), Phase::ExamMode);
        assert!(flow.exam_mode_active());
    }

    #[test]
    fn primary_on_idle_starts_checks() {
        let mut flow = flow_in(&[launched(false)]);
        assert_eq!(
            flow.handle(Event::PrimaryPressed).unwrap(),
            vec![Effect::RunChecks]
        );
        assert_eq!(flow.phase(), Phase::Checking);
    }

    fn clean() -> Event {
        Event::VerifyCompleted {
            remaining: Vec::new(),
        }
    }

    fn dirty() -> Event {
        Event::VerifyCompleted {
            remaining: vec!["Slack".into()],
        }
    }

    /// Idle -> checks found 3 apps -> waiting for confirmation.
    fn confirming() -> Flow {
        flow_in(&[
            launched(false),
            Event::PrimaryPressed,
            Event::ChecksCompleted { issues: 3 },
        ])
    }

    #[test]
    fn found_apps_wait_for_confirmation_and_change_nothing() {
        let mut flow = flow_in(&[launched(false), Event::PrimaryPressed]);
        let effects = flow.handle(Event::ChecksCompleted { issues: 3 }).unwrap();
        assert!(effects.is_empty());
        assert_eq!(flow.phase(), Phase::Confirm);
        assert!(!flow.exam_mode_active());
    }

    #[test]
    fn confirming_persists_exam_mode_before_fixing() {
        let mut flow = confirming();
        let effects = flow.handle(Event::PrimaryPressed).unwrap();
        assert_eq!(effects, vec![Effect::BeginExamMode, Effect::RunFix]);
        assert_eq!(flow.phase(), Phase::Fixing);
        assert_eq!(flow.issues(), 3);
        assert!(flow.exam_mode_active());
    }

    #[test]
    fn cancelling_the_confirmation_changes_nothing() {
        let mut flow = confirming();
        assert!(flow.handle(Event::SecondaryPressed).unwrap().is_empty());
        assert_eq!(flow.phase(), Phase::Idle);
        assert!(!flow.exam_mode_active());
    }

    #[test]
    fn no_issues_skips_confirm_and_fix() {
        let mut flow = flow_in(&[launched(false), Event::PrimaryPressed]);
        let effects = flow.handle(Event::ChecksCompleted { issues: 0 }).unwrap();
        assert_eq!(effects, vec![Effect::RunVerify]);
        assert!(!flow.exam_mode_active());
    }

    #[test]
    fn happy_path_ends_in_ready_then_quit() {
        let mut flow = confirming();
        for event in [Event::PrimaryPressed, Event::FixCompleted, clean()] {
            flow.handle(event).unwrap();
        }
        assert_eq!(flow.phase(), Phase::Ready);
        assert_eq!(
            flow.handle(Event::PrimaryPressed).unwrap(),
            vec![Effect::Quit]
        );
    }

    #[test]
    fn app_that_came_back_is_closed_again_without_asking_twice() {
        let mut flow = confirming();
        flow.handle(Event::PrimaryPressed).unwrap();
        flow.handle(Event::FixCompleted).unwrap();
        assert_eq!(flow.handle(dirty()).unwrap(), vec![Effect::RunFix]);
        assert_eq!(flow.phase(), Phase::Fixing);
    }

    #[test]
    fn app_that_keeps_coming_back_fails_with_its_name() {
        let mut flow = confirming();
        for event in [
            Event::PrimaryPressed,
            Event::FixCompleted,
            dirty(),
            Event::FixCompleted,
        ] {
            flow.handle(event).unwrap();
        }
        assert!(flow.handle(dirty()).unwrap().is_empty());
        assert_eq!(flow.phase(), Phase::Failed);
        assert!(flow.failure().unwrap().contains("Slack"));
        // Exam mode stays on: whatever was changed must still be restorable.
        assert!(flow.exam_mode_active());
        assert_eq!(
            flow.handle(Event::PrimaryPressed).unwrap(),
            vec![Effect::RunFix]
        );
    }

    #[test]
    fn after_a_failed_fix_the_user_can_restore_instead() {
        let mut flow = confirming();
        flow.handle(Event::PrimaryPressed).unwrap();
        flow.handle(Event::FixFailed {
            reason: "access denied".into(),
        })
        .unwrap();
        assert_eq!(flow.phase(), Phase::Failed);
        assert_eq!(flow.failure(), Some("access denied"));
        assert_eq!(
            flow.handle(Event::SecondaryPressed).unwrap(),
            vec![Effect::RunRestore]
        );
        assert_eq!(flow.phase(), Phase::Restoring);
    }

    #[test]
    fn fix_failure_keeps_exam_mode_and_retries_fix() {
        let mut flow = confirming();
        flow.handle(Event::PrimaryPressed).unwrap();
        flow.handle(Event::FixFailed {
            reason: "declined".into(),
        })
        .unwrap();
        assert!(flow.exam_mode_active());
        assert_eq!(
            flow.handle(Event::PrimaryPressed).unwrap(),
            vec![Effect::RunFix]
        );
    }

    #[test]
    fn restore_path_clears_exam_mode() {
        let mut flow = flow_in(&[launched(true)]);
        assert_eq!(
            flow.handle(Event::PrimaryPressed).unwrap(),
            vec![Effect::RunRestore]
        );
        assert_eq!(
            flow.handle(Event::RestoreCompleted).unwrap(),
            vec![Effect::EndExamMode]
        );
        assert_eq!(flow.phase(), Phase::Restored);
        assert!(!flow.exam_mode_active());
        flow.handle(Event::PrimaryPressed).unwrap();
        assert_eq!(flow.phase(), Phase::Idle);
    }

    #[test]
    fn failed_restore_keeps_exam_mode_and_retries_restore() {
        let mut flow = flow_in(&[launched(true), Event::PrimaryPressed]);
        flow.handle(Event::RestoreFailed {
            reason: "not found".into(),
        })
        .unwrap();
        assert!(flow.exam_mode_active());
        assert_eq!(
            flow.handle(Event::PrimaryPressed).unwrap(),
            vec![Effect::RunRestore]
        );
    }

    #[test]
    fn primary_is_ignored_while_busy() {
        let mut flow = flow_in(&[launched(false), Event::PrimaryPressed]);
        let error = flow.handle(Event::PrimaryPressed).unwrap_err();
        assert_eq!(error.phase, Phase::Checking);
        assert_eq!(flow.phase(), Phase::Checking);
    }

    #[test]
    fn stale_events_are_rejected() {
        let mut flow = flow_in(&[launched(false)]);
        assert!(flow.handle(Event::FixCompleted).is_err());
        assert!(flow.handle(Event::RestoreCompleted).is_err());
        assert!(flow.handle(Event::SecondaryPressed).is_err());
    }

    #[test]
    fn unexpected_failures_retry_the_right_step() {
        let mut checking = flow_in(&[launched(false), Event::PrimaryPressed]);
        checking
            .handle(Event::Failed { reason: "x".into() })
            .unwrap();
        assert_eq!(
            checking.handle(Event::PrimaryPressed).unwrap(),
            vec![Effect::RunChecks]
        );

        let mut restoring = flow_in(&[launched(true), Event::PrimaryPressed]);
        restoring
            .handle(Event::Failed { reason: "x".into() })
            .unwrap();
        assert_eq!(
            restoring.handle(Event::PrimaryPressed).unwrap(),
            vec![Effect::RunRestore]
        );

        let mut idle = flow_in(&[launched(false)]);
        assert!(idle.handle(Event::Failed { reason: "x".into() }).is_err());
    }

    #[test]
    fn busy_phases() {
        assert!(Phase::Checking.is_busy());
        assert!(Phase::Restoring.is_busy());
        assert!(!Phase::Confirm.is_busy());
        assert!(!Phase::Ready.is_busy());
        assert!(!Phase::ExamMode.is_busy());
    }
}
