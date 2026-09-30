//! The visible progress steps for each phase.

pub const CHECK_STEPS: &[&str] = &[
    "Apps",
    "Background services",
    "Scheduled tasks",
    "Startup items",
    "Network & adapters",
    "Browsers & editors",
];

pub const FIX_STEPS: &[&str] = &[
    "Saving a restore point",
    "Stopping scheduled tasks",
    "Disabling startup items",
    "Stopping services",
    "Closing apps",
];

pub const VERIFY_STEPS: &[&str] = &["Scanning everything again", "Confirming nothing came back"];

pub const RESTORE_STEPS: &[&str] = &[
    "Re-enabling services",
    "Re-enabling scheduled tasks",
    "Re-enabling startup items",
    "Reopening your apps",
    "Confirming everything is back",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepState {
    Pending = 0,
    Active = 1,
    Done = 2,
}

/// States for `total` steps when `completed` of them are finished: done, one active, rest pending.
pub fn step_states(total: usize, completed: usize) -> Vec<StepState> {
    (0..total)
        .map(|index| match index.cmp(&completed) {
            std::cmp::Ordering::Less => StepState::Done,
            std::cmp::Ordering::Equal => StepState::Active,
            std::cmp::Ordering::Greater => StepState::Pending,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_completed_marks_first_active() {
        assert_eq!(
            step_states(3, 0),
            vec![StepState::Active, StepState::Pending, StepState::Pending]
        );
    }

    #[test]
    fn partly_completed() {
        assert_eq!(
            step_states(3, 2),
            vec![StepState::Done, StepState::Done, StepState::Active]
        );
    }

    #[test]
    fn all_completed_has_no_active_step() {
        assert_eq!(step_states(2, 2), vec![StepState::Done, StepState::Done]);
        assert_eq!(step_states(2, 5), vec![StepState::Done, StepState::Done]);
    }

    #[test]
    fn step_lists_are_not_empty() {
        for steps in [CHECK_STEPS, FIX_STEPS, VERIFY_STEPS, RESTORE_STEPS] {
            assert!(!steps.is_empty());
        }
    }
}
