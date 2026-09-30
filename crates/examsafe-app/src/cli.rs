//! Command-line mode: `ExamSafe.exe --cli <command>`. Drives the exact same flow and service as
//! the window, so it doubles as the end-to-end test harness.
//!
//! Commands:
//!   scan               list catalog apps that are running
//!   make-safe --yes    close them, verify, and record them for restore (--yes = confirm)
//!   restore            reopen what exam mode closed
//!   status             is exam mode on, and what is recorded
//!
//! Exit codes: 0 done, 1 failed, 2 usage error, 3 confirmation needed (add --yes).

use std::collections::VecDeque;
use std::io::Write;

use examsafe_core::apps::AppFinding;
use examsafe_core::flow::{Effect, Event, Flow, Phase};
use examsafe_core::service::{ExamService, run_effect};

const USAGE: &str = "usage: ExamSafe.exe --cli scan | make-safe --yes | restore | status";

/// Writes one line to stdout. A console that went away is not actionable, so write errors are
/// deliberately ignored here (and only here).
fn say(line: &str) {
    let _ = writeln!(std::io::stdout().lock(), "{line}");
}

pub fn run(args: &[String], service: &ExamService) -> u8 {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["scan"] => scan(service),
        ["status"] => status(service),
        ["make-safe"] => {
            let code = scan(service);
            if code == 0 {
                say("Nothing was closed. Add --yes to close the apps listed above.");
                3
            } else {
                code
            }
        }
        ["make-safe", "--yes"] => drive(service, false),
        ["restore"] => drive(service, true),
        _ => {
            say(USAGE);
            2
        }
    }
}

fn scan(service: &ExamService) -> u8 {
    match service.scan() {
        Ok(findings) if findings.is_empty() => {
            say("No apps from the exam list are running.");
            0
        }
        Ok(findings) => {
            for app in &findings {
                say(&format!(
                    "{:<24} {:<22} {} process(es){}",
                    app.name,
                    app.category,
                    app.pids.len(),
                    if app.launch.is_some() {
                        ", will reopen"
                    } else {
                        ""
                    }
                ));
            }
            0
        }
        Err(error) => {
            say(&format!("Scan failed: {error}"));
            1
        }
    }
}

fn status(service: &ExamService) -> u8 {
    match service.load_record() {
        Ok(None) => say("Exam mode: off"),
        Ok(Some(record)) => {
            say(&format!(
                "Exam mode: on ({} app(s) recorded)",
                record.closed_apps.len()
            ));
            for app in &record.closed_apps {
                say(&format!("  {} -> {:?}", app.name, app.launch));
            }
        }
        Err(error) => {
            say(&format!("Could not read the exam-mode record: {error}"));
            return 1;
        }
    }
    0
}

/// Runs the flow to completion. `restore` = start from exam mode and restore; otherwise make
/// safe, answering the confirm step with yes.
fn drive(service: &ExamService, restore: bool) -> u8 {
    let exam_mode_active = match service.load_record() {
        Ok(record) => record.is_some(),
        Err(error) => {
            say(&format!("Could not read the exam-mode record: {error}"));
            return 1;
        }
    };
    if restore && !exam_mode_active {
        say("Exam mode is off - nothing to restore.");
        return 0;
    }
    if !restore && exam_mode_active {
        say("Exam mode is already on. Run 'restore' first.");
        return 1;
    }

    let mut flow = Flow::new();
    let mut findings: Vec<AppFinding> = Vec::new();
    let mut queue = VecDeque::from([Event::Launched { exam_mode_active }, Event::PrimaryPressed]);
    while let Some(event) = queue.pop_front() {
        let effects = match flow.handle(event) {
            Ok(effects) => effects,
            Err(error) => {
                say(&format!("Internal error: {error}"));
                return 1;
            }
        };
        say(&format!("- {:?}", flow.phase()));
        for effect in effects {
            if effect == Effect::Quit {
                continue;
            }
            match run_effect(service, effect, &[], &findings, unix_now()) {
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
            let names: Vec<&str> = findings.iter().map(|app| app.name.as_str()).collect();
            say(&format!("  closing: {}", names.join(", ")));
            queue.push_back(Event::PrimaryPressed);
        }
    }

    match flow.phase() {
        Phase::Ready => {
            say(&format!(
                "Done: {} app(s) closed and verified. Run 'restore' after the exam.",
                flow.issues()
            ));
            0
        }
        Phase::Restored => {
            say("Done: apps reopened, exam mode off.");
            0
        }
        phase => {
            say(&format!(
                "Stopped in {phase:?}: {}",
                flow.failure().unwrap_or("unknown reason")
            ));
            1
        }
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}
