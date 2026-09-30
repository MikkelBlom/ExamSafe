# ExamSafe

One click to make your PC exam-safe, and one click to put everything back afterwards.

**Status:** early version. Closing and reopening **apps** is real and tested end to end.
Services, scheduled tasks, startup items and browser extensions are not handled yet.

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
pwsh ./tools/e2e-charmap.ps1   # real exe vs a real app (after cargo build --release)
```

© 2026 Mikkel Blom. All rights reserved.
