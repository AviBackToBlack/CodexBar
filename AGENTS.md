# Repository Guidelines

## Project Overview

- Windows desktop tray app for AI-provider usage and limits (Win-CodexBar port of CodexBar).
- Default product surface: Tauri 2 desktop shell in `apps/desktop-tauri`, not the CLI.
- Shared domain/backend and CLI live in the `rust/` crate `codexbar`.
- Material under `docs/` that describes the upstream macOS/Swift project is historical unless the task is explicitly about upstream parity.
- When repo docs conflict, trust active sources: `apps/desktop-tauri` plus `rust/src`.

## Architecture & Data Flow

- Cargo workspace (root `Cargo.toml`): members `rust`, `apps/desktop-tauri/src-tauri`; **default-member** is the Tauri crate.
- Path dependency: `codexbar-desktop-tauri` → `codexbar = { path = "../../../rust" }`.
- Frontend: React 18 + Vite in `apps/desktop-tauri/src/`. Typed invoke bridge in `src/lib/tauri.ts`; DTOs in `src/types/bridge.ts`.
- Surfaces: the hidden `main` webview routes by window label / surface mode — TrayPanel, PopOut, Settings, FloatBar. Settings, float bar, and flyout use detached windows where needed.
- **Provider refresh**: `codexbar::core::instantiate_provider` (`rust/src/core/provider_factory.rs`) → `Provider::fetch_usage` → shell `commands/providers.rs` (semaphore + timeout) → `AppState.provider_cache` → events → React `useProviders`.
- **Settings**: `%config%/CodexBar/settings.json` via `Settings::load` / `save` and `secure_file` (DPAPI-capable on Windows). Frontend `updateSettings` patch → save → `codexbar:settings-updated` / float-bar config events.
- **Tray**: `tray_bridge` + `tray_menu`. Icon pixels from shared `codexbar::tray::{render_bar_icon_rgba, render_percent_icon_rgba}`.
- **Float bar**: `floatbar/` owns the auxiliary always-on-top window. The builder must pin `.theme(Some(tauri::Theme::Dark))` — WebView2 resolves `prefers-color-scheme` on a shared process profile; an unpinned window flips other webviews under theme `auto`.
- **Proof harness**: env `CODEXBAR_PROOF_MODE` (e.g. `settings:menu`) opens a target surface and suppresses blur-dismiss for automation / CUA capture.

## Key Directories

- `apps/desktop-tauri/src/` — React UI (surfaces, hooks, i18n, bridge types)
- `apps/desktop-tauri/src-tauri/src/` — Tauri shell (`main`, tray, floatbar, shell windows, commands, proof_harness)
- `rust/src/core/` — `ProviderId`, `Provider` trait, `instantiate_provider`, fetch context
- `rust/src/providers/` — one module per provider (fetch/parse/auth)
- `rust/src/settings/` — settings model and load/save
- `rust/src/browser/` — Windows browser detection + cookie extraction
- `rust/src/tray/` — shared tray-icon renderer
- `rust/src/cli/` — CLI subcommands (`codexbar` binary)
- `scripts/` — `dev.ps1`, `local-check.ps1`, release and smoke scripts
- `docs/` — BUILDING, WINDOWS_PROOF, COOKIES, ADRs
- `.github/workflows/` — `pr-check.yml` (hosted gate), `interaction-guard.yml`

## Development Commands

```text
# Local CI slice (mirrors hosted PR check)
.\scripts\local-check.ps1

# Rust backend / CLI
cargo test --manifest-path rust/Cargo.toml
cargo clippy --manifest-path rust/Cargo.toml --all-targets -- -D warnings
cargo build -p codexbar
cargo run -p codexbar -- --help

# Tauri shell crate
cargo test --manifest-path apps/desktop-tauri/src-tauri/Cargo.toml
cargo clippy --manifest-path apps/desktop-tauri/src-tauri/Cargo.toml --all-targets -- -D warnings

# Frontend (cwd apps/desktop-tauri) — use pnpm, not npm
pnpm install
pnpm test
pnpm run build
pnpm run tauri:dev
pnpm run tauri:build:debug
pnpm run tauri:build

# Dev launch helpers (repo root)
.\scripts\dev.ps1
.\scripts\dev.ps1 -SkipBuild
./dev.sh
```

