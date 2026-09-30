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
