# win-pydl — Project Plan

A full-featured download manager for Windows, modeled on **JDownloader**. It has a Rust engine, a Tauri 2 + SvelteKit UI, and built-in **yt-dlp** and **gallery-dl** support.

> Working name: `win-pydl` (from the folder name). The app is now Rust rather than Python, so a rename is still open (see Open Decisions).

---

## 1. Goals & Constraints

| Item | Decision |
|---|---|
| Purpose | Personal daily driver, polished enough to replace IDM/JDownloader |
| Benchmark | JDownloader (LinkGrabber, packages, hoster plugins) |
| Platform | Windows 10/11 x64 (WebView2 runtime) |
| Language | Rust (stable, 2024 edition) |
| UI | Tauri 2 + SvelteKit + TypeScript, modern Windows 11 (Fluent) look |
| Distribution | NSIS installer + portable zip, signed auto-updates from GitHub Releases |
| Source | Private GitHub repo, MIT license |
| External tools | Bundled `yt-dlp.exe`, `gallery-dl.exe`, `ffmpeg.exe`, `7z.exe`, auto-updated by the app |

### Non-goals (for now)
- macOS/Linux builds. The core crate stays platform-neutral where cheap, but this isn't a target.
- Automated captcha solving. If a site needs a captcha, the user is sent to the browser.
- Auto-categorising files by extension. Not selected. Packages and per-package folders cover organisation.

---

## 2. Feature Set

### 2.1 Download sources / protocols
| Protocol | Approach | Phase |
|---|---|---|
| HTTP/HTTPS | Own segmented engine (`reqwest` + `tokio`) | MVP |
| Video sites | `yt-dlp` subprocess, JSON progress | MVP |
| Galleries | `gallery-dl` subprocess with full flag support and impersonation | MVP |
| FTP / FTPS | `suppaftp` (async) with REST resume | 4 |
| SFTP | `russh` + `russh-sftp` | 4 |
| BitTorrent / magnet | `librqbit` embedded | 5 |
| Hoster plugins | Rhai scripts resolve page URLs to direct links | 6 |

### 2.2 JDownloader-style features
- **LinkGrabber**: paste, drag and drop, clipboard detection, or browser-sent links land in a staging area. Each link is resolved in the background (HEAD/probe, yt-dlp `-J`, gallery-dl `-j --simulate`, or a plugin) to show filename, size, availability (online/offline/unknown) and handler type. The user edits names and folders, picks formats, then confirms to queue.
- **Packages**: downloads are grouped into packages. Each package has a target folder, priority, queue, password list for extraction, and settings overrides. Grouping is automatic (multipart archive names, gallery/playlist source) and can be edited by hand.
- **Hoster plugins**: scripted site resolvers (see §6).

### 2.3 Download management
- Multiple **queues** with their own concurrency limit (default 3 active downloads, 8 segments each).
- **Scheduler**: start/stop times per queue, days of week, and an action when the queue finishes (nothing / sleep / hibernate / shutdown / close app).
- **Speed limiting**: global and per-download caps (token bucket via `governor`), with time-based profiles (e.g. 2 MB/s from 08:00 to 18:00).
- **Checksum verification**: MD5 / SHA-1 / SHA-256 / CRC32, from user input, plugin-provided hashes, or `.sfv` / `.md5` / `.sha256` files found in the package.
- **Retries**: exponential backoff with jitter, per-error-class policy (network vs 4xx vs 5xx vs 429 `Retry-After`).
- **Duplicate prevention**: global history check on URL and on final path + size + hash. A managed gallery-dl `--download-archive` and yt-dlp `--download-archive` per site, so re-runs only fetch new items.

