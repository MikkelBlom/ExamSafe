//! The visible progress steps for each phase.

// Labels describe only what really happens. Services, scheduled tasks and startup items get
// their own steps when they are implemented.
pub const CHECK_STEPS: &[&str] = &[
    "Listing running programs",
    "Matching them against the exam rules",
];

pub const FIX_STEPS: &[&str] = &[
    "Saving a restore record",
    "Asking apps to close",
    "Force-closing apps that didn't respond",
];

pub const VERIFY_STEPS: &[&str] = &["Scanning again", "Confirming nothing came back"];

pub const RESTORE_STEPS: &[&str] = &["Reopening your apps", "Clearing the restore record"];

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
