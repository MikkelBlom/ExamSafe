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
    ChecksCompleted {
        issues: usize,
    },
    FixCompleted,
    FixFailed {
        reason: String,
    },
    VerifyCompleted {
        clean: bool,
    },
    RestoreCompleted,
    RestoreFailed {
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
                    self.begin_fix()
                }
            }
            (P::Fixing, Event::FixCompleted) => {
                self.phase = P::Verifying;
                vec![Effect::RunVerify]
            }
            (P::Fixing, Event::FixFailed { reason }) => self.fail(reason, Retry::Fix),
            (P::Verifying, Event::VerifyCompleted { clean: true }) => {
                self.phase = P::Ready;
                Vec::new()
            }
            (P::Verifying, Event::VerifyCompleted { clean: false }) => {
                if self.fix_attempts < MAX_FIX_ATTEMPTS {
                    self.begin_fix()
                } else {
                    self.fail(
                        "Some items keep turning themselves back on.".to_owned(),
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

    #[test]
    fn issues_found_persists_exam_mode_before_fixing() {
        let mut flow = flow_in(&[launched(false), Event::PrimaryPressed]);
        let effects = flow.handle(Event::ChecksCompleted { issues: 7 }).unwrap();
        assert_eq!(effects, vec![Effect::BeginExamMode, Effect::RunFix]);
        assert_eq!(flow.phase(), Phase::Fixing);
        assert_eq!(flow.issues(), 7);
        assert!(flow.exam_mode_active());
    }

    #[test]
    fn no_issues_skips_fix_and_verifies() {
        let mut flow = flow_in(&[launched(false), Event::PrimaryPressed]);
        let effects = flow.handle(Event::ChecksCompleted { issues: 0 }).unwrap();
        assert_eq!(effects, vec![Effect::RunVerify]);
        assert!(!flow.exam_mode_active());
    }

    #[test]
    fn happy_path_ends_in_ready_then_quit() {
        let mut flow = flow_in(&[
            launched(false),
            Event::PrimaryPressed,
            Event::ChecksCompleted { issues: 3 },
            Event::FixCompleted,
            Event::VerifyCompleted { clean: true },
        ]);
        assert_eq!(flow.phase(), Phase::Ready);
        assert_eq!(
            flow.handle(Event::PrimaryPressed).unwrap(),
            vec![Effect::Quit]
        );
    }

    #[test]
    fn dirty_verify_retries_fix_without_persisting_twice() {
        let mut flow = flow_in(&[
            launched(false),
            Event::PrimaryPressed,
            Event::ChecksCompleted { issues: 3 },
            Event::FixCompleted,
        ]);
        let effects = flow
            .handle(Event::VerifyCompleted { clean: false })
            .unwrap();
        assert_eq!(effects, vec![Effect::RunFix]);
        assert_eq!(flow.phase(), Phase::Fixing);
    }

    #[test]
    fn repeated_dirty_verify_fails_with_fix_retry() {
        let mut flow = flow_in(&[
            launched(false),
            Event::PrimaryPressed,
            Event::ChecksCompleted { issues: 3 },
            Event::FixCompleted,
            Event::VerifyCompleted { clean: false },
            Event::FixCompleted,
        ]);
        let effects = flow
            .handle(Event::VerifyCompleted { clean: false })
            .unwrap();
        assert!(effects.is_empty());
        assert_eq!(flow.phase(), Phase::Failed);
        assert!(flow.failure().is_some());
        // Exam mode stays on: whatever was changed must still be restorable.
        assert!(flow.exam_mode_active());
        assert_eq!(
            flow.handle(Event::PrimaryPressed).unwrap(),
            vec![Effect::RunFix]
        );
    }

    #[test]
    fn fix_failure_keeps_exam_mode_and_retries_fix() {
        let mut flow = flow_in(&[
            launched(false),
            Event::PrimaryPressed,
            Event::ChecksCompleted { issues: 2 },
        ]);
        flow.handle(Event::FixFailed {
            reason: "declined".into(),
        })
        .unwrap();
        assert_eq!(flow.phase(), Phase::Failed);
        assert_eq!(flow.failure(), Some("declined"));
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
            reason: "service missing".into(),
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
    }

    #[test]
    fn busy_phases() {
        assert!(Phase::Checking.is_busy());
        assert!(Phase::Restoring.is_busy());
        assert!(!Phase::Ready.is_busy());
        assert!(!Phase::ExamMode.is_busy());
    }
}