- Raw `cargo build --release` on the Tauri crate can still embed the **dev URL**. Prefer `pnpm run tauri:build` / `tauri:build:debug` or `scripts/dev.ps1`.
- Binaries: `codexbar.exe` (CLI), `codexbar-desktop-tauri.exe` (desktop).
- Default desktop work runs from the repo root (default-member). Use `cd rust` only for CLI/backend-only focus.
- There is no active root `Scripts/` (capital S) pipeline — use `scripts/`.
- Format before handoff when Rust changed: `cargo fmt --all`. Clippy both manifests with `-D warnings` (or explain skips).

## Code Conventions & Common Patterns

- Prefer small, typed structs/enums and focused modules; keep changes local.
- Provider-specific logic stays inside `rust/src/providers/<name>/` (or that module). Do not add cross-provider branching in shared paths.
- **New provider**: (1) `ProviderId` variant + metadata methods (`cli_name`, `display_name`, …), (2) provider module implementing `Provider`, (3) match arm in `core/provider_factory.rs::instantiate`. The factory is exhaustive — missing arms fail to compile. Never duplicate factories in the shell or CLI.
- Errors: `thiserror` (`ProviderError`) and `anyhow` where already used; keep user-facing messages friendly.
- Logging: `tracing` only. Never log secrets, cookies, tokens, or raw API keys.
- Frontend tests are co-located `*.test.ts` / `*.test.tsx`. Bridge types in `types/bridge.ts` must stay aligned with Rust command payloads (including settings tab ids).
- **Settings tab ids** (case-sensitive; backend whitelist in `surface_target.rs` must mirror frontend `SettingsTabId` / `TAB_META`): `general`, `providers`, `notifications`, `menuBar`, `menu`, `usageSpend`, `advanced`, `about`. Unknown ids fall back to General in the UI. Old ids `display` / `apiKeys` / `cookies` are not valid settings tabs.
- Cookie import UX uses **explicit browser selection** in Preferences — do not assume Chrome-only.
- Claude CLI output is user-configurable; do not treat a customizable status line as the usage source of truth.
- Keep provider data siloed: never show identity / plan / email from provider A in provider B UI.
- Secrets (manual cookies, API keys, token accounts): use existing redaction, `secure_file`, and keyring helpers.
- Do not add dependencies or tooling without confirmation.
- Do not open issues or PRs against upstream `steipete/CodexBar` unless the user explicitly asks. This repo is Win-CodexBar only.

## Important Files

- `apps/desktop-tauri/src-tauri/src/main.rs` — shell entry, command registration, setup
- `apps/desktop-tauri/src/App.tsx` — surface routing by window label
- `apps/desktop-tauri/src/lib/tauri.ts` — frontend invoke bridge
- `apps/desktop-tauri/src/types/bridge.ts` — DTOs + `SettingsTabId`
- `rust/src/core/provider_factory.rs` — sole provider factory
- `rust/src/core/provider.rs` — `ProviderId` + `Provider` trait
- `apps/desktop-tauri/src-tauri/src/commands/providers.rs` — refresh engine
- `apps/desktop-tauri/src-tauri/src/tray_bridge.rs` — tray icon and menu
- `apps/desktop-tauri/src-tauri/src/floatbar/window.rs` — float bar window builder
- `apps/desktop-tauri/src-tauri/src/surface_target.rs` — proof / settings tab whitelist
- `apps/desktop-tauri/src-tauri/tauri.conf.json` — active Tauri config
- `scripts/local-check.ps1` — local CI slice
- `.github/workflows/pr-check.yml` — hosted PR gate
- `CONTEXT.md` — CI budget glossary (Blacksmith pool / `CI_BUDGET_MODE`)

## Runtime/Tooling Preferences

