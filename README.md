# ExamSafe

One click to make your PC exam-safe, and one click to put everything back afterwards.

**Status:** prototype. The UI, tray, exam flow and admin helper are real; the checks and fixes
are simulated until the detection engine is built.

- Product design: [docs/IDEA.md](docs/IDEA.md)
- Code architecture and testing: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)

## Build and run (Windows)

Requires Rust (stable, MSVC toolchain).

Portable exe (recommended):

```bash
pwsh ./tools/build-portable.ps1
```

Then run `dist\ExamSafe.exe` - copy it anywhere, no installer needed.

For development: `cargo run -p examsafe-app`.

## Quality gate

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pwsh ./tools/check-architecture.ps1
```

© 2026 Mikkel Blom. All rights reserved.