### 2.4 Networking, accounts, extraction, remote
- **Proxies**: HTTP / HTTPS / SOCKS5 (with auth) profiles, assigned globally, per site (host-pattern rules), or per download. Settings are passed to yt-dlp and gallery-dl via `--proxy`.
- **VPN-aware**: bind to a chosen network interface (`local_address` / gallery-dl `--source-address`). An optional kill-switch pauses everything if that interface goes down or its IP changes.
- **Accounts & cookies**: site logins stored in Windows Credential Manager (`keyring` crate). Cookie jars per site, imported from the browser extension (preferred, see §7) or a Netscape `cookies.txt`, and handed to the tools as temporary cookie files.
- **Auto-extract**: zip / 7z / rar (incl. RAR5 and multipart) via bundled `7z.exe`. Runs when every part of a package is finished and verified. Tries the package password list, then the global one. Optionally deletes archives after a successful extract.
- **Remote / web control**: optional local HTTP server (`axum`) serving the same Svelte UI plus a REST + WebSocket API. Token authentication. Binds to localhost by default, with LAN opt-in.

### 2.5 Windows integration
- Start with Windows, minimised to tray (`tauri-plugin-autostart`).
- Tray icon with live speed, pause all / resume all, and an "add from clipboard" entry.
- Toast notifications on completion and errors, with *Open* and *Show in folder* actions.
- Keep the system awake while downloads are active (`SetThreadExecutionState`).
- Single instance (`tauri-plugin-single-instance`). Later launches forward their arguments to the running app.
- Protocol and file associations: `magnet:` (`tauri-plugin-deep-link`), `.torrent`, and an optional `.dlc`/link-list import.
- Explorer context menu "Download links from file with win-pydl" for `.txt` link lists. This uses the classic registry verb, which shows under *Show more options* on Windows 11. A modern `IExplorerCommand` menu needs package identity, so it's deferred.

---

## 3. Architecture

```
┌──────────────────────── Tauri app (src-tauri) ────────────────────────┐
│  Svelte UI (WebView2)  ◄── typed commands/events (tauri-specta) ──►   │
│                                                                        │
│  App layer: commands, event batching (≤10 Hz), tray, notifications,   │
│             clipboard watcher, single-instance, deep links             │
│                               │                                        │
│  ┌────────────────────── pydl-core (no Tauri dep) ──────────────────┐ │
│  │ Manager ─ Queues ─ Scheduler ─ RateLimiter ─ RetryPolicy           │ │
│  │    │                                                               │ │
│  │    ├── Handlers (trait Downloader):                                │ │
│  │    │     HttpSegmented │ Ftp │ Sftp │ Torrent │ YtDlp │ GalleryDl  │ │
│  │    ├── Resolvers (trait Resolver): Probe │ YtDlp -J │ gdl -j │ Rhai │ │
│  │    ├── PostProcessors: Checksum │ Extract(7z) │ Move/Rename         │ │
│  │    ├── Network: proxy rules, interface binding, cookie jars        │ │
│  │    ├── Storage: SQLite (sqlx) + migrations                         │ │
│  │    └── Tools: version check/update for yt-dlp, gallery-dl, ffmpeg  │ │
│  └────────────────────────────────────────────────────────────────────┘ │
└────────────▲───────────────────────────────────▲───────────────────────┘
             │ named pipe \\.\pipe\win-pydl       │ HTTP/WS (optional)
   ┌─────────┴─────────┐                 ┌───────┴────────┐
   │ pydl-native-host  │◄─ stdio ─ Browser extension (MV3) │ pydl-remote (axum) │
   └───────────────────┘                 └────────────────┘
```

### 3.1 Repository layout (Cargo workspace + pnpm)
```
win-pydl/
├─ Cargo.toml                  # workspace
├─ crates/
│  ├─ pydl-core/               # engine, storage, handlers, scheduler
│  ├─ pydl-plugins/            # Rhai host + plugin API
│  ├─ pydl-native-host/        # browser native-messaging bridge exe
│  └─ pydl-remote/             # axum server for remote control
├─ src-tauri/                  # Tauri shell, commands, tray, OS integration
├─ ui/                         # SvelteKit (adapter-static, SPA, ssr=false)
├─ extension/                  # MV3 extension (Chromium + Firefox manifests)
├─ plugins/                    # bundled .rhai hoster plugins
├─ tools/                      # build-time scripts to fetch yt-dlp/gallery-dl/ffmpeg/7z
├─ tests/fixtures/             # local test server scenarios, tool output samples
└─ .github/workflows/          # CI build/test, release + updater manifest
```

