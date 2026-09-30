# Features

| Feature | Status | Description | Date | Depends on | Blocked by |
|---|---|---|---|---|---|
| Idea & design document | Implemented | `docs/IDEA.md` | 2026-09-30 | – | – |
| Cargo workspace + layer rules | Implemented | core / platform / helper / app; `tools/check-architecture.ps1` | 2026-09-30 | – | – |
| CI | Implemented (blocked) | GitHub Actions: fmt, clippy -D warnings, architecture, tests — blocked by GitHub billing | 2026-09-30 | – | GitHub billing |
| Exam flow state machine | Implemented | make safe → confirm → close → verify → ready → close app → restore; retries; failure paths | 2026-09-30 | – | – |
| App catalog | Implemented | `catalog/apps.json`, 36 apps (AI, remote, chat, sync, device link, launchers); `EXAMSAFE_CATALOG` override | 2026-09-30 | – | – |
| Close apps | Implemented | Confirm list → WM_CLOSE → 4 s grace → force close; protected own process chain | 2026-09-30 | – | – |
| Verify | Implemented | Re-scan; re-close once; then fail naming what keeps coming back | 2026-09-30 | – | – |
| Journal + restore | Implemented | Journal v2 written before closing; restore reopens via path or Store app id as normal user | 2026-09-30 | – | – |
| Command line | Implemented | `ExamSafe.exe --cli scan / make-safe --yes / restore / status` | 2026-09-30 | – | – |
| Portable single exe | Implemented | `tools/build-portable.ps1` → `dist/ExamSafe.exe`; helper mode built in | 2026-09-30 | – | – |
| Elevated helper | Implemented (unused) | UAC → `--helper` ping; will run service/task/startup changes | 2026-09-30 | – | – |
| Slint UI (simple + advanced) | Implemented | Confirm list, honest texts + scope note, Advanced = real running apps with toggles | 2026-09-30 | – | – |
| Tray icon | Implemented | Status colour + tooltip, left-click opens, right-click menu | 2026-09-30 | – | – |
| Services / scheduled tasks / startup items | Planned | Via elevated helper, journaled | – | Helper | Go-ahead |
| Browser/IDE/OS AI controls | Planned | Extension policy blocklist, VS Code settings, Windows AI policies | – | Helper | – |
| Network adapters / VMs | Planned | Disable VPN/virtual adapters, stop WSL | – | Helper | – |
| Exam profiles + remember decisions | Planned | Presets, per-profile allow/block remembered | – | – | – |
| Distribution & updates | Planned | Signed installer, updater, catalog channel — IDEA §11b | – | – | – |
| macOS / Linux | Planned | Platform adapters | – | Core stable | – |
