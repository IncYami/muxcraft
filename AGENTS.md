# Repository Guidelines

## Project Structure & Module Organization

Muxcraft is a Rust CLI for batch remuxing from TOML. `src/main.rs` handles CLI arguments; `src/config.rs` parses and resolves configuration; `src/pipeline.rs` orchestrates media tools. Focused modules handle cuts, chapters, subtitles, PGS, fonts, TMDB metadata, and terminal output. Integration tests live in `tests/core.rs` and `tests/media.rs`. `docs/configuration.md` is the configuration reference; `docs/configs/` contains generic TOML examples. Keep flowchart sources (`.mmd` and `.dot`) and their rendered `.svg` files in sync. Nix packaging is in `default.nix`, `package.nix`, and `flake.nix`.

## Build, Test, and Development Commands

- `nix-shell` opens a development environment with Rust, FFmpeg, FLAC, MKVToolNix, and cargo-audit.
- `cargo build --release --locked` builds the binary using the committed lockfile.
- `cargo test --locked` runs unit and integration tests. Media tests need the external tools on `PATH`.
- `cargo fmt --check` checks Rust formatting; run `cargo fmt` before committing.
- `cargo clippy --all-targets` checks Rust code for common mistakes.
- `cargo audit` checks locked dependencies against RustSec advisories.
- `cargo run -- docs/configs/simple-batch.toml --mode plan --no-check` inspects a sample configuration without media files.

## Coding Style & Configuration Conventions

Use Rust 2024 style and rustfmt defaults (four-space indentation). Prefer small functions with explicit errors via `anyhow`; preserve the existing module boundaries. Use `snake_case` for Rust functions and fields and descriptive test names. Keep code, documentation, logs, and TOML examples in en-US. In TOML, prefer single-quoted literal strings and bare numeric episode keys (`01`); quote dotted track IDs inside `[mux]` (`'AudioTrack.01'`). Document any configuration change in `docs/configuration.md` and update an example when useful.

## Testing Guidelines

Add focused unit tests for parsing and transformations; use `tests/core.rs` for configuration and CLI behavior and `tests/media.rs` for real media-tool flows. Test meaningful failure cases and audit output when changing cut or mux behavior. There is no numeric coverage target. Run the full test suite before opening a pull request.

## Commits & Pull Requests

Recent commits use short, imperative subjects, such as `Verify cut media before final mux`. Keep each commit focused. Pull requests should explain the behavior change, list commands run, and link an issue when applicable. Include a TOML example for new configuration syntax and update affected flowcharts. A push to `main` triggers cross-platform builds and a dated GitHub release, so verify the CI result after merging.
