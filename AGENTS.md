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

## Technical Writing

Use the current [ASD-STE100 Simplified Technical English standard](https://www.asd-ste100.org/) for new or changed English text.
Apply it to README files, configuration comments, help text, error messages, commit messages, and release or pull request text.
Use approved words with their approved meanings. Use project terms consistently.
Explain terms such as Rust, Matroska, and MKVToolNix when a reader might not know them.
Give one topic in each sentence. Use active voice.
Use no more than 20 words in each procedure sentence. Use no more than 25 words in each description sentence.
Give one action in each instruction. Start each instruction with a command verb.
Check the official rules and dictionary before you claim ASD-STE100 compliance.
Keep required code syntax, exact command names, quoted output, and translated text intact.

## Rust Maintenance and Security

For each Rust application or dependency change, check the latest stable Rust release and each direct dependency's latest stable release.
Update the Rust toolchain, direct dependencies, and `Cargo.lock` to those releases.
Change application code when an updated API requires it. Keep the build and Nix packages compatible.
If an update is blocked, document the specific incompatibility. Check it again during the next Rust change.
Keep the architecture simple. Use the standard library or current dependencies when they can do the work.
Remove unused crates. Add the smallest set of dependencies that the work needs.
Check maintenance and provenance before you add a crate.
Prefer crates from an official vendor, an established organization, or active professional contributors.
Run `cargo audit` after dependency changes and before you complete Rust application changes.
Resolve actionable security advisories before you commit. Document each advisory that you cannot resolve.

## Testing Guidelines

Add focused unit tests for parsing and transformations; use `tests/core.rs` for configuration and CLI behavior and `tests/media.rs` for real media-tool flows. Test meaningful failure cases and CLI output when changing cut or mux behavior. There is no numeric coverage target. Run the full test suite before opening a pull request.

## Commits & Pull Requests

Use short command sentences for commit subjects, such as `Verify cut media before final mux`.
Keep each commit focused. Explain behavior changes and list test commands in pull requests. Link an issue when applicable.
Include a TOML example for new configuration syntax. Update affected flowcharts.
Use semantic versions with three numbers (`x.x.x`). Each completed project change needs a commit and a push to `origin/main`.
Increase the package version in `Cargo.toml` when a commit changes Rust application code.
Update `Cargo.lock` in the same commit. Increase the patch number for fixes and internal changes.
Increase the minor number for new functionality while the project version starts with `0`.
Increase the major number for incompatible changes after version `1.0.0`.
Documentation-only, instruction-only, and Nix packaging-only changes do not need a new application version.
Include only work for the task. Leave ignored personal examples out of commits.
Check CI after the push. Report the commit, push, and CI result.
