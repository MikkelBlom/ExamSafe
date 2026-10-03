//! Headless UI tests: the window's texts, button labels and actions per phase, using Slint's
//! testing backend (no screen, no clicks). The backend can only be initialised once per process,
//! so everything runs in one sequential test.

use std::cell::Cell;
use std::rc::Rc;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};

use crate::{AppWindow, Phase};

fn find(ui: &AppWindow, label: &str) -> Option<ElementHandle> {
    ElementHandle::find_by_accessible_label(ui, label).next()
}

fn has(ui: &AppWindow, label: &str) -> bool {
    find(ui, label).is_some()
}

fn button(ui: &AppWindow, label: &str) -> ElementHandle {
    ElementHandle::find_by_accessible_label(ui, label)
        .find(|element| element.accessible_role() == Some(AccessibleRole::Button))
        .unwrap_or_else(|| panic!("no button labelled {label:?}"))
}

fn press(ui: &AppWindow, label: &str) {
    button(ui, label).invoke_accessible_default_action();
}

#[test]
fn window_texts_and_actions_follow_the_phase() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = AppWindow::new().unwrap();
    let primary = Rc::new(Cell::new(0));
    let secondary = Rc::new(Cell::new(0));
    {
        let primary = Rc::clone(&primary);
        ui.on_primary_action(move || primary.set(primary.get() + 1));
        let secondary = Rc::clone(&secondary);
        ui.on_secondary_action(move || secondary.set(secondary.get() + 1));
    }

    // Idle, before the startup scan finished: generic text, main button enabled.
    assert!(has(&ui, "Ready when you are"));
    assert_eq!(
        button(&ui, "Make my PC exam-safe").accessible_enabled(),
        Some(true)
    );
    press(&ui, "Make my PC exam-safe");
    assert_eq!(primary.get(), 1);

    // Idle after the scan: the count is stated, singular and plural.
    ui.set_running_count(1);
    assert!(has(
        &ui,
        "1 app on the exam list is running. One click closes it, checks it stays closed, and reopens it afterwards."
    ));
    ui.set_running_count(3);
    assert!(has(
        &ui,
        "3 apps on the exam list are running. One click closes them, checks they stay closed, and reopens them afterwards."
    ));
    ui.set_running_count(0);
    assert!(has(
        &ui,
        "None of the apps on the exam list are running right now."
    ));

    // Busy: the button says so and cannot be pressed.
    ui.set_phase(Phase::Checking);
    let working = button(&ui, "Working…");
    assert_eq!(working.accessible_enabled(), Some(false));
    working.invoke_accessible_default_action();
    assert_eq!(primary.get(), 1, "a disabled button must not act");

    // Confirm: asks with the right count, offers Cancel as the secondary action.
    ui.set_phase(Phase::Confirm);
    ui.set_issue_count(1);
    assert!(has(&ui, "Close 1 app?"));
    ui.set_issue_count(4);
    assert!(has(&ui, "Close 4 apps?"));
    button(&ui, "Close apps");
    press(&ui, "Cancel");
    assert_eq!(secondary.get(), 1);

    // Ready: honest about whether anything was closed.
    ui.set_phase(Phase::Ready);
    assert!(has(&ui, "Apps are closed"));
    ui.set_issue_count(0);
    assert!(has(&ui, "Nothing to close"));
    button(&ui, "Close ExamSafe");

    // Failed: shows the reason; offers restore only when something was changed.
    ui.set_phase(Phase::Failed);
    ui.set_failure_reason("Couldn't close Slack: access denied.".into());
    assert!(has(&ui, "Couldn't close Slack: access denied."));
    button(&ui, "Try again");
    assert!(!has(&ui, "Restore my PC instead"));
    ui.set_exam_mode_active(true);
    press(&ui, "Restore my PC instead");
    assert_eq!(secondary.get(), 2);

    // Exam mode: the only main action is restore.
    ui.set_phase(Phase::ExamMode);
    assert!(has(&ui, "Exam mode is on"));
    press(&ui, "Restore my PC");
    assert_eq!(primary.get(), 2);

    // Icon glyphs must not be read out by screen readers.
    ui.set_phase(Phase::Ready);
    assert!(
        !has(&ui, "\u{e73e}"),
        "check-mark glyph is exposed to accessibility"
    );

    // Advanced opens from the link and closes with Back.
    ui.set_phase(Phase::Idle);
    assert!(!ui.get_advanced_open());
    press(&ui, "Advanced");
    assert!(ui.get_advanced_open());
    press(&ui, "Back");
    assert!(!ui.get_advanced_open());
}
