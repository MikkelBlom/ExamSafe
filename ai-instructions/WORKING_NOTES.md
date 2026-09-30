# Working notes

## Open decisions (need Mikkel)
1. **Tech stack:** Tauri 2 (recommended: easiest premium UI, Rust core, tray built in, RAM only
   while window open) vs Slint (lower RAM, harder to make premium). Proposal: a 1-day spike
   building the same window + tray in both, measuring RAM, then decide.
2. **Automatic restore trigger:** ExamSafe must not run during the exam, so it cannot watch for the
   exam ending. Options: (a) manual "Restore" button + reminder on next launch (safest);
   (b) one-shot scheduled restore at exam end time + buffer from the profile (risk: exam overruns).
   Recommendation: (a), with (b) as opt-in.
3. **How far "fix" goes by default:** also disable VPN/virtual network adapters and stop WSL/VMs
   (ExamMonitor logs adapters and checks for VMs), or only suggest it?
4. **Prototype:** delete `prototype/powershell/` or keep as reference? Its rule catalog is
   useful seed data for the known-app catalog.
5. **GitHub repo:** no remote exists yet. Create private `MikkelBlom/StudentExamSafety`?

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

## Architecture decisions
- UI unprivileged; admin changes via an elevated helper executing a journaled plan. (Least
  privilege, and one UAC prompt per fix.)
- Catalog / traits / profiles are data files, not code — updatable without releases.