### 3.2 Key crates
| Concern | Crate |
|---|---|
| Async runtime | `tokio` |
| HTTP | `reqwest` (rustls, http2, socks, cookies, stream) |
| Rate limiting | `governor` |
| DB | `sqlx` (SQLite, offline mode, migrations) |
| Serialization | `serde`, `serde_json` |
| Errors / logs | `thiserror`, `anyhow` (app layer), `tracing` + `tracing-appender` (rolling file logs) |
| Hashing | `sha2`, `md-5`, `sha1`, `crc32fast` |
| FTP / SFTP | `suppaftp`, `russh`, `russh-sftp` |
| Torrent | `librqbit` |
| Scripting | `rhai` (+ `scraper` for HTML, `regex`) |
| Secrets | `keyring` (Windows Credential Manager) |
| Win32 | `windows` crate (clipboard listener, power, shutdown, named pipes) |
| TS bindings | `specta` + `tauri-specta` |
| Tauri plugins | single-instance, autostart, notification, deep-link, updater, dialog, opener, clipboard-manager, window-state |

### 3.3 Core abstractions (sketch)
```rust
trait Resolver {            // LinkGrabber stage
    fn can_handle(&self, url: &Url) -> Confidence;
    async fn resolve(&self, url: &Url, ctx: &ResolveCtx) -> Result<Vec<LinkCandidate>>;
}

trait Downloader {          // Execution stage
    async fn start(&self, job: &mut Job, ctx: &RunCtx) -> Result<Outcome>;
    async fn pause(&self, job: &Job) -> Result<()>;
    fn supports_resume(&self) -> bool;
}

enum JobKind { Http, Ftp, Sftp, Torrent, YtDlp, GalleryDl }
```
- `RunCtx` holds the rate-limiter handle, network profile, cookie jar, a cancellation token, and a progress sender.
- Progress goes through a `tokio::sync::watch`/`broadcast` per job. The app layer **coalesces** updates and pushes one batched event to the UI every 100–250 ms.

---

## 4. HTTP Segmented Engine (MVP core)

1. **Probe**: `HEAD`, or `GET` with `Range: bytes=0-0` when HEAD isn't supported. Records final URL after redirects, `Content-Length`, `Accept-Ranges`/206 support, `ETag`, `Last-Modified`, and the filename from `Content-Disposition` (RFC 6266/5987), falling back to the URL path, then content-type.
2. **Allocate**: create `<name>.pydl.part` and pre-size it with `set_len`. Segments write with positional writes (`FileExt::seek_write`), so no locking is needed between segments.
3. **Segments**: start with N (default 8, min segment 1 MiB). **Dynamic splitting** (IDM-style): when a connection finishes, split the largest remaining segment in half and start a new connection on it, until segments reach the minimum size.
4. **Persistence**: segment offsets are flushed to SQLite every ~2 s and on pause/exit. A small sidecar `.pydl.meta` file allows recovery if the DB is lost.
5. **Resume validation**: resume uses `If-Range` with the ETag/Last-Modified. If the server returns 200 instead of 206, or the size/ETag changed, the user chooses restart or keep.
6. **Non-range servers**: a single connection. Resume isn't possible, and the UI says so.
7. **Expiring links**: on 403/410 mid-download, ask the resolver/plugin to **refresh** the URL, then continue at the same offsets.
8. **Finish**: flush and fsync, verify the length, run the checksum if one is known, then rename atomically to the final name. Collisions follow the setting: rename `(1)`, overwrite, skip, or ask.
9. **Headers**: per-job custom headers, referer, user agent, cookies (from the extension or jar), and Basic/Digest auth.

