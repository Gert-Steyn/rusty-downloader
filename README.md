# Rusty Downloader

A download manager for Windows, modeled on JDownloader: segmented HTTP, LinkGrabber, packages,
yt-dlp and gallery-dl integration, and later FTP/SFTP, torrents, hoster plugins and browser capture.

See [PLAN.md](PLAN.md) for the full design and roadmap. Current status: **Phase 1 (MVP)**: 1a HTTP engine and 1b download manager are done; the UI for them is 1c.

## Stack

- **Engine**: Rust (`crates/rdl-core`), tokio, sqlx/SQLite, tracing
- **App shell**: Tauri 2 (`src-tauri`)
- **UI**: SvelteKit + TypeScript (`ui`), Fluent / Windows 11 styling

## Prerequisites

- Rust stable (MSVC toolchain) and Visual Studio C++ Build Tools
- Node.js 24 LTS and pnpm 10
- WebView2 runtime (preinstalled on Windows 11)

## Commands

```bash
pnpm install
```

```bash
pnpm dev
```

`pnpm dev` starts Vite and the Tauri app with hot reload. `pnpm build` produces the NSIS installer under
`target/release/bundle/nsis/`.

```bash
cargo test --workspace
```

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

```bash
pnpm ui:check
```

## Layout

```
crates/rdl-core/   engine, storage (migrations/), paths, logging
src-tauri/         Tauri shell: commands, window, single-instance
ui/                SvelteKit SPA (adapter-static)
```

## Data locations

- Installed: `%APPDATA%\rusty-downloader\` (database, `logs\`, `plugins\`)
- Portable: put an empty `portable.flag` next to the exe; data then goes to `.\data\`

Set `RDL_LOG` (e.g. `debug` or `rdl_core=trace`) to change log verbosity.
