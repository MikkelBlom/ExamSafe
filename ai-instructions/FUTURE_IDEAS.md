# Future ideas

## Detection
- (High) Community-maintained catalog + presets per university (SDU, AU, KU, DTU, ITU…).
- (Medium) Detect AI features inside Office (Copilot in Word/Excel) and Google Workspace settings.
- (Medium) "Exam calendar" import (itslearning / Digital Exam) to pre-fill exam time windows so
  scheduled tasks due during the exam are flagged automatically.
- (Low) Hash-based identity for unsigned binaries.

## Restore
- (High) **Timed automatic restore.** You usually know when the exam ends: let the profile hold an
  end time + buffer, and restore automatically after it. Constraint: must never fire while the exam
  is still running (overtime, extra time) and must not itself show up as something running during
  the exam — e.g. a one-shot scheduled task created at "Close ExamSafe", disabled by default, with
  a generous buffer. v1 is manual restore only.

## Quality
- (Medium) Headless test of the *controller* (UI + flow + fake OS together). The per-phase UI test
  exists; driving the controller needs its worker threads and timers under the testing backend's
  mock time.
- (Medium) Measure CPU during busy animations; add `tools/measure.ps1` run to a manual
  pre-release checklist.
- (Low) Coverage report (cargo-llvm-cov) in CI.

## UX
- (Medium) "Practice run" mode: full check+fix+restore days before the exam, so exam morning is boring.
- (Medium) Shareable read-only report ("what was checked, when") for peace of mind.
- (Low) Onboarding tour for non-technical students.

## Platform
- (Medium) macOS adapter (launchd agents, login items, TCC permissions).
- (Low) Linux adapter (systemd user units, autostart .desktop files).
