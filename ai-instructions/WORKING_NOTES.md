# Working notes

## Decisions made (2026-09-30, by Mikkel)
- **Stack: Rust + Slint.** No Chromium/WebView wrapper (Tauri rejected). Reason: Slint used the
  least resources in Mikkel's own test in another project; "Task Manager philosophy" — native,
  small, instant. Tray via standalone `tray-icon` crate (verify Slint has no tray API of its own).
- **Exam flow:** one button makes the PC exam-safe → app says "Everything is ready" → user presses
  **Close ExamSafe** (app exits fully) → next launch shows only **Restore my PC**. No automatic
  restore in v1; timed auto-restore is in FUTURE_IDEAS.
- **Defaults:** turn off *everything relevant* for the profile, including VPN/virtual adapters and
  running VMs/WSL. A hypervisor that stays present is reported, not changed.
- **UI:** simple view = as few decisions/buttons as possible (one click). Advanced settings are
  hidden unless opened, and give full transparency + per-item control over what is closed,
  terminated, stopped, disabled or blocked, and how.
- **Prototype:** early PowerShell prototype deleted (still in git history, commit 7a186ab).
- **Repo:** private GitHub repo, HTTPS remote.

- **Licence (later):** repo may become public but NOT open source — source-available, all rights
  reserved ("look, don't use"). Decide exact licence text together with code signing. Constraint:
  Slint must then be used under its **royalty-free licence** (not GPLv3, which would force the app
  open); that licence requires Slint attribution (AboutSlint widget or notice) — add before any
  public release.
- **Quality bar:** strict architecture + healthy automated tests are requirements, not nice-to-haves.

## Open decisions
- None blocking. Next stage (read-only detection engine) needs a go-ahead.

## Prototype findings (2026-09-30)
- **Measured** (release build, Mikkel's PC, `tools/measure.ps1`): 24.5 MB working set, 7.4 MB
  private, ~0.02 % CPU idle with window open, 0 % tray-only, 1 process, 5 threads, 125 ms to
  window. Busy-state animation CPU not yet measured.
- Hidden window keeps its memory (window is hidden, not destroyed). Fine at this size.
- **Software renderer gotcha:** gradients are not clipped to `border-radius` (a gradient highlight
  on the orb rendered as a square). Use solid colours on rounded shapes.
- Icons use the **Segoe Fluent Icons** font (Windows 11 only). For macOS/Linux or Windows 10,
  switch to bundled SVG/Path icons — to be verified with the software renderer.
- Slint-generated code triggers our strict clippy lints; it is wrapped in `mod ui` with lints
  allowed. Don't loosen the workspace lints instead.
- Synthetic mouse clicks (SetCursorPos + mouse_event) sometimes land on the previous cursor
  position — use the Slint testing backend for automated UI tests instead.
- Real UAC path (elevated helper) compiled and wired but only exercised in `Direct` mode by
  Claude, since the UAC prompt needs a human. Mikkel should click through it once.

## Research findings
- **ExamMonitor (SDU)** logs running processes (name + description), browser URLs, network
  adapters, screenshots, and has VM detection. Source: SDU pages + 2019 secret.club analysis
  (may be outdated). Implication: ExamSafe must exit before the exam; no mid-exam actions.
- **Browser extensions** are detectable offline: `<User Data>/<Profile>/Extensions/<id>/<ver>/manifest.json`
  (names may be `__MSG_x__` → resolve via `_locales`). Firefox: `extensions.json` in profile.
  Disable reliably via policy `ExtensionInstallBlocklist` (HKLM\Software\Policies\Google\Chrome,
  ...\Microsoft\Edge). Editing Preferences directly is HMAC-protected — don't.
- **Browser AI:** Chrome `GenAiDefaultSettings`=2 / `GeminiSettings`; Edge `HubsSidebarEnabled`=0.
  AI websites can be blocked with `URLBlocklist`.
- **VS Code:** `chat.disableAIFeatures: true` disables built-in AI + Copilot; third-party AI
  extensions (Cline, Claude Code, Continue…) must be disabled separately.
- **Cloud sync trait:** `HKLM\...\Explorer\SyncRootManager` lists registered sync roots — but Google
  Drive did NOT register there on Mikkel's PC (uses its own virtual drive), so traits must combine
  several signals.
- **Startup toggle** = `...\Explorer\StartupApproved\Run|Run32|StartupFolder` binary value; low bit
  of byte 0 set = disabled.
- **Services are the big blind spot** of the manual routine: Claude Cowork, TeamViewer, GlideX,
  SQL Server, Quick Share all run as services regardless of startup toggles.
- **Known-app seed list:** the deleted prototype's `config/default-rules.json` (git history,
  7a186ab) holds ~60 app rules (process/service/task/startup/port footprints) — reuse as the
  starting catalog when building the engine.

- **Slint vs Tauri 2 resources (web research, not yet measured here):** Tauri idle is usually
  quoted at 30–80 MB, but that often counts only the main process; WebView2 spawns several
  msedgewebview2 processes and the real total is higher (tauri issue #5889). Slint with the
  software renderer is about 30–40 MB flat. Slint's GPU/Skia renderer has shown 200–440 MB on
  Windows (slint issue #13470), so **use the software renderer by default**. Tauri's tray is built on
  the same `tray-icon` crate we would use with Slint, so tray support doesn't favour either one.
  Plan: measure both on Mikkel's PC during the spike.

## Architecture decisions
- UI unprivileged; admin changes via an elevated helper executing a journaled plan. (Least
  privilege, and one UAC prompt per fix.)
- Catalog / traits / profiles are data files, not code — updatable without releases.
- Slint UI uses a custom design system, not stock Fluent/Material styling.
