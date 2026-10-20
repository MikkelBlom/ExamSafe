# Features

| Feature | Status | Description | Date | Depends on | Blocked by |
|---|---|---|---|---|---|
| Idea & design document | Implemented | `docs/IDEA.md` | 2026-09-30 | – | – |
| Cargo workspace + layer rules | Implemented | core / platform / helper / app; `tools/check-architecture.ps1` | 2026-09-30 | – | – |
| Local quality gate | Implemented | `tools/check.ps1`: fmt, clippy -D warnings, tests, architecture. Hosted CI removed (2026-10-20, Mikkel's call) | 2026-10-20 | – | – |
| Exam flow state machine | Implemented | make safe → confirm → close → verify → ready → close app → restore; retries; failure paths | 2026-09-30 | – | – |
| Catalog | Implemented | `catalog/apps.json`: 50 entries with processes, services, scheduled tasks, service target (disabled/manual), VPN/virtual adapter words; `EXAMSAFE_CATALOG` override | 2026-10-20 | – | – |
| Close apps | Implemented | Confirm list → WM_CLOSE → 4 s grace → force close; protected own process chain | 2026-09-30 | – | – |
| Verify | Implemented | Re-scan; re-close once; then fail naming what keeps coming back | 2026-09-30 | – | – |
| Journal + restore | Implemented | Journal v3 written before any change; reads v1/v2 | 2026-10-20 | – | – |
| Exact app restore | Implemented | Records root process arguments, folder, had-window; restarts the same way via desktop Explorer (normal user, not ExamSafe's child), hidden if it had no window; Store apps activated with arguments; skips what is already running | 2026-10-20 | – | – |
| Services + scheduled tasks | Implemented (service part not run on real SCM yet) | Journal snapshot (start type incl. delayed, running / enabled) → one helper run (tasks, then services) → re-verify; exact restore + re-verify; security + exam software never touched (core + helper) | 2026-10-20 | Helper | Mikkel runs `tools/e2e-services.ps1` elevated |
| Command line | Implemented | `ExamSafe.exe --cli scan / inventory / make-safe --yes / restore / status` | 2026-10-20 | – | – |
| Portable single exe | Implemented | `tools/build-portable.ps1` → `dist/ExamSafe.exe`; helper mode built in | 2026-09-30 | – | – |
| Elevated helper | Implemented | UAC → `--helper`; `ApplyChanges` for services/tasks; `EXAMSAFE_HELPER=direct` skips UAC for dev/e2e | 2026-10-20 | – | – |
| Slint UI (simple + advanced) | Implemented | Advanced: sections for apps, services, tasks, startup items, listening ports, adapters, each with state + planned action; remembered toggles for actionable items (ports follow their owner) | 2026-10-20 | – | – |
| Tray icon | Implemented | Status colour + tooltip with count; menu Open / Make safe / Restore / Quit (greyed when not applicable); window X hides to tray except in Ready (quits) | 2026-10-20 | – | – |
| Startup items | Shown only | Listed in Advanced / `--cli inventory` with owning app; not changed | – | – | Go-ahead |
| Browser/IDE/OS AI controls | Planned | Extension policy blocklist, VS Code settings, Windows AI policies | – | Helper | – |
| Network adapters | Shown only | VPN/virtual flagged in Advanced; not disabled yet. WSL/Docker/VirtualBox services handled via services | – | Helper | Go-ahead |
| Exam profiles + remember decisions | Planned | Presets, per-profile allow/block remembered | – | – | – |
| Distribution & updates | Planned | Signed installer, updater, catalog channel — IDEA §11b | – | – | – |
| macOS / Linux | Planned | Platform adapters | – | Core stable | – |
