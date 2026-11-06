# AviBackToBlack/CodexBar fork notes

This repository is a personal development fork of [steipete/CodexBar](https://github.com/steipete/CodexBar).

It intentionally contains two different downstream lines that must not be merged into each other.

## Upstreams

- Canonical macOS/Swift project: `steipete/CodexBar`
- Windows-native port used for dogfooding: `nesszer/Win-CodexBar`

## Branches

### `main`

Tracks the canonical `steipete/CodexBar` line and hosts fork-only documentation plus manual build entry points.

### `personal/litellm-v0.83.0` (current)

Personal macOS compatibility patch based directly on the upstream `v0.83.0` release. The provider is implemented as a TypeScript/JavaScript plugin in this version, and the patch preserves the virtual **Key budget**, followed by Personal and Team budgets when present.

### `personal/litellm-work-key` (legacy backup)

The original September 2026 macOS/Swift compatibility patch for a non-standard work LiteLLM virtual-key budget configuration.

This branch is for private-use behavior and is not intended to define general LiteLLM semantics or require Windows parity.

### `windows/devin`

Windows dogfood line based on `nesszer/Win-CodexBar`, currently carrying the custom Devin billing/quota work.

The current Devin implementation is a working development version, not yet the final upstream submission. Functional Devin improvements are intentionally out of scope for repository cleanup/build work.

**Do not merge `windows/devin` into `main`.** The branches represent different implementations and project trees.

## Getting a macOS `.dmg`

1. Open **Actions**.
2. Select **Build macOS DMG**.
3. Choose **Run workflow**.
4. Enter a branch, tag, or commit SHA. Default: `personal/litellm-v0.83.0`.
5. Download the `CodexBar-macOS-<sha>` artifact when the run completes.

The artifact contains:

- `CodexBar-macOS-<sha>.dmg`
- `CodexBar-macOS-<sha>.dmg.sha256`

The app is built universal (`arm64` + `x86_64`), ad-hoc signed, and not notarized.

## Getting a Windows `.exe`

1. Open **Actions**.
2. Select **Build Windows EXE**.
3. Choose **Run workflow**.
4. Enter a branch, tag, or commit SHA. Default: `windows/devin`.
5. Download the `CodexBar-Windows-<sha>` artifact when the run completes.

The artifact contains:

- `codexbar.exe`
- `codexbar.exe.sha256`

The build happens entirely on a GitHub-hosted Windows runner. No local Rust, Cargo, Node, or pnpm toolchain is required on the workstation.

## Updating the macOS line

Use a new versioned branch from the desired upstream release tag; port the personal LiteLLM key-budget behavior into the version's actual provider implementation, then validate it with **Build macOS DMG**. Keep previous working branches as recoverable backups. The `main` branch is the fork's build entry point, not the macOS application source selected by the workflow.

## Updating the Windows line

Refresh `windows/devin` from the desired `nesszer/Win-CodexBar` release/main revision, preserve the downstream Devin patch set, then validate it with **Build Windows EXE**.

## Release workflows inherited from upstreams

The Windows tree contains upstream release/SignPath/CircleCI-related files. They belong to `nesszer/Win-CodexBar` release infrastructure and are not the build entry points for this fork.

For this fork, use only the two manual workflows exposed from `main` unless intentionally working on release infrastructure.
