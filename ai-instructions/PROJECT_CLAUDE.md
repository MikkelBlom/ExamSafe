# CLAUDE.md — ExamSafe (StudentExamSafety)

**Read the global CLAUDE.md first** (same folder), then this.

## What this is
A desktop app that makes a multi-purpose PC (study + job + AI tooling) provably exam-safe for a
specific exam, with one button, and restores everything afterwards. Windows first; macOS/Linux
later. Owner/baseline user: Mikkel (SDU student — Digital Exam + ExamMonitor). Full design:
`docs/IDEA.md`.

## Current phase
**Apps stage done (2026-10-03):** apps are really detected, closed, verified and reopened.
Next stage: services, scheduled tasks and startup items via the elevated helper — confirm scope
with Mikkel before starting each new stage. Never close Mikkel's open apps or trigger shutdowns
while testing; use harmless targets (tools/e2e-charmap.ps1). Stack is decided: **Rust + Slint** with the software
renderer (native, no Chromium/WebView wrapper — do not propose Tauri or Electron again).
Code architecture and quality gate: `docs/ARCHITECTURE.md` — keep it strict: layer rules are
enforced by `tools/check-architecture.ps1`, clippy warnings are errors, domain logic lives in
`examsafe-core` and is unit tested.

## Never violate
1. Never "fire and forget" — every fix is re-verified; only a clean re-scan may say "exam safe".
2. Every change is journaled before it happens, is reversible, and survives crash/reboot.
3. Ask before closing anything that may hold unsaved work.
4. ExamSafe must never run during the exam, never touch the exam software, never hide anything
   from it. Compliance tool, not evasion tool.
5. Every verdict is explainable (which rule/trait caused it).
6. Never disable antivirus/security software.
7. Keep the core platform-agnostic (platform code behind an adapter).

## UI bar
Clean, modern, premium, Apple-like, smooth animations, no generic "AI app" look. Simple view:
one click, minimal decisions. Advanced settings hidden by default, fully transparent and
customizable. Tray icon control. Flow: make safe → "ready" → Close ExamSafe → next launch = Restore.
