# Sessions

## 2026-09-30 — Claude Code (desktop) — Idea & design
**Summary:** Captured the ExamSafe idea. Read-only survey of Mikkel's PC (processes, services,
tasks, startup, ports, browser/IDE extensions, adapters, sync roots). Researched SDU ExamMonitor,
browser/VS Code AI controls. Wrote `docs/IDEA.md` and set up `ai-instructions/`.
**Issues:** Claude started building a PowerShell tool before the idea was agreed — Mikkel stopped
it. Code parked in `prototype/powershell/` (never run). Lesson: idea first, build only on go-ahead.
**Next:** Mikkel reviews `docs/IDEA.md` and the open decisions in WORKING_NOTES.md; then tech spike.

## 2026-09-30 (later) — Claude Code (desktop) — Design decisions
**Summary:** Mikkel decided: Rust + Slint (no WebView), manual restore flow (ready → Close →
next launch Restore), default = turn off everything relevant, hidden advanced settings with full
per-item control. Timed auto-restore added to FUTURE_IDEAS. PowerShell prototype deleted.
Private GitHub repo created (HTTPS).
**Next:** Slint spike once Mikkel gives the go-ahead.

## 2026-09-30 (evening) — Claude Code (desktop) — Slint prototype
**Summary:** Built the prototype as the real foundation: Cargo workspace (core / platform / helper /
app), strict lints, architecture guard, GitHub Actions CI. Core flow state machine with full unit
tests; exam-mode record persisted before changes; elevated helper handshake (UAC, command-line
request, validated response file); Slint UI with custom design system, animated simple view and
hidden advanced sheet; native tray icon. Checks/fixes are simulated. Measured 24.5 MB / ~0% CPU /
125 ms startup. Recorded licence intent (source-available, all rights reserved).
**Issues:** Software renderer doesn't clip gradients to rounded corners (removed orb highlight).
Slint-generated code fails strict clippy (isolated in `mod ui`). Real UAC path not clicked through
by Claude (needs a human).
**Next:** Mikkel tries the prototype incl. the UAC prompt; then the read-only detection engine.

## 2026-09-30 (late) — Claude Code (desktop) — Portable single exe
**Summary:** Mikkel's `cargo run` failed with "examsafe-helper.exe not found" (cargo run only
builds the app binary). Fixed at the root: helper became a library; the app exe relaunches itself
via UAC as `ExamSafe.exe --helper --request <hex>`, dispatched in `main` before any UI. Added
`tools/build-portable.ps1` → `dist/ExamSafe.exe` (9.3 MB, gitignored). Verified helper mode from
the portable exe (valid ping → response file, bad request → exit 2).
**Issues:** UI click-through automation still flaky (synthetic clicks) — UI path to the helper
verified only via the direct helper request, not by clicking.
**Next:** Mikkel runs dist\ExamSafe.exe and accepts the UAC prompt once.
