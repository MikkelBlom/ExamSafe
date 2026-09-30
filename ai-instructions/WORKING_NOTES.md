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

## Open decisions
- None blocking. Next step needs a go-ahead: the Slint spike (see docs/IDEA.md §12).

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
