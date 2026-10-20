//! Headless UI tests: the window's texts, button labels and actions per phase, using Slint's
//! testing backend (no screen, no clicks). The backend can only be initialised once per process,
//! so everything runs in one sequential test.

use std::cell::Cell;
use std::rc::Rc;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};

use slint::{ModelRc, VecModel};

use crate::{AppWindow, Phase, PlanItem};

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
        "1 item on the exam list is on. One click turns it off, checks it stays off, and puts it back afterwards."
    ));
    ui.set_running_count(3);
    assert!(has(
        &ui,
        "3 items on the exam list are on. One click turns them off, checks they stay off, and puts them back afterwards."
    ));
    ui.set_running_count(0);
    assert!(has(&ui, "Nothing on the exam list is on right now."));

    // Busy: the button says so and cannot be pressed.
    ui.set_phase(Phase::Checking);
    let working = button(&ui, "Working…");
    assert_eq!(working.accessible_enabled(), Some(false));
    working.invoke_accessible_default_action();
    assert_eq!(primary.get(), 1, "a disabled button must not act");

    // Confirm: asks with the right count, offers Cancel as the secondary action.
    ui.set_phase(Phase::Confirm);
    ui.set_issue_count(1);
    assert!(has(&ui, "Turn off 1 item?"));
    ui.set_issue_count(4);
    assert!(has(&ui, "Turn off 4 items?"));
    button(&ui, "Turn them off");
    press(&ui, "Cancel");
    assert_eq!(secondary.get(), 1);

    // Ready: honest about whether anything was closed.
    ui.set_phase(Phase::Ready);
    assert!(has(&ui, "Everything is off"));
    ui.set_issue_count(0);
    assert!(has(&ui, "Nothing to turn off"));
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

    // Advanced rows: section titles once, state and planned action shown, a toggle only for
    // items ExamSafe acts on, and toggling reports the row.
    let toggled = Rc::new(Cell::new((-1, true)));
    {
        let toggled = Rc::clone(&toggled);
        ui.on_plan_item_toggled(move |index, value| toggled.set((index, value)));
    }
    let row = |section: &str, name: &str, state: &str, action: &str, toggleable: bool| PlanItem {
        section: section.into(),
        name: name.into(),
        detail: "detail".into(),
        state: state.into(),
        action: action.into(),
        toggleable,
        enabled: true,
    };
    ui.set_plan_items(ModelRc::new(VecModel::from(vec![
        row(
            "Services",
            "TeamViewer",
            "Running · Automatic",
            "Stop and set to Disabled",
            true,
        ),
        row(
            "Network adapters",
            "bdvpn",
            "Connected",
            "Shown only",
            false,
        ),
    ])));
    assert!(has(&ui, "Services"));
    assert!(has(&ui, "Running · Automatic"));
    assert!(has(&ui, "Stop and set to Disabled"));
    assert!(has(&ui, "Network adapters"));
    assert!(
        !has(&ui, "Turn off bdvpn for the exam"),
        "shown-only rows have no toggle"
    );
    let toggle = ElementHandle::find_by_accessible_label(&ui, "Turn off TeamViewer for the exam")
        .next()
        .expect("toggle for an actionable row");
    assert_eq!(toggle.accessible_role(), Some(AccessibleRole::Checkbox));
    toggle.invoke_accessible_default_action();
    assert_eq!(toggled.get(), (0, false));

    press(&ui, "Back");
    assert!(!ui.get_advanced_open());
}