---

## 5. yt-dlp & gallery-dl Integration

### 5.1 Tool management
- Bundled under `<app>/tools/` (installed) or `<portable>/tools/`.
- **Tool updater**: checks the GitHub releases of `yt-dlp/yt-dlp` and `mikf/gallery-dl` on startup (at most daily) and on demand. Downloads into a temp folder, verifies the published checksums (yt-dlp `SHA2-256SUMS`), and swaps atomically. Keeps the previous version for rollback. Channel per tool: stable / nightly.
- `ffmpeg.exe` is bundled for yt-dlp merges and gallery-dl ugoira conversion. Its path is passed explicitly (`--ffmpeg-location`).
- Every tool runs with `CREATE_NO_WINDOW`, a job object (so children die with the app), and UTF-8 forced (`PYTHONIOENCODING=utf-8`).

### 5.2 yt-dlp
- **Resolve** with `yt-dlp -J --flat-playlist`: title, formats, playlist entries. LinkGrabber shows a format picker (presets: best, ≤1080p, audio-only mp3/m4a, custom `-f` string).
- **Download** with `--newline --progress-template "download:%(progress)j"`, parsing the JSON lines for bytes, speed and ETA.
- Exposed options: format/sort (`-f`, `-S`), merge format, subtitles (`--write-subs`, `--embed-subs`, langs), thumbnails/metadata embedding, chapters, SponsorBlock, `--impersonate`, cookies, `--download-archive`, `--concurrent-fragments`, `--limit-rate`, `--proxy`, output template, and a **raw args** box.

### 5.3 gallery-dl
- **Resolve** with `gallery-dl -j --simulate <url>` (or `-s`). This gives the item list with metadata, so the LinkGrabber can show the count, a preview of the first thumbnails, and an estimated size where known.
- **Download** runs gallery-dl natively, so its extractors, postprocessors, rate limits and auth all work. Progress comes from parsing stdout (one path per downloaded file, `# ` prefix for skipped files) against the resolved total, plus `--write-log`/stderr for errors. An optional **"resolve-only" mode** uses `-g` URLs fed to the HTTP engine, for simple sites where segmented speed matters.
- **Common flags as UI controls** (per download, with per-site presets):

| Group | Flags |
|---|---|
| Output | `-d/--destination`, `-D/--directory`, `-f/--filename` (format string), `--no-part`, `--no-skip`, `--no-mtime`, `--rename`/`--rename-to` |
| Selection | `--range`, `--chapter-range`, `--filter`, `--chapter-filter`, `-A/--abort N`, `-T/--terminate N` |
| Auth | `-u/--username`, `-p/--password` (from the keyring), `--netrc`, `-C/--cookies` (temp file from the jar), `--cookies-from-browser` |
| Network | `--proxy`, `--source-address`, `-r/--limit-rate`, `-R/--retries`, `--http-timeout`, `--sleep`, `--sleep-request`, `--sleep-extractor`, `--user-agent` |
| **Impersonation** | `-o browser=<chrome\|firefox>` (header + TLS-cipher mimicry), `--user-agent browser`, plus TLS-fingerprint impersonation (curl_cffi-backed) where the bundled build supports it |
| Archive | `--download-archive` (managed per site by the app) |
| Metadata / post-processing | `--write-metadata`, `--write-info-json`, `--write-tags`, `--zip`, `--cbz`, `--ugoira <fmt>`, `-P/--postprocessor`, `--exec`, `--exec-after`, `--no-postprocessors` |
| Advanced | `-o/--option KEY=VALUE` (repeatable list), `-c/--config` file, `--config-ignore`, **raw args** box |

