# Features

| Feature | Status | Description | Date | Depends on | Blocked by |
|---|---|---|---|---|---|
| Idea & design document | Implemented | `docs/IDEA.md` | 2026-09-30 | – | – |
| Cargo workspace + layer rules | Implemented | core / platform / helper / app; `tools/check-architecture.ps1` | 2026-09-30 | – | – |
| CI | Implemented | GitHub Actions: fmt, clippy -D warnings, architecture, tests (windows-latest) | 2026-09-30 | – | – |
| Exam flow state machine | Implemented | `core::flow` — make safe → ready → close → restore, retries, failure paths | 2026-09-30 | – | – |
| Exam-mode persistence | Implemented | `%LOCALAPPDATA%\ExamSafe\exam-mode.json`, written before any change | 2026-09-30 | – | – |
| Elevated helper handshake | Implemented | UAC → `examsafe-helper` ping → response; `EXAMSAFE_NO_ELEVATE=1` for dev | 2026-09-30 | – | – |
| Slint UI (simple + advanced) | Prototype | Custom design system, animated states, advanced sheet with per-item toggles (sample data) | 2026-09-30 | – | – |
| Tray icon | Prototype | Status colour + tooltip, left-click opens, right-click menu (Open/Quit) | 2026-09-30 | – | – |
| Performance measurement | Implemented | `tools/measure.ps1` — 24.5 MB working set, ~0% idle CPU, 125 ms startup | 2026-09-30 | – | – |
| Read-only detection engine | Planned | Inventory, catalog, traits; replaces simulated checks | – | Prototype | Go-ahead |
| Exam profiles + ask & remember | Planned | Presets, per-profile decisions | – | Engine | – |
| Real fix + journal + restore | Planned | Helper executes journaled plan; verify loop | – | Engine | – |
| Browser/IDE/OS AI controls | Planned | Policies/settings for extensions and built-in AI | – | Fix | – |
| Distribution & updates | Planned | Signed installer, updater, catalog channel — IDEA §11b | – | Fix + restore | – |
| macOS / Linux | Planned | Platform adapters | – | Core stable | – |