- Package manager: **pnpm@10.18.1** (`packageManager` in `apps/desktop-tauri/package.json` + lockfile). Do not introduce npm or yarn lockfiles.
- Node: CI uses Node 20; no `.nvmrc` in repo — prefer Node 20 locally for parity.
- Rust: edition **2024**, stable toolchain; CI target `x86_64-pc-windows-msvc`. No committed `rust-toolchain.toml` / `rustfmt.toml` / `clippy.toml` — defaults plus CI flags (`clippy -- -D warnings`).
- Tray / DPAPI / browser-cookie behavior: validate on **Windows-native** hosts. WSL/Linux is insufficient for those paths.
- CUA driver (when installed): `%LOCALAPPDATA%\Programs\Cua\cua-driver\bin\cua-driver.exe` for UI automation against a rebuilt binary, optionally with `CODEXBAR_PROOF_MODE`.

## Testing & QA

- Rust: prefer focused `#[cfg(test)]` unit tests near the changed module. Run both manifests after Rust changes.
- Frontend: Vitest 3 + jsdom + Testing Library. From `apps/desktop-tauri`: `pnpm test` (`src/**/*.{test,spec}.{ts,tsx}`).
- **Hosted PR check** (when `vars.CI_BUDGET_MODE != 'off'`): `cargo fmt --check`, clippy both crates with `-D warnings`, cargo test both crates, `pnpm --dir apps/desktop-tauri test`, `pnpm --dir apps/desktop-tauri run build` on Blacksmith Windows. Budget details: `CONTEXT.md`, ADRs under `docs/adr/`.
- **Local mirror**: `.\scripts\local-check.ps1` (default Rust + Tauri + Frontend). Does not run full installer / smoke unless you pass the matching flags.
- UI / tray / float-bar behavior: rebuild the desktop binary and verify on Windows (manual or CUA). Proof example: `CODEXBAR_PROOF_MODE=settings:menu` (float bar section is on the **menu** tab).
- Parser / fetcher changes: add deterministic samples or fixtures where practical.
- No coverage thresholds are configured — do not invent any.

## Commit & PR Guidelines

- Short imperative commit messages (e.g. `Fix Claude CLI parser`, `Improve cookie import errors`).
- Keep commits scoped to one change.
- In PRs / patches include:
  - Summary of behavior changes
  - Commands run (`cargo test`, `pnpm test`, `.\scripts\local-check.ps1`, etc.)
  - Screenshots / GIFs for UI changes (Windows)
  - Linked issue / reference when relevant
- Hosted PR check exists (`.github/workflows/pr-check.yml`); still run and report the local slice. Do not claim there is no CI.
- UI / tray / settings / visual PRs: attach CUA Driver visual proof when possible; otherwise equivalent manual proof and why CUA was skipped (see PR template).
- Before non-trivial merge: thermo-nuclear structure review when the project process requires it.

## Release & Winget Notes

- Treat Winget updates as a normal release step after GitHub release artifacts are stable.
- Winget does not track "latest" GitHub releases; every version needs its own immutable manifest folder in `microsoft/winget-pkgs`, for example `manifests/f/Finesssee/Win-CodexBar/0.23.6/`.
- For routine version bumps, copy the previous approved manifest folder and change only version-specific fields: `PackageVersion`, `InstallerUrl`, `InstallerSha256`, `DisplayName`, `DisplayVersion`, `ReleaseNotes`, and `ReleaseNotesUrl`.
- Keep stable package identity and installer behavior unchanged unless there is a real packaging reason: `PackageIdentifier`, `InstallerType`, `Scope`, `ProductCode`, `Publisher`, package URLs, and silent install behavior.
- Before opening a Winget PR, verify the release installer URL resolves and recompute the SHA-256 from the downloaded asset. On Windows, run `winget validate` when available.
- The first Winget package submission was approved in `microsoft/winget-pkgs#366653`; the v0.23.5 update was approved in `microsoft/winget-pkgs#366794`. Future updates should be faster, but still expect Microsoft validation/review.