> ⚠️ **Verify at implementation time**: gallery-dl flag names, the impersonation option names, and whether the standalone `gallery-dl.exe` ships `curl_cffi` all change between versions. The UI flag schema lives in a versioned JSON file (`gallery-dl-flags.json`), so updates don't need code changes. A startup self-check runs `gallery-dl --help` and hides controls for flags the bundled version lacks.

### 5.4 Shared behaviour
- Speed limiting: the global budget is split across active subprocesses through their `--limit-rate`/`-r` flags. This is approximate, and the docs say so.
- Pause = kill the process, resume = re-run with the archive/`--continue`. Both tools skip already-finished files.
- Cookies, proxy, interface and account settings from §2.4 are mapped to each tool's flags automatically.

---

## 6. Hoster Plugin System (Rhai)

- Plugins are `.rhai` files in `plugins/` (bundled) and `%APPDATA%\win-pydl\plugins\` (user). They hot-reload on change.
- Manifest header: `name`, `version`, `url_patterns` (regex), `needs_account`, `max_connections`, `min_wait`.
- Entry points: `check(url)` → availability/size/name, `resolve(url, account)` → direct links + headers + optional hash, `login(account)` → session cookies, and `refresh(url)` for expired links.
- Host API (sandboxed, no filesystem or process access): `http.get/post` (shares the cookie jar and proxy), `html.select`, `regex`, `json`, `sleep`, `log`, `wait_countdown(secs)` (shown in the UI), `open_browser(url)` for manual captcha.
- Limits: max operations, timeout per call, memory cap.
- Resolution order: user plugin → bundled plugin → yt-dlp (if `--simulate` succeeds) → gallery-dl (if an extractor matches) → plain HTTP probe.

---

## 7. Browser Integration

- **Extension (Manifest V3)**, Chromium (Chrome/Edge/Brave) plus a Firefox manifest variant:
  - Intercepts downloads (`chrome.downloads.onCreated`) that match rules: min size, extensions, an always/never site list, and a modifier key to bypass. It cancels the browser download and sends URL, referer, filename, cookies (`chrome.cookies.getAll` for that URL) and user agent.
  - Context menus: *Download with win-pydl*, *Send all links on page to LinkGrabber*, *Send selection links*, *Send page to gallery-dl / yt-dlp*.
  - A popup shows connection status and lets you pause interception.
- **Native messaging host** (`pydl-native-host.exe`): registered under `HKCU\Software\Google\Chrome\NativeMessagingHosts`, `...\Microsoft\Edge\...` and `...\Mozilla\NativeMessagingHosts` by the installer. Relays JSON over a named pipe to the app and starts the app if it isn't running.
- **Clipboard monitoring**: `AddClipboardFormatListener` on a hidden window. Detects URLs with configurable patterns and ignores the app's own copies. Off, ask (toast), or straight to LinkGrabber.
- Note: Chrome's app-bound cookie encryption (Chrome 127+) makes reading Chrome cookies from disk unreliable. The extension is the main way to get cookies. `--cookies-from-browser` is offered mainly for Firefox.

---

## 8. Data Model (SQLite)

| Table | Key fields |
|---|---|
| `packages` | id, name, dest_dir, queue_id, priority, extract_passwords, settings_json, created_at |
| `downloads` | id, package_id, kind, source_url, resolved_url, filename, dest_path, size, downloaded, status, error, handler_opts_json (yt-dlp/gdl args), checksum_algo/value, etag, last_modified, created/finished_at |
| `segments` | download_id, idx, start, end, written |
| `queues` | id, name, max_active, schedule_json, on_complete_action, speed_limit |
| `link_grabber` | staged candidates (same shape as downloads + availability) |
| `history` | url_hash, final_path, size, sha256, finished_at (duplicate detection) |
| `accounts` | id, site, username, keyring_ref, cookies_json, enabled |
| `proxy_profiles` / `proxy_rules` | type, host, port, auth keyring_ref / host pattern → profile |
| `speed_profiles` | schedule → global limit |
| `tools` | name, version, channel, path, last_check |
| `settings` | key, value_json |

Locations: installed builds use `%APPDATA%\win-pydl\` (db, logs, plugins, archives, cookies). Portable builds use `.\data\`, detected by a `portable.flag` file next to the exe.

---

## 9. UI (SvelteKit, Fluent style)

- **Theme**: light/dark that follows the system, Mica window background (Tauri `windowEffects`), Segoe UI Variable, accent colour from Windows.
- **Layout**: left nav (*Downloads*, *LinkGrabber*, *Queues*, *Completed/History*, *Plugins*, *Tools*, *Settings*), a main virtualised table (thousands of rows stay smooth), a bottom details panel (segments visualisation, log, files, options), and a status bar (global speed, active/queued count, speed-limit toggle).
- **Downloads view**: package rows that expand to files. Columns are configurable (name, size, progress, speed, ETA, status, host, queue). Multi-select actions, drag to reorder priority, and a right-click menu.
- **LinkGrabber view**: incoming links grouped into proposed packages, availability badges, inline rename, folder picker, format picker (yt-dlp), and a gallery-dl options drawer. *Start all* / *Add to queue*.
- **Add dialog**: URL(s), handler auto-detected (overridable), folder, queue, and per-handler options with presets.
- Keyboard shortcuts (Ctrl+V to add from clipboard, Del, Space to pause/resume, Ctrl+F to filter).
- Svelte stores fed by batched Tauri events. Commands are fully typed via tauri-specta.

---

## 10. Packaging, Updates, CI

- **Build**: `cargo tauri build` produces an NSIS installer. A CI step also produces the **portable zip** (exe + tools + `portable.flag`).
- **Installer extras**: native-host registry entries, `magnet:` handler, `.torrent` association, optional autostart, WebView2 bootstrapper.
- **App updates**: `tauri-plugin-updater` with a minisign key (kept in GitHub secrets) and `latest.json` on GitHub Releases. Because the repo is private, the update endpoint needs a public release mirror or a token-free proxy (see Open Decisions).
- **Authenticode signing**: optional for personal use. Without it, expect SmartScreen warnings.
- **CI (GitHub Actions, windows-latest)**: `cargo fmt --check`, `clippy -D warnings`, `cargo test`, `pnpm check` / `vitest`, plus a build artefact on every push. On tag `v*`: build, sign the updater bundle, and publish the release.

---

## 11. Testing Strategy

- **Unit**: segment split math, filename parsing (Content-Disposition, sanitisation of Windows-reserved names and lengths), retry policy, scheduler time logic, gallery-dl/yt-dlp output parsers (fixtures of real output).
- **Integration**: an in-process `axum` test server that simulates range support, no range, slow/throttled transfers, dropped connections mid-segment, changed ETag, redirects, 429 with `Retry-After`, expiring URLs (403 then refresh), and huge sparse files.
- **Tool tests** (opt-in, network): smoke runs of the bundled yt-dlp/gallery-dl against a few stable public URLs.
- **UI**: Vitest for stores/components. A Playwright smoke test against the static UI with a mocked IPC layer.
- **Soak**: 500 queued downloads, pause/resume storms, kill the app mid-download, then confirm resume works.

---

## 12. Roadmap

| Phase | Scope | Exit criteria |
|---|---|---|
| **0. Scaffold** | Workspace, Tauri + SvelteKit shell, SQLite migrations, tracing logs, CI, Fluent theme base | App launches, CI green |
| **1. MVP** | Segmented HTTP engine (resume, retries, dynamic splitting, headers/cookies/auth), queues + concurrency, packages, LinkGrabber (paste/clipboard/drag-drop), yt-dlp + gallery-dl (resolve, download, progress, common flags, impersonation, archives) + tool updater, tray, toasts, history/duplicate detection, single instance, settings | Used as daily driver for HTTP, video and galleries |
| **2. Browser** | MV3 extension (Chromium + Firefox), native host, download interception, context menus, cookie hand-off | Browser downloads flow into the app with cookies |
| **3. Control** | Scheduler, speed limiting + profiles, keep-awake, on-complete actions, checksum verification | Overnight queue then shutdown works |
| **4. Network & protocols** | Proxies + rules, interface binding + kill-switch, accounts in the keyring, FTP/FTPS, SFTP | Downloads via SOCKS5/VPN interface, FTP/SFTP resume |
| **5. Torrent** | librqbit integration, magnet/.torrent associations, seeding limits (ratio/time) | Torrent download + seed from the same UI |
| **6. Plugins & extract** | Rhai plugin host + API, 2–3 reference plugins, auto-extract with password lists | A hoster resolved via plugin, archive auto-extracted |
| **7. Remote & release polish** | axum remote UI/API, portable build, auto-update pipeline, installer polish | Install, auto-update and phone control work end to end |

---

## 13. Risks & Mitigations

| Risk | Mitigation |
|---|---|
| yt-dlp/gallery-dl break when sites change | Auto-updater with nightly channel and rollback. Tool version shown in errors. |
| gallery-dl flags/impersonation differ by version | Versioned flag schema plus `--help` self-check (§5.3) |
| PyInstaller-built tool exes flagged by Defender | Verify checksums, document an exclusion, allow pointing to a user-installed copy as a fallback |
| Chrome cookie encryption | Cookies come from the extension. Firefox for `--cookies-from-browser`. |
| WebView2 lag with many live rows | Virtualised table, batched events, only visible rows re-render |
| Private repo vs updater endpoint | Public "releases-only" repo, or a token-authenticated endpoint |
| Windows 11 modern context menu complexity | Classic menu first. Sparse-package `IExplorerCommand` later if wanted. |
| Subprocess rate limits are approximate | Document it. Use "resolve-only" mode when exact limits matter. |

---

## 14. Open Decisions

1. **Final name**: keep `win-pydl` or rename now that it's Rust (affects crate names, pipe name, registry keys and the extension ID, so decide before Phase 2).
2. **Updater hosting**: separate public releases repo vs. private repo + token. Recommended: public releases-only repo.
3. **Rhai vs Lua (`mlua`)**: Rhai is chosen for its pure-Rust build and sandboxing. Lua is the fallback if plugin performance or familiarity becomes an issue.
4. **gallery-dl default mode**: native download (recommended) vs resolve-only via the HTTP engine, per site.
5. **Default concurrency**: 3 active × 8 segments. Tune after real-world testing.
6. **Authenticode certificate**: skip for personal use, or buy one (or use Azure Trusted Signing) if it's ever distributed.

---

## 15. Decision Log (from planning Q&A, 2026-10-24)

- Language **Rust**. UI **Tauri 2 + SvelteKit/TS**, **modern Windows 11** look.
- Goal **personal daily driver**. Benchmark **JDownloader**: LinkGrabber, packages, hoster plugins.
- Protocols: **HTTP(S) segmented, FTP/SFTP, BitTorrent/magnet, yt-dlp, gallery-dl** (common flags + impersonation).
- Browser: **extension + native host, and clipboard monitoring**.
- yt-dlp/gallery-dl: **bundled binaries, auto-updated**.
- Plugins: **scripted (Rhai)**.
- Management: **scheduler & queues, speed limiting, checksum verification**.
- Extras: **proxy/VPN-aware networking, accounts & cookies, auto-extract, remote/web control**.
- Packaging: **installer + portable, auto-update**.
- MVP: **HTTP core + UI + yt-dlp/gallery-dl**.
- gallery-dl UI: **common flags as controls + raw args**.
- Duplicates: **global history + managed download archives**.
- OS: **autostart to tray, toasts, prevent sleep, Explorer & protocol integration**.
- Repo: **private GitHub, MIT**.
