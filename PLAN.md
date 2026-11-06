# Rusty Downloader — Project Plan

A full-featured download manager for Windows, modeled on **JDownloader**. It has a Rust engine, a Tauri 2 + SvelteKit UI, and built-in **yt-dlp** and **gallery-dl** support.

> Naming: the product is **Rusty Downloader** (binary, package, data folder: `rusty-downloader`; app identifier `dev.rustydownloader.app`). Internal crates use the short prefix `rdl-` (`rdl-core`, later `rdl-plugins`, `rdl-native-host`, `rdl-remote`). Repo: `Gert-Steyn/rusty-downloader`.

---

## 0. Current Status (updated 2026-11-02)

**Where we are:** Phases 0, 1 and **2 (browser integration, 2a–2e) are done**; only a live Firefox run is still owed (see the 2d/2e follow-ups). **Phase 3 (3a keep-awake, 3b speed limits, 3c queues, 3d checksums) is built**; only its exit check by hand (an overnight queue ending in a real shutdown) is owed. **Phase 4 (4a proxies, 4b adapter binding and kill-switch, 4c accounts and cookies, 4d FTP / FTPS, 4e SFTP) is built**; its exit check by hand (4b with a real VPN going up and down) is still owed. **Version 1.0.0 is released** (GitHub Release `v1.0.0`, 2026-11-02: installer, portable zip, extension zips; see "Release 1.0"). **Next up:** a LinkGrabber bug found in 1.0, then a gallery subfolders switch and rerunning a gallery from the right-click menu, then planning Phase 5 (torrents). To pick up where the last session stopped, read **"Resume here"** below.

The app works as an HTTP(S), video and gallery download manager that lives in the tray. You can paste, drop or copy links into the LinkGrabber (optionally "ask first", with ignore patterns), review and rename them, pick a video format or gallery options, and start them. Links you downloaded before are flagged. Plain files download segmented and resumable; `ftp://`, `ftps://` and `sftp://` links download over one connection and resume where they stopped; SFTP host keys are trusted on first use. Pages from sites yt-dlp supports are recognised automatically, and playlists expand into one link per video. Pages yt-dlp doesn't know (or can't read) go to gallery-dl, and a gallery becomes one download into its own folder. yt-dlp, gallery-dl and ffmpeg install themselves on first use. Finished downloads go into History; Windows notifications report finished packages and failures. A browser extension (Edge, Chrome, Brave; Firefox untested) sends links, selections, pages and matching downloads to the app with the browser's cookies, starting the app if needed. Windows doesn't go to sleep while downloads run. Speed can be capped from the status bar, per download, or by a weekly schedule. Packages can go in separate queues that run only at set times and sleep, shut down or close the app when done, after a minute's countdown you can cancel. A download can carry an MD5 / SHA-1 / SHA-256 / CRC32 checksum, typed in or taken from a `.sfv` / `.md5` / `.sha1` / `.sha256` file in its package, and fails if its file doesn't match. Downloads, link checks, tool updates and yt-dlp / gallery-dl can go through HTTP, HTTPS or SOCKS5 proxies, picked per site by rules or per download, and can all be tied to one network adapter (a VPN's) with a kill-switch that holds downloads while it's down. Site accounts add a login and cookies imported from a `cookies.txt` to every request for their site. Settings cover the download folder, limits, clipboard, start with Windows, the tray and keep-awake. Everything survives an app restart. It installs for one user or everyone (and uninstalls cleanly), or runs portable from a folder.

### Done so far

| Milestone | Commit | What landed |
|---|---|---|
| Phase 0 – scaffold | `d489b40` | Cargo workspace, Tauri 2 + SvelteKit shell, SQLite migrations, tracing logs, CI workflow, Fluent theme |
| Rename | `768a07c` | `win-pydl` → Rusty Downloader / `rdl-core` |
| 1a – HTTP engine | `fe0f510` | Probe, filenames (RFC 6266/5987, Windows-safe), dynamic segment splitting, pause/resume, `If-Range`, retries |
| 1b – Download manager | `f2f4992` | SQLite-backed packages/downloads/queues, scheduler with per-queue limits, segment saves every ~2 s, crash recovery, event stream |
| 1c – Downloads UI | `bea4e42` | tauri-specta typed IPC, batched events, virtualised treegrid, details panel, add/remove dialogs |
| 1d – LinkGrabber | `adba9eb` | Background probing + availability, archive-set grouping, inline renames, clipboard watcher, drag-drop, global paste |
| 1e – Tools + yt-dlp | `462a74b` | Tool manager (install on first use, SHA-256 verified, versioned folders, rollback, daily yt-dlp auto-update), job-object subprocess runner, yt-dlp resolve/download, playlist expansion, format presets, Tools page |
| 1f – gallery-dl | `ba04aa8` | gallery-dl from Codeberg (stable) or `gdl-org/builds` (nightly), `-j` resolve into one gallery link, native download with file-count progress, managed per-site archives, flag schema + `--help` self-check, options dialog with impersonation and raw args, migration 4 |
| 1g – Desktop polish | `804ae23` | Settings page, history + duplicate warnings, tray icon and menu, close to tray, start with Windows (in the tray), Windows notifications with buttons, clipboard "ask first" + ignore patterns, right-click menu and drag-to-reorder in Downloads, migration 5 |
| 2a – Request context | `ab1cc74` | `rdl-core::request` (referer, user agent, browser-shaped cookies with RFC 6265 matching), redirects followed by hand so each hop gets its own cookies, temporary Netscape cookie files for yt-dlp and gallery-dl (checking and downloading), LinkGrabber `add_with_request`, migration 6 |
| 2b – Pipe server | `f281838` | `rdl-bridge` crate (protocol v1: `ping`, `status`, `add`; native-messaging framing; pipe server and client), app handler sending links to the LinkGrabber or the queue with their request context, `browser` link source, Windows notification for browser links, `tools/dev/pipe.mjs` |
| 2c – Native host | `f7df5be` | `rdl-native-host.exe`: relays native messages to the pipe, checks the pipe is served by `rusty-downloader.exe` from its own folder under the same user, starts the app in the tray for an `add`, keeps the browser's pipes out of the app; error replies gain a `code` (`notRunning`, `untrusted`, `unavailable`) |
| 2d – Extension | `8ad707b` | MV3 extension (`extension/`, plain JS, Chromium + generated Firefox build): right-click menus, download takeover by type/size/site rules, cookies per link site, popup with status and switch, options page; pinned id; unit tests in CI |
| 2e – Registration | `8ad707b` | App registers the host for Chrome, Edge, Brave, Chromium and Firefox on every start; `pnpm dev` builds the host, `pnpm build` bundles it next to the app |
| 3a – Keep-awake | `744f4d3` | A Windows power request ("system required") held while downloads run, released when none do; "Keep the computer awake while downloading" setting (on by default) |
| 3b – Speed limits | `3489fb7` | Token-bucket limiter in the engine; global limit (status bar switch), per-download limits (right-click, migration 7), weekly schedule rules in Settings; `--limit-rate` for yt-dlp / gallery-dl |
| 3c – Queues | `45ee68e` | Queues page (add, rename, remove, downloads at once), weekly window per queue (stops and requeues at close), "when finished" action (sleep, hibernate, shut down, close the app) after a cancellable 60 s countdown, "Move to <queue>" in the package menu |
| 3d – Checksums | `a89a16b` | `rdl-core::checksum` (MD5, SHA-1, SHA-256, CRC32; pasted values recognised by length; list parsing), check after each plain / yt-dlp download, a mismatch fails and keeps the file, a retry deletes it, `.sfv`/`.md5`/`.sha1`/`.sha256` lists in the package fill in values, "Checksum…" dialog and ✓/✗ in details, migration 8; fixed a pause-vs-start race |
| 4a – Proxies | `fa6a8b2` | `rdl-core::proxy` (profiles, host-pattern rules, default, per-download choice), `rdl-core::secrets` (Windows Credential Manager), `rdl-core::network` (route per request, one client per proxy) used by downloads, LinkGrabber checks and tool updates; `--proxy` for yt-dlp and gallery-dl; Settings → Network and "Connection…" per download; migration 9; fixed a LinkGrabber race |
| 4b – Adapter binding | `bb5c13c` | `rdl-core::adapters` (Windows adapters, usable address), `Binding` in `Network` (bound to an address, or down: nothing connects), `--source-address` for the tools, a network hold in the manager (running downloads requeued, nothing starts), `network_watch` in the app (every 2 s and on settings changes; hold before unbinding, bind before releasing; restart on a new address; notifications), adapter picker, kill-switch and status bar note |
| 4c – Accounts | `fa1f137` | `rdl-core::accounts` (site pattern, optional login, cookies), password in Credential Manager, cookies sealed with DPAPI (migration 10), `request::parse_netscape` + `merge_cookies`, `Network::with_account` for downloads / link checks / both tools, HTTP Basic answered once per host, `--username` / `--password` for the tools, Settings → Accounts with cookies.txt import; two racy tests fixed |
| 4d – FTP / FTPS | `c731b94` | `rdl-core::ftp` on `suppaftp` (tokio, rustls): size and `MDTM` probe, one connection resumed with `REST` from the part file, retries that resume, start-over when `REST` is refused, explicit FTPS checked against the Windows store, login from the link / account / anonymous, control and data connections bound to the 4b adapter; `DownloadKind::Ftp`; LinkGrabber, Add links and the browser accept `ftp(s)://` |
| 4e – SFTP | `2eaf0fb` | `rdl-core::sftp` on `russh` 0.65 + `russh-sftp` 3: password then `%USERPROFILE%\.ssh` key login, host keys trusted on first use in `known_hosts` (migration 11) and refused when changed, resume by reading from the part file's length, bound to the 4b adapter; `rdl-core::single` now holds the one-connection resume loop FTP and SFTP share; `DownloadKind::Sftp`; Settings → Known SFTP hosts with Forget |
| Release 1.0 | `091d0b4` | Version 1.0.0; NSIS installer for this user or all users with branded images, license page and launch box; clean uninstall (`--uninstall` run by an NSIS hook); portable zip (`pnpm portable`); `release.yml` publishes installer, portable zip and extension zips on `v*` tags; a scheduler race fixed (queue limit changes mid-pass) |

Tests: 238 Rust tests and 9 extension tests (`pnpm ext:test`) (unit tests; engine, manager and LinkGrabber suites against a local misbehaving HTTP server; an FTP suite against a small local FTP server; an SFTP suite against a local `russh` SSH / SFTP server; tool-manager suite against fake GitHub and Codeberg APIs; gallery-dl suite against a fake `gallery-dl.cmd`; pipe suite against real named pipes; native-host suite driving the real host binary over stdin/stdout; app tests). They include a check that `ui/src/lib/bindings.ts` is up to date. Five opt-in network tests in `crates/rdl-core/tests/tools_live.rs` install the real tools, pause/resume a real yt-dlp download, check the gallery-dl flag schema against the real `--help`, and resolve and download a real gallery twice (`RDL_LIVE_DIR=<folder> RDL_LIVE_GALLERY=https://commons.wikimedia.org/wiki/Category:Big_Buck_Bunny cargo test -p rdl-core --test tools_live -- --ignored --test-threads 1`). `svelte-check` passes with 0 errors or warnings. Every milestone was also verified end to end in the running app (see "How to test in the real app" below). CI on GitHub passes (last checked 2026-11-05, run 42056799073 at `1273642`: clippy, tests, the installer and the portable zip), and the `v1.0.0` release workflow passed (run 41475789550).

### How 1e works

- **Tools** (`rdl-core::tools`): yt-dlp from `yt-dlp/yt-dlp` (or `yt-dlp-nightly-builds`), ffmpeg from `yt-dlp/FFmpeg-Builds` (the *shared* win64 GPL build, ~90 MB, `bin/` unpacked). **Decision: tools are downloaded on demand, not bundled.** yt-dlp installs the first time the LinkGrabber meets a web page; ffmpeg the first time a yt-dlp download starts.
  - Each install lives in `<data>/tools/<tool>/<release>-<sha8>/`. The `tools` row points at the active exe and keeps the previous one, so update and rollback are row updates and never touch files a running download holds. Older folders are pruned.
  - Downloads are checked against the release's checksum file before unpacking, and the new binary must answer `--version` before it's switched to (5 min timeout: Defender scans a never-seen PyInstaller exe on first launch, which took over 60 s here).
  - Updates are detected by asset SHA-256, not tag, because FFmpeg-Builds republishes under `latest`. Background check runs 10 s after start and hourly, at most daily per tool. yt-dlp and gallery-dl auto-update; ffmpeg only on request.
- **Processes** (`rdl-core::process`): `CREATE_NO_WINDOW`, UTF-8 env, and one job object per process with kill-on-close. Dropping the job kills the whole tree (yt-dlp.exe is a PyInstaller bootloader plus a child, plus ffmpeg).
- **yt-dlp** (`rdl-core::ytdlp`): always `--ignore-config --no-playlist --encoding utf-8`. Resolve is `-J --flat-playlist`. Downloads print tagged lines: `RDLINFO` (planned path and size), `RDLPROG` (progress JSON, throttled to 4 Hz) and `RDLDONE` (final path after post-processing). Streams of one item are summed for progress. Pause kills; resume re-runs with `--continue`, so the output name must be deterministic: either the user's rename or `%(title).180B [%(id)s].%(ext)s`.
- **LinkGrabber:** a link whose probe returns HTML, or fails, is offered to yt-dlp. "Unsupported URL" falls back to the plain HTTP result. Playlists replace their link with one link per entry (untitled entries use the URL slug). Format presets: best, ≤4K/1080/720/480, MP3, M4A, custom `-f`. A size estimate comes from the `-J` formats.
- **Test sites:** YouTube and Vimeo ask automated clients to sign in from this machine. Use `https://archive.org/details/BigBuckBunny_124` (video, format `0` = 46,935,223 bytes) and `https://soundcloud.com/forss/sets/soulhack` (11-track playlist).

### How 1f works

- **Where gallery-dl comes from:** development moved to **Codeberg** in 2026; the GitHub releases of `mikf/gallery-dl` no longer carry binaries. Stable installs `gallery-dl.exe` from `codeberg.org/mikf/gallery-dl` (Forgejo's release API has the same JSON shape as GitHub's, at `https://codeberg.org/api/v1/`) and checks it against the release's `SHA256SUMS`. Nightly installs `gallery-dl_windows.exe` from GitHub `gdl-org/builds`, which has no checksum file, so it is checked against the SHA-256 `digest` GitHub records for each asset (`Upstream.sums = None`). `ToolsConfig.codeberg_base` lets tests point Codeberg at a local server.
- **Resolve** (`rdl-core::gallerydl`): `--config-ignore -j --range 1-2000 -- <url>`, 90 s limit. `-j` prints one JSON array of messages at the end: `2` directory metadata, `3` file URL + metadata, `6` a sub-gallery URL, and an error as `[-1, {error, message}]` (older: `["HttpError", "..."]`). Unsupported URLs exit with status 64 and log `Unsupported URL`; a page with nothing on it returns `[]` and exit 0. Hitting the 2,000-file cap or the time limit marks the gallery `partial` (count is a lower bound). Title comes from a string `page` (Wikimedia, where `title` is each file's own), else the first metadata with `title`, `album`, `gallery`, `user`, ... (see `title_of`).
- **LinkGrabber order:** HTML page (or failed probe) → yt-dlp → gallery-dl → plain HTTP. gallery-dl is also asked when yt-dlp *fails* on a page (image tweets make yt-dlp say "no video"); if both fail, the first tool that knew the site explains why. A find by yt-dlp's catch-all extractors (`Generic`, `HTML5MediaEmbed`) is held back until gallery-dl has had its turn: those match any page with an embedded video (on a Wikimedia category of 82 images they offered the one video). A gallery stays **one** link (kind `gallerydl`); its name is the **folder** the files go into, and the gallery title replaces the default package name like a video title does.
- **Download** (`manager/gallerydl_job.rs`): `-D <package folder>/<name>`, so the folder is the gallery's own. With stdout piped, gallery-dl prints each finished file's absolute path, with `# ` in front when skipped (already on disk or in the archive). Progress is **files**, not bytes: `downloads.files_done/files_total` (migration 4), `Event::Progress.files`, and the UI shows `12/40`. A Selection option or raw args leave the total unknown; finishing stores the real count. Pause kills; resume re-runs, the skipped files count as done again, and the shown count never drops. A non-zero exit fails with the last `[...][error]` log line plus the warning that explains it (e.g. `429 Too many requests`).
- **Archives:** `--download-archive <data>/archives/gallery-dl/<host>.sqlite3`, one per site (by URL host, `www.` dropped), on by default. Re-downloading a gallery into a new folder therefore fetches nothing; the option is in the dialog.
- **Delete files:** each file a gallery download wrote, or skipped because it was already in the gallery's folder, is stored in `download_files`; "delete files" removes exactly those (for any status), then the folder only if it's empty.
- **Flag schema:** `crates/rdl-core/src/gallerydl/flags.json` (embedded, versioned, checked against 1.32.17's `--help`) lists the controls by group. `GalleryDlOptions` stores values by schema id plus `archive`, `userConfig` (off = `--config-ignore`) and `rawArgs`. The `--help` self-check (`supported_flags`, cached per exe path) drops flags the installed version lacks, and the dialog hides them.
- **ffmpeg** is put on gallery-dl's `PATH` when already installed (for ugoira conversion) but never installed just for a gallery.
- **Testing on Wikimedia:** it answers `429 Too many requests` after a few full-size files; use small ranges and a sleep.

### How 1g works

- **Settings** (`rdl-core::settings::Settings`): one JSON value under the `app` key (missing fields get defaults; the old `linkgrabber.clipboard_watch` key is read once as a fallback). `AppState::update_settings` validates (limits 1–20 active, 1–32 connections, absolute folder, trimmed patterns), saves and applies: the main queue's `max_active`, `Manager::set_max_connections` (new downloads only), and the LinkGrabber's duplicate lookups. "Start with Windows" goes through `tauri-plugin-autostart` (HKCU `Run` entry with `--minimized`), and `get_settings` reports what Windows actually has. The Settings page saves each change at once and reloads on every visit, since the LinkGrabber's clipboard switch also writes settings.
- **Default folder:** `AppState::download_dir` (the chosen folder, else the user's Downloads) is used by Add links and LinkGrabber confirm; `system_downloads_dir` is the plain Downloads folder for the Settings page.
- **History** (`rdl-core::history`, migration 5 adds `name`, `kind`, `package`): the job runner records every completed download (gallery: its folder). Rows are keyed by SHA-256 of the URL without its `#fragment`, never by download id. `exists` is checked when read. The History page searches name/link/folder/package (LIKE wildcards escaped), opens, reveals, forgets and clears.
- **Duplicate warnings:** with `warn_duplicates` on, every `GrabbedLink` carries `previous` (last finish time, path, whether it still exists), filled in list/add/update/playlist expansion. The UI shows "Downloaded 3 days ago" and unticks the link once if the file is still there; re-ticking sticks.
- **Clipboard:** links matching an ignore pattern (`*` wildcard, else substring, case-insensitive; `settings::wildcard_match`) are dropped. With "ask first" the watcher emits a `ClipboardOffer` event and the toast gets an Add button (10 s).
- **Tray** (`src-tauri/src/tray.rs`): menu Open / Add links from clipboard / Pause all / Resume all / Quit; left click opens the window; the tooltip shows running downloads and combined speed (from download events, once a second). The main window starts hidden (`visible: false`) and is shown in setup unless launched with `--minimized` and "Start in the tray" is on. With "close to tray" on, closing hides the window; a second launch (single instance) or the tray brings it back.
- **Notifications** (`src-tauri/src/notify.rs`): WinRT toasts via `tauri-winrt-notification`. Unpackaged apps need a known AppUserModelID, so at start the app registers `HKCU\Software\Classes\AppUserModelId\dev.rustydownloader.app` (name and an icon written to `<data>/notification-icon.png`). A package finishing → "Download finished" (Open, Show in folder); failures within 3 s → one toast (Retry / Retry all). Nothing is sent while the window has focus. Toasts are shown one at a time: two in the same instant collided in Windows' notification store (`SQLITE_CONSTRAINT_UNIQUE`, 0x87AF0813) and one was lost.
- **Downloads table:** right-click opens a `ContextMenu` (download: pause/resume/retry, open, show in folder, copy link, remove; package: pause/resume all, open folder, copy links, move to top/bottom, remove). Package rows drag with a custom data type (`application/x-rdl-package`, ignored by the link-drop overlay); dropping on a package puts the dragged one before it, below the list puts it last. `Manager::move_package` renumbers `packages.position` and emits `PackagesReordered`; the scheduler starts packages in that order.
- **Checked in the app (2026-10-25):**
  - DB migrated to schema 5; the window starts hidden and is shown in setup.
  - Settings: a valid change saves ("Saved"), an invalid one (0 at once) shows the error and the field reverts; a relative folder is refused; the chosen folder is used by Add links.
  - History: two downloads from a local server were recorded; search narrows; the page lists name, link, package, size, date.
  - Duplicates: re-adding `sample.bin` showed "Downloaded 2 minutes ago", unticked; re-ticked, it stayed ticked through a recheck. A link differing only in its query wasn't flagged.
  - Notifications (read back from Windows' toast history): "Download finished · notes.txt · 6.8 KB" with Open and Show in folder; two packages finishing together now both arrive (they collided before the fix); a 404 gave "missing-file.zip failed" with Retry (it showed the whole URL before a fix).
  - Close to tray (closing the window kept the process with no visible window); a second launch brought the window back and exited.
  - Start with Windows added `Run\Rusty Downloader = …\rusty-downloader.exe --minimized` and removed it when switched off; launching with `--minimized` stayed in the tray.
  - Clipboard "ask first": a copied link matching an ignore pattern did nothing; text with a wanted and an ignored link offered "1 link copied", and Add added just the wanted one.
  - Downloads: the right-click menu (with Move to bottom disabled on the last package), Move to top, and dragging a package (indicator shown, link overlay not) reordered as expected.
  - Not checked: clicking the tray menu items and the notification buttons (neither can be driven from DevTools); the Mica background.
  - Cleaned up: test downloads (with files), LinkGrabber, history and notifications cleared, settings back to defaults, no `Run` entry. The test overwrote the clipboard (it held no text before) and cleared it afterwards.

### How 2a works

- **`RequestContext`** (`rdl-core::request`): `referer`, `user_agent` and `cookies`. A `Cookie` has the fields of the browser's `chrome.cookies.Cookie` (`name`, `value`, `domain`, `hostOnly`, `path`, `secure`, `httpOnly`) except that `expires` is whole Unix seconds (the extension will convert `expirationDate`). `cleaned()` drops a non-http(s) referer, an invalid user agent, and malformed or expired cookies; everything from outside goes through it, and so does every read from the DB.
- **Storage** (migration 6): `downloads.request_json` and `link_grabber.request_json`, NULL when empty. A download's copy is cleared when it completes, so cookies don't outlive their use (a failed or paused one keeps them for the retry). Playlist entries inherit their playlist link's. The UI never sees cookies: they aren't on `Download` or `GrabbedLink`, and `bindings.ts` is unchanged.
- **HTTP:** the engine client no longer follows redirects; `http::get` does, up to 10, and picks the headers again for each hop (`RequestContext::headers_for`). So a redirect to another host (or a CDN subdomain the cookie doesn't cover) gets no cookies, as in a browser. The probe and every segment / stream request go through it. The tool manager uses `http::api_client()`, which still follows redirects itself. Too many redirects is `Error::TooManyRedirects`.
- **Cookie matching:** RFC 6265: host-only cookies match their host exactly, others also subdomains; IPs only exactly; path prefix on `/` boundaries; `Secure` only over https; longer paths first in the header.
- **Tools:** `CookieFile` writes a Netscape `cookies.txt` (`#HttpOnly_` prefix, empty expiry for session cookies, which both tools read as session cookies) to `%TEMP%
dl-cookies-<pid>-<random>.txt` and deletes it on drop, after the process has ended. Files older than a day left by a crash are swept when the manager starts. yt-dlp gets `--add-header Referer:…`, `--add-header User-Agent:…` and `--cookies`, when resolving and downloading. gallery-dl gets `--user-agent` and `--cookies`, unless the download's own options already set a cookies file / cookies-from-browser or a user agent (`GalleryDlOptions::is_set`); no referer (extractors send the one their site wants).
- **Entry points:** `LinkGrabber::add_with_request(urls, source, request)` (checking uses the request; confirming copies it onto the downloads) and `NewDownload.request`. Nothing in the app passes a request yet; the pipe server (2b) will.
- **Tests:** unit tests for matching, cleaning, the header, the file format and its removal; a manager test against a `/private` route (403 without cookie and referer) that also checks a redirect to `localhost` arrives without the cookie and that `request_json` is cleared on completion; a gallery-dl test where the fake tool proves it got the cookie file both when resolving and when downloading through LinkGrabber → Manager.

### How 2b works

- **`rdl-bridge`** (new crate, no dependency on `rdl-core`, so the native host stays small): `protocol` (messages), `frame` (little-endian `u32` length + UTF-8 JSON, exactly native messaging's framing, at most 4 MiB), `pipe` (server and client, Windows only).
- **Protocol v1:** every message is `{"v":1, "id":<optional u64, echoed>, "type":…}`.
  - `ping` → `pong {app, version, protocol}`; `status` → `status {active, queued, grabber}`.
  - `add {links, target: "grabber"|"queue", referer?, userAgent?, cookies?, filename?, package?}` → `added {target, added, duplicates, rejected}`. Cookies are `chrome.cookies.Cookie` objects as the browser returns them (`expirationDate` in fractional seconds, `session`).
  - At most 1,000 links per `add`. Unknown fields are ignored, so the extension can send more than the app reads. A wrong `v`, an unknown `type`, bad JSON or no links get an `error {message}` reply and the connection stays open; a bad length gets an error reply, then the connection is closed (the stream can't be resynced).
- **Pipe:** `\\.\pipe\rusty-downloader-<USERNAME>` (per user, so two signed-in users each reach their own app). Bound with `first_pipe_instance` (if anything else holds the name, the app logs an error and runs without the bridge) and `reject_remote_clients`. Up to 8 connections at once, any number of requests per connection, closed after 60 s idle. Logs record the request type, never cookies. `pipe::connect` waits up to 5 s while the pipe is busy; `NotFound` means the app isn't running (2c will start it then).
- **App** (`src-tauri/src/bridge.rs`): started from `setup`; the bind happens inside the async runtime (binding outside it panicked at start, found in the app). Links are checked again (http/https only; others come back in `rejected`) and the request context goes through `RequestContext::cleaned()`.
  - `grabber`: `LinkGrabber::add_with_request` with the new `LinkSource::Browser` ("1 link added from the browser" in the app). When the window isn't in front, a Windows notification "N links from the browser" with a Review button that opens the LinkGrabber.
  - `queue`: one package (the given name, else the usual default) in the default download folder, each download carrying the request context; the browser's `filename` is used only for a single link.
- **Dev tool:** `node tools/dev/pipe.mjs '{"type":"ping"}' …` sends messages to the running app and prints the replies (`--raw` sends text as is).
- **Checked in the app (2026-10-25):** ping and status; an `add` with a cookie, referer and a `javascript:` link (rejected) landed in the LinkGrabber and was checked online, and its `request_json` held the cleaned context (browser-only fields like `sameSite` dropped); the same link again came back as a duplicate; a `queue` add downloaded `report-from-browser.bin` into the chosen folder and its cookies were cleared on completion; unknown type, non-http links and bad JSON got error replies; the in-app toast and the Windows notification ("1 link from the browser", read back from the toast history) both showed. Not checked: clicking Review (notification buttons can't be driven). Cleaned up afterwards: settings back to defaults, test package, history entry, LinkGrabber links and notifications removed.

### How 2c works

- **`rdl-native-host`** (`crates/rdl-native-host`, binary `rdl-native-host.exe`, built next to `rusty-downloader.exe`): the browser starts it when the extension connects. It reads native-messaging frames from stdin, relays each one to the pipe unchanged, and writes the app's reply to stdout; it exits when the browser closes stdin. stdout is the protocol, so diagnostics go to stderr. Release builds have no settings at all; debug builds read `RDL_HOST_PIPE`, `RDL_HOST_APP` and `RDL_HOST_LAUNCH_WAIT_MS`, for the tests.
- **What it reads of a message:** only `id` (to answer in the app's place) and `type`. Only an `add` may start the app: the popup's `ping`/`status` must not launch it, they get `error` with `code: "notRunning"`.
- **Error codes** (new optional `code` on `error` replies in the protocol): `notRunning` (app not running, or started but not answering within 20 s), `untrusted` (the pipe isn't the app's; nothing was sent), `unavailable` (connected but no proper answer, 30 s per reply, or a reply over native messaging's 1 MB limit to the browser).
- **Trust check** (`trust.rs`, one of two modules allowed `unsafe`): after connecting and before writing anything, `GetNamedPipeServerProcessId` → `OpenProcess` → `QueryFullProcessImageNameW` must name `rusty-downloader.exe` in the host's own folder (paths resolved, case ignored), and the server's token user must equal the host's (`EqualSid`). This closes the pipe-squatting gap from 2b: a squatter gets nothing. Checked on every (re)connect.
- **Starting the app** (`launch.rs`, the other `unsafe` module): `rusty-downloader.exe --minimized`, detached, in a new process group, and out of the browser's job (`CREATE_BREAKAWAY_FROM_JOB`, retried without it if the job forbids that). First the host's own std handles are made non-inheritable: Rust's `Command` hands every inheritable handle to the child, and the app then held the browser's stdout pipe open, so the browser would wait forever for the host's output to end (found in the app; the launch test now checks stdout closes). Then it polls the pipe every 200 ms for up to 20 s.
- **Connection:** one per browser session, reused. If writing to it fails (the app closed it after 60 s idle, or restarted), it reconnects and checks again. A message is never sent twice, so a lost reply can't add links twice.
- **Checked against the real app (2026-10-25)**, with a script playing the browser and the debug binaries side by side: with the app off, `ping` answered `notRunning` without starting it; `add` started it in the tray (0.6 s warm, 4.9 s on the first run), passed the trust check, and the link reached the LinkGrabber with a "1 link from the browser" notification; a second host session reached the running app. Cleaned up afterwards.
- **Not done here:** browsers can't find the host until 2e registers it (`HKCU\…\NativeMessagingHosts`).

### How 2d works

- **Layout:** `extension/src/` holds one set of sources, plain ES modules with no bundler: `background.js` (worker), `native.js` (talking to the host), `rules.js` (pure helpers, unit-tested by `extension/test/` with `node --test`), `popup.*`, `options.*`, `style.css`. `node extension/build.mjs` (`pnpm ext:build`) writes `extension/dist/chromium/` and `extension/dist/firefox/` (git-ignored). The Firefox manifest is generated from the Chromium one: no `key`, `background.scripts` instead of a service worker, and `browser_specific_settings.gecko.id = extension@rustydownloader.dev`. Icons are copied from `src-tauri/icons`.
- **Pinned id:** the manifest's `key` is a public key generated for this project (the private key wasn't kept and isn't needed for unpacked installs), so Chromium gives the unpacked extension the id `pnfhclaphbmimlmgkdamhmiaidjcmioh` everywhere. Edge confirmed it. `node extension/build.mjs --check` (in CI) fails if `src-tauri/src/native_host.rs` doesn't allow that id and the Firefox id.
- **Permissions:** `nativeMessaging`, `contextMenus`, `cookies`, `downloads`, `storage`, `notifications`, `scripting`, host access to `<all_urls>` (cookies for any site, links from any page). Firefox grants site access separately; the options page shows an "Allow access to all sites" button when it's missing.
- **Talking to the app** (`native.js`): `connectNative("dev.rustydownloader.host")`, one port per browser session (the host keeps its pipe), closed after 3 idle minutes so the host exits and the worker can sleep; replies matched by `id`, 60 s timeout. A host the browser can't start ("not found", "forbidden") becomes `notInstalled`. The popup uses one-shot `sendNativeMessage` with `status`, which never starts the app.
- **Right-click menus:** *Download with Rusty Downloader* (link), *Send selected links* (anchors the selection touches plus links written as text; falls back to the selection text where scripts can't run), *Send all links on this page*, *Send this page (video or gallery)* (the page URL; the LinkGrabber decides between yt-dlp and gallery-dl). All go to the LinkGrabber with the tab's URL as referer. Only http(s) links, de-duplicated, at most 1,000 per message.
- **Cookies:** read with `cookies.getAll({url})` for one link per site (at most 25 sites per send), from the tab's cookie store (private windows, Firefox containers), merged; the app then sends each cookie only where it matches. `sendCookies` can be switched off.
- **Taking over downloads:** `downloads.onCreated` → `shouldIntercept` (switched on; not a private window; http(s); never-sites win; always-sites; file type list, by default archives, disc images, installers and media; or at least 20 MB). A match is paused at once and sent to the queue with the referer, the browser's user agent and cookies; if the app took it, the browser's copy is cancelled and erased and a notification says so; if not, it's resumed and a notification says why.
  - **Found in Edge:** a small file can be entirely received the moment the download is created (always when the worker has to wake first). Edge then reports the pause as done, finishes the download anyway, and `cancel` succeeds without effect; erasing it from the list hid a second copy in Downloads. Now only a download that is paused *and still missing bytes* (`stillMissingBytes`) is taken over; the others stay with the browser. As a backstop, a download found complete after cancelling stays visible and a "Downloaded twice" notification appears.
- **Popup:** Connected (with the app's counts) / Not running ("It starts by itself when you send something.") / Not set up / Not reachable; the takeover switch and a summary of the rule. **Options:** the switch, file types, minimum size, always/never sites, the cookie switch; saved in `storage.sync` on every change.
- **Test hook:** `globalThis.rustyDownloader.onMenu(info, tab)` in the worker runs a menu item as if clicked (DevTools can't click context menus) and returns the totals.

### How 2e works

- **Registration** (`src-tauri/src/native_host.rs`): on every start, if `rdl-native-host.exe` is next to the app, it writes `%APPDATA%\rusty-downloader\native-host\chromium.json` (`allowed_origins`: the pinned extension) and `firefox.json` (`allowed_extensions`: the Firefox id), and sets the default value of `HKCU\Software\{Google\Chrome, Microsoft\Edge, BraveSoftware\Brave-Browser, Chromium, Mozilla}\NativeMessagingHosts\dev.rustydownloader.host` to the right one. Brave and Chromium only when their vendor key exists. Without the host exe it logs a warning and registers nothing. The last copy of the app to start wins, including a dev build.
- **Building the host:** `pnpm dev` now runs `cargo build -p rdl-native-host` first, so the debug host sits next to the debug app. `pnpm build` uses `src-tauri/tauri.bundle.json`, which builds the release host (`pnpm host:build`) and adds it as a resource mapped to the install folder: the installer puts `rdl-native-host.exe` next to `rusty-downloader.exe` and the uninstaller deletes it (checked in the generated NSIS script and the installer's file list). It's not in the main config, because Tauri's build script would then demand a release host for every `cargo test`.
- **CI:** the check job also runs `pnpm ext:test` and `node extension/build.mjs --check`.
- **Checked end to end in Edge 156 (2026-10-25)**, with a throwaway profile, `--load-extension` (plus `--disable-features=DisableLoadExtensionCommandLineSwitch`) and DevTools on port 9333 (`tools/dev/edge.mjs`), against a local site that logs each request's `Cookie`, `Referer` and `User-Agent`:
  - The registry entries and manifests were written; the extension loaded with the pinned id; `sendNativeMessage` reached the app through the debug host.
  - All links on a page (4; a `mailto:` dropped), a selection (an anchor plus a written-out link), a single link (came back as a duplicate) reached the LinkGrabber; the app's checks of them carried Edge's cookies, the page as referer and Edge's user agent, and so did a tool run's (cookie file).
  - Clicking a `.zip` link: Edge's download was paused, cancelled and erased; the app downloaded an identical file with the cookies and referer.
  - With the app closed, the popup said "Not running" and didn't start it; a download then started it (`--minimized`) and went to the app within 2 s. The app kept running after Edge closed.
  - A never-site kept the download in Edge. The small-file race above was found here and fixed; afterwards a 400 KB file stayed with Edge only and a 100 MB file went to the app only.
  - Cleaned up afterwards: test downloads, LinkGrabber links, history entries, notifications, settings, the test profile, and the test files that reached Downloads.
  - Not checked: Firefox, real sign-in sites, the popup's look (screenshots of the test Edge hung).

### How 3a works

- **`src-tauri/src/keep_awake.rs`** (allowed `unsafe`, `windows-sys` `Win32_System_Power`): `PowerRequest` wraps `PowerCreateRequest` with the reason "Rusty Downloader is downloading", and `PowerSetRequest` / `PowerClearRequest` with `PowerRequestSystemRequired`. Dropping it releases and closes it. Not `SetThreadExecutionState`: that belongs to the calling thread, and async tasks move between threads.
- **When:** a task started from `setup` checks every 2 s: held while the `keep_awake` setting is on and `Manager::running_count()` (new: running download tasks, including ones stopping) is above 0, released otherwise. So a download, a switch flip or a pause takes effect within 2 s; that is far below Windows' idle-sleep timeout. Logged as "keeping the computer awake" / "letting the computer sleep again".
- **What it holds off:** idle sleep only. The screen can still turn off, and closing a laptop lid or choosing Sleep still sleeps.
- **Setting:** `Settings::keep_awake` (default on; older saves get it on), a switch in Settings → Windows.
- **Tests:** the unit test holds a request and checks `CallNtPowerInformation(SystemExecutionState)` reports `ES_SYSTEM_REQUIRED`; when nothing else holds the machine awake it also checks release and drop. On this machine something else already held it, so only holding was checked. `powercfg /requests` would list the request by its reason, but it needs an elevated prompt.
- **Checked in the app (2026-10-25)** with a throttled 40 MB download: held when it started, released when the switch was turned off (clicked in the UI), held again when turned on, released when the download was paused, each within 2 s. Cleaned up afterwards.

### How 3b works

- **Limiter** (`rdl-core::ratelimit`): `RateLimiter` is a token bucket (holds a quarter second of its rate, at least 16 KiB; `MIN_RATE` 1 KiB/s) whose rate can change at any time (`None` = unlimited). Taking more than is there puts it in debt and the taker sleeps until it's paid, so many connections sharing one bucket still add up to the rate. A `Throttle` is a list of buckets taken in turn. Own code rather than `governor` (§3.2): its quota can't change while in use.
- **Engine:** `SegmentConfig.throttle`; every received chunk is taken from it after it's written, in both the segmented and the single-stream path, and a pause still interrupts the wait. Found by the test: skipping each segment's last chunk let a quarter of the bytes through free, since segments are split small.
- **Manager:** a global bucket (`set_speed_limit`, `speed_limit`) and one per running download (`set_download_speed_limit`, stored in `downloads.speed_limit`, migration 7, applied to a running download at once, reported as `Event::SpeedLimitChanged`; `Download.speed_limit` for the UI). Plain downloads pass through both. yt-dlp and gallery-dl get `--limit-rate` when they start: the download's own limit, or its share of the global one (divided by the downloads running then) if smaller. Fixed for the run, so approximate. gallery-dl's own `limit-rate` option wins.
- **When which limit** (`rdl-core::speed`): `Settings.speed_limit_on` (the switch) with `speed_limit` (default 2 MiB/s) wins; otherwise every `speed_schedule` rule (days 0 = Monday, `HH:MM` from/to, limit) whose window covers now applies and the lowest counts. A window that doesn't end after it starts runs overnight and belongs to its start day (`00:00–00:00` is the whole day). At most 20 rules, checked on save. Local time via `chrono`.
- **App** (`src-tauri/src/speed.rs`): settings changes apply the limit at once (`AppState::apply`); a task re-checks every 15 s, so scheduled limits start and end on time, and sends `SpeedLimitStatus` to the UI when what's in force changes. Commands `speed_limit_status`, `set_speed_limit_on`, `set_download_speed_limit`.
- **UI:** the status bar's "No speed limit" is now a button (gauge icon): "Limited to 2.00 MB/s" (switch), "Limited to 1.00 MB/s until 18:00" (schedule), with a hint; clicking flips the switch. Settings has a Speed card: the switch with its MB/s value, and the scheduled limits (day buttons, from/to time fields, MB/s, Remove, "Add a scheduled limit"). A download's right-click menu has "Speed limit…" (a dialog in MB/s, comma or dot; empty removes it), and the details panel says "limited to …".
- **Tests:** the limiter's rate with 1 and 8 takers, chunks bigger than the bucket, rate changes, two buckets in a row (paused tokio time); schedule windows (day, overnight, whole day), switch vs rules, validation; a manager test timing a 4 MiB download under a 2 MiB/s global limit and then without it; a per-download limit set, holding, and lifted mid-download; `--limit-rate` in both tools' arguments.
- **Checked in the app (2026-10-25)** with a 200 MB local file: the status bar button turned the limit on and the download ran at 2.01 MB/s; "Speed limit…" from the right-click menu with "0,5" slowed it to 515 KB/s at once and the details panel showed "limited to 512 KB/s" (the dialog first did nothing: a number input bound by Svelte gives a number, which broke `.trim()`; it's a text field now); switching off and adding a rule for today in Settings made the status bar say "Limited to 1.00 MB/s until 18:00"; removing it gave "No speed limit". A screenshot caught the rule's time fields stretching to full width; fixed. Cleaned up afterwards.

### How 3c works

- **Model** (`rdl-core::manager::queues`): `Queue` (for the page: settings plus `open` and counts of packages, active, queued, paused), `QueueSchedule` (`enabled`, `days` 0 = Monday, `start`/`stop` `HH:MM`, stored in `queues.schedule_json`; disabled means always open), `OnComplete` (`none`/`sleep`/`hibernate`/`shutdown`/`exit`, stored in `queues.on_complete_action`), `QueueChanges` (validated: name 1–60 characters and unique, 1–20 at once, valid times, at least one day). Day/time windows are shared with speed rules in `rdl-core::timewindow` (overnight windows belong to their start day).
- **Manager:** `list_queues`, `add_queue`, `update_queue`, `remove_queue` (not Main; its packages move to the end of Main), `move_package_to_queue` (to the end of the queue; running downloads move with it). `Event::QueuesChanged`, `Event::QueueFinished { queue_id, on_complete }`.
- **Scheduler:** each pass (on wake-ups and every 5 s) checks each queue's window with local time. Closed: nothing starts, and running downloads are stopped with the new `StopReason::ScheduleClosed`, which leaves them `queued` with their progress kept; a download already being paused or removed is left alone. Open: free slots are filled as before.
- **Finished:** a queue reports `QueueFinished` once when a download in it ended (completed *or failed*) since it last finished and nothing in it is active, queued or paused. So pausing everything, removing downloads or an empty queue never sets it off, while a night of broken links still ends in the shutdown. (First written as "completed" only; caught while writing these notes, since failures would have kept an overnight queue from ever finishing.)
- **App** (`src-tauri/src/on_complete.rs`): on `QueueFinished` with an action, a 60 s countdown (one at a time): the window comes to the front with a banner ("Night has finished. Shutting down in 42 s." + Cancel), and a Windows notification has a Cancel button too. Then: `exit` closes the app; `shutdown` saves the downloads' state (`Manager::shutdown`) and runs `shutdown /s /t 0`; `sleep`/`hibernate` call `SetSuspendState` (the only `unsafe` here), and say so in a notification if Windows refuses (hibernation off). Debug builds only: `RDL_ON_COMPLETE_SECONDS` shortens the countdown and `RDL_ON_COMPLETE_DRY_RUN` logs sleep/hibernate/shutdown instead of doing them. Commands: `list_queues`, `add_queue`, `update_queue` (for Main, keeps Settings' "Downloads at once" the same), `remove_queue`, `move_package_to_queue` (its argument is `packageId`: `package` is a reserved word in JavaScript), `on_complete_pending`, `cancel_on_complete`; event `OnCompleteCountdown`.
- **UI:** the Queues page (one card per queue: name, "Running" / "Waits until 01:00", counts, downloads at once, "Only run at set times" with day buttons and start/stop, "When finished", Remove; "Add queue" at the bottom); "Move to <queue>" in the package menu when there are other queues; "Queue" in a package's details when there's more than one.
- **Tests:** schedules open/close and validation; manager tests for adding, renaming, unique names, moving a package in and back out when its queue is removed; a queue outside its window waits, starts when it opens and stops (requeued, progress kept) when it closes; a queue reports finishing once with its action, again after a download fails, and not while a download is paused.
- **Checked in the app (2026-10-25)**, started with the debug countdown at 12 s and dry run on: a queue added on the Queues page, given a closed window and "Shut down" through its controls ("Waits until 17:05"); a package moved there through its right-click menu ("Move to Night") waited; opening the window ran it; the banner showed "Night has finished. Shutting down in 10 s.", the notification "Night: Shut down in 12 s" with Cancel, and Cancel in the app stopped it (no shutdown). Then "Sleep" ran out its countdown (logged as a dry run), and "Close Rusty Downloader" closed the app for real, saving state. Cleaned up afterwards.

### How 3d works

- **`rdl-core::checksum`:** `ChecksumAlgo` (`md5`/`sha1`/`sha256`/`crc32`, the values the first migration's `downloads.checksum_algo` CHECK allows), `of_file` (1 MiB reads on a blocking thread, stops when its token fires), `of_bytes`, `parse_value` (the first hex word of a known length, of the chosen algorithm's length if one is given, so `SHA256: AB12…`, `ab12… *file.iso` and a bare value all work), `parse_list` (md5sum style `hash *name`, BSD `SHA256 (name) = hash`, SFV `name HASH`; `;`/`#` comments, a BOM, folders in names dropped) and `ChecksumAlgo::for_list` (by extension). Crates `md-5`, `sha1` (0.11, the same `digest` as `sha2`), `crc32fast`.
- **Storage:** `checksum_algo` / `checksum_value` (lowercase hex) existed; migration 8 adds `checksum_ok` (NULL not checked, 1 matched, 0 didn't). `Download.checksum` is `{algo, value, verified}`; `Event::ChecksumChanged` carries it.
- **Manager** (`manager/checksums.rs`): `set_checksum(id, algo, text)`, empty text clears; refused for galleries (many files). After a plain or yt-dlp download finishes (`job::start`, before `finalize`) its file is hashed against the value read fresh from the DB, so one set while it runs counts. Hashing is cancellable; a pause during it lets the download complete unchecked. A mismatch is `Error::ChecksumMismatch` ("checksum mismatch for beta.bin: expected …, got …"): the download fails, the file stays, the segments are cleared and `downloaded` stays at the full size. A retry deletes that file first (`checksum_ok = 0`) and downloads again.
  - Setting a value on a download whose file is there (completed, or failed on its checksum) checks it in the background: a completed one that doesn't match fails, a failed one that now matches completes (with history), without downloading again. A result is only recorded if the value didn't change while hashing.
  - **Lists:** when a plain download finishes as `.sfv` / `.md5` / `.sha1` / `.sha256` (at most 1 MiB), each other download in its package with no checksum yet whose saved name, given name or link's last part matches an entry (case ignored) gets that value. Finished ones are checked at once, the rest when they finish.
- **Race fixed on the way:** a pause arriving between the scheduler picking a queued download and marking it active was lost, and the download ran anyway (the flaky "paused queue" test, which also failed on the code before 3d). Now the job is registered as running before it's marked active, `mark_active` only moves `queued` → `active`, and `pause` asks the job to stop if moving it out of `queued` failed. That test then passed 25 runs in a row (about 1 in 8 failed before).
- **UI:** "Checksum…" in a download's right-click menu (not for galleries): algorithm ("Recognise it" or one of the four) and a text field; the command's error shows in the dialog. The details panel shows "CRC32 68ba51c9 ✓ Matches" / "✗ Doesn't match" / "Not checked yet".
- **Tests:** known vectors per algorithm, file hashing and cancel, pasted values, SFV and md5sum/BSD lists; manager tests: a value set mid-download is checked at the end; a mismatch fails and keeps the file, a retry downloads again, the right value then completes it with no new request, and a wrong value fails a completed one; a `.sfv` in the package checks the file that finished before it and fails the one after it.
- **Checked in the app (2026-11-01)** with three files from a local server: the package's `sums.sfv` gave `alpha.bin` "✓ Matches" (it had finished first) and failed `beta.bin` ("expected deadbeef"); right-click → "Checksum (CRC32)…" showed the value; "Recognise it" with `MD5: 25C6…` pasted completed `beta.bin` with "✓ Matches" without fetching it again; a wrong SHA-1 failed the completed `alpha.bin`, Retry fetched it again and it failed again; a short SHA-256 got "That isn't a SHA-256 checksum: expected 64 hex digits.". Screenshots caught the algorithm drop-down on a line of its own (a global `.field` rule) and a failed download showing "0 B of 6.00 MB"; both fixed. Cleaned up afterwards (package, files, history).

### How 4a works

- **`rdl-core::proxy`:** `ProxyKind` (`http`/`https`/`socks5`; SOCKS5 is sent as `socks5h`, so names are looked up at the proxy), `ProxyProfile` (what the UI sees: never the password, only `has_password`), `ProxyInput` (validated: name 1–60 and unique, host without scheme, port or `user@`, a colon only in an IPv6 address; `password: None` keeps the stored one, `""` removes it), `ProxyChoice` (`auto` / `direct` / `profile {id}`, stored as `downloads.proxy` NULL / `direct` / id, migration 9), `ProxyRule` and `ProxySettings` (rules in order, first match wins, then the default; at most 200; cleaned to lowercase without `https://` or a trailing `/`). `host_matches`: `example.com` is that host and its subdomains (not `badexample.com`), with `*` the whole host must match. Storage uses the first migration's `proxy_profiles` / `proxy_rules` tables; the default is the `proxy.default` setting. `proxy_url` builds `scheme://user:pass@host:port` with the login percent-escaped.
- **`rdl-core::secrets`:** a `Secrets` trait with `CredentialManager` (via `keyring-core` + `windows-native-keyring-store`; entries appear as `proxy/<id>.rusty-downloader`, generic credentials) and `MemorySecrets` (tests, or when Credential Manager can't be opened, logged as an error). 4c's site logins will go here too. The real-store test is `#[ignore]` (CI's account may have no store); it passed here.
- **`rdl-core::network::Network`:** loads profiles, passwords and rules; `route(url, choice)`; `client(url, choice)` (redirects left to `http::get`) and `api_client(url)` (redirects followed), one cached `reqwest` client per proxy URL, dropped whenever profiles or rules change; `tool_proxy` for the tools; `test(url, choice)` (any HTTP answer is success except 407, and failures carry reqwest's cause chain, e.g. "SOCKS error: credentials not accepted"). `Network::direct` is the no-database default, so `ManagerConfig.network`, `ToolsConfig.network` and `LinkGrabber::start`'s new second argument (it replaced the unused user agent) can stay `None` in tests.
- **Where it applies:** plain downloads (probe and segments, by the download's choice), LinkGrabber checks (rules), yt-dlp and gallery-dl resolving (rules) and downloading (the download's choice) via `--proxy` (a gallery's own `proxy` option wins), and tool update checks and downloads (rules). `NewDownload.proxy` can carry a choice from the start; `Manager::set_download_proxy` changes it (event `ProxyChanged`) for the next start.
- **App:** `AppState.network`; commands `proxy_overview`, `proxy_add`, `proxy_update`, `proxy_remove`, `proxy_set_settings`, `proxy_test` (default page `http://www.msftconnecttest.com/connecttest.txt`, Windows' own connectivity check), `set_download_proxy`.
- **UI:** Settings → Network (`NetworkSettings.svelte`): proxies with Test / Edit / Remove and the test result inline; "Add a proxy" dialog (name, type, server and port, user, password; the port follows the type until typed; an existing password shows as "Saved; type to replace it" with a remove box); "Which connection": rules (site, directly / through a proxy, move up/down, remove) and "Everything else goes". A download's right-click menu has "Connection…" (follow the rules, directly, or a proxy) and the details panel shows a non-automatic choice.
- **LinkGrabber race fixed on the way:** a finished check wrote `online` and then, in a second statement, the split-archive package name, so a reader could see an online `movie.part2.rar` still in the old package (the `split_archives_are_grouped_by_base_name` test failed that way under load). Both are one `UPDATE` now.
- **Tests:** unit tests for input checks, host patterns, rule order, settings cleaning, URL escaping, choice storage, memory secrets; integration tests (`tests/network.rs`) with local SOCKS5 and HTTP proxies that check logins and count tunnels (`tests/common/proxy.rs`): passwords never in the DB and kept / replaced / removed, removing a profile drops its rules and the default, settings survive a reload, downloads through the default, a direct rule bypassing it, a download's own choice beating the rule, a wrong login failing, the test button (407 reported), and LinkGrabber checks following a rule; yt-dlp / gallery-dl `--proxy` arguments.
- **Checked in the app (2026-11-01)** with a local SOCKS5 proxy that wants a login and logs each tunnel: "Add a proxy" (screenshot) saved it with the password in Credential Manager (`cmdkey /list` showed `proxy/1.rusty-downloader`, gone after Remove); Test said "Works · 2091 ms" and the proxy saw `www.msftconnecttest.com:80` by name; "Everything else goes through Local SOCKS" sent a download from the local file server through it; a rule "127.0.0.1 goes directly" made the next one skip it; a LinkGrabber check of the archive.org video went through it (probe and yt-dlp: two `archive.org:443` tunnels) and found "Big Buck Bunny"; a wrong password from Edit made Test say "SOCKS error: credentials not accepted" and the proxy logged the refused login. A screenshot caught "Everything else goes" out of line with the rules; fixed. Cleaned up afterwards (proxies, rules, downloads, files, the LinkGrabber link, history).

### How 4b works

- **`rdl-core::adapters`:** `list()` via the `ipconfig` crate (every adapter but loopback: Windows id (GUID), name, description, `address`). `usable_address` takes IPv4 first, else a global IPv6, never link-local or loopback; an adapter that isn't up, or has only `169.254.x.x` / `fe80::` (Windows' address for an unconnected adapter; Tailscale shows one while disconnected), has no address and counts as down. `BoundAdapter {id, name}` is what the settings keep (`Settings.bind_adapter`, `Settings.kill_switch`, default on), the name for when the adapter is gone.
- **`Network` binding:** `Binding::Any` / `Bound(ip)` / `Down(name)`, set with `set_binding` (drops the cached clients when it changes). Every client gets `local_address(ip)` while bound, and while down `client`, `api_client` and `tool_connection` fail with `Error::NetworkDown` ("the network adapter for downloads (Tailscale) is down"), so link checks and tool updates can't go around the VPN either. `ToolConnection {proxy, source_address}` replaced 4a's `proxy` on both tools' `RunSpec` and `resolve`: `--proxy` and `--source-address` (gallery-dl's own options still win).
- **Network hold (`Manager::set_network_hold`):** while held, each scheduler pass stops running downloads with `StopReason::NetworkHeld` (requeued, progress kept, like a closing queue window) and starts nothing; no queue counts as finished. Released, they resume by themselves. `restart_running` requeues what runs (new address). `ManagerConfig.hold` starts it held, so nothing can start before the adapter has been checked.
- **App (`src-tauri/src/network_watch.rs`):** `wanted(settings, adapters)` gives the binding, the hold (only with the kill-switch) and a `NetworkStatus` (`any` / `up {adapter, address}` / `down {adapter, held}`); `apply` holds before unbinding and binds before releasing, and restarts running downloads when the address changes. Worked out at start before the manager exists, then every 2 s and at once after a settings change (`AppState.network_check`). Changes go to the UI (`NetworkStatusChanged`) and, going down or coming back, to a Windows notification. Commands `network_adapters`, `network_status`.
- **UI:** Settings → Network starts with "Adapter": a list ("Any connection (Windows decides)" or "Tailscale · not connected" / "WiFi · 192.168.0.124"; a chosen adapter that's gone stays listed as "not found"), Refresh, the state ("Connected: everything goes out from …" / "… is down: nothing connects, and downloads wait for it") and "Pause downloads while it's down". The status bar shows "Via WiFi", or in red "Tailscale is down · downloads paused", linking to Settings.
- **Tests:** usable address choice, the real adapter list, `wanted` for nothing chosen / up / down with and without the kill-switch / gone; network tests: bound to `127.0.0.1` a download works, bound to `192.0.2.1` (not this machine's) it fails, down it fails with the adapter named and the tools are refused; a hold requeues a running download with its progress, keeps it waiting, and on release it finishes without starting over; `restart_running` requeues and it still finishes; both tools' `--source-address`.
- **Checked in the app (2026-11-01):** the adapter list matched `Get-NetAdapter` (WiFi 192.168.0.124; Tailscale, Ethernet and two Wi-Fi Direct adapters not connected). Choosing WiFi showed "Connected: everything goes out from 192.168.0.124" and "Via WiFi" (screenshot); an internet file downloaded through it. With a 100 MB download from `proof.ovh.net` running at 1 MB/s, choosing the down Tailscale requeued it at 11.5 MB and kept it waiting, with "Tailscale is down · downloads paused" in red (screenshot). While bound to WiFi the local test server (`127.0.0.1`) couldn't be reached (see follow-ups). **Not checked:** a real VPN coming up and going down (the user skipped it; Tailscale was disconnected), so the resume, the "is back" / "is down" notifications and a restart on a new address were only exercised by tests. Cleaned up afterwards (adapter back to Any, speed limit off, downloads, files, history).

### How 4c works

- **`rdl-core::accounts`:** an `Account` is a site pattern (`example.com` with its subdomains, or `*` patterns, as in proxy rules), an optional user name, `has_password`, a cookie count and `enabled`. Stored in the first migration's `accounts` table (no login = empty user name, since the column is `NOT NULL`; `UNIQUE (site, username)`); the password in Credential Manager as `account/<id>`; the cookies as JSON sealed with DPAPI for the current Windows user (`Secrets::seal` / `open`, `windows-dpapi` with app entropy; `MemorySecrets` keeps them as is) in `accounts.cookies_sealed` (migration 10; `cookies_json` stays unused). Credential Manager can't hold a cookie jar (about 2.5 KB per entry), hence DPAPI. `AccountInput` validates like proxies: `password: None` keeps, `""` removes; a password needs a user name. `Login` never prints its password.
- **Which account:** the enabled one whose site matches the link's host; the most specific pattern wins (`video.example.com` before `example.com`).
- **Cookies:** `request::parse_netscape` reads `cookies.txt` (tabs, `#HttpOnly_`, `0` / empty expiry = session, BOM, comments; cleaned like every other cookie, so a value with a tab is dropped). Importing keeps only cookies for the account's site (`cookies_for_site`: its domain, subdomains, or a parent domain it receives cookies from) and reports the rest as "for other sites"; new ones replace stored ones with the same name, domain and path. `RequestContext::merge_cookies` adds an account's cookies to a request unless the browser sent the same cookie (fresher). Cookie matching then works per hop as since 2a.
- **Where it applies (`Network::with_account`, inside `Network` so no new plumbing):** plain downloads and LinkGrabber checks (cookies, and the login in `DownloadSpec.login`); yt-dlp and gallery-dl resolving and downloading (cookies into their cookie file, and `ToolConnection.login` as `--username` / `--password`, which both tools spell the same).
- **HTTP Basic:** `http::get` answers a `401` with a `WWW-Authenticate: Basic` challenge from the login's own site by sending it again with the login; that host is remembered for the download (`DownloadSpec.basic_hosts`), so its later requests (segments) carry the login straight away. Never sent unasked and never to another host.
- **App:** commands `account_list`, `account_add`, `account_update`, `account_remove`, `account_import_cookies` (reads the chosen file, at most 16 MB), `account_clear_cookies`.
- **UI:** Settings → Accounts (`AccountsSettings.svelte`, after Network): each account with its user, "(password saved)", cookie count, "not used" when disabled, and Import cookies.txt… (Windows file dialog), Clear cookies, Edit, Remove; the import result inline ("1 cookie added; 1 for other sites left out"). "Add an account" dialog: site, user name, password, "Use this account".
- **Tests fixed on the way:** `queue_limit_caps_concurrent_downloads` (1b) counted active downloads from events and lost a `Completed` when the broadcast lagged under load, so it saw two active; it polls the stored statuses now. 4a's proxy test expected `ProxyChanged` as the very next event while the queue could still report finishing; it waits for it now.
- **Tests:** cookies.txt parsing and round trip, merging (browser wins), account input checks, most specific account wins, cookies kept for their site only, logins only for their site and not printed, DPAPI seal / open and tamper refusal; an integration test against `/members` (wants a cookie) and `/basic` (wants Basic auth): refused without an account, the import keeps 1 of 2 cookies, the download then completes, Basic is refused once per host, tools get the login, other sites get nothing, a disabled account is ignored, cookies are stored sealed and survive a reload, clear and remove (password gone).
- **Checked in the app (2026-11-01)** against a local site wanting cookie `member=yes` on `/members` and Basic `me` / `s3cret` on `/basic`: without an account `/members` failed (403); "Add an account" (screenshot) saved it with the password in Credential Manager (`account/1.rusty-downloader`); the import (called with the file's path, since DevTools can't drive the Windows file dialog) said 1 imported, 1 for other sites (a `.youtube.com` cookie); Retry then sent `member=yes` and completed; `/basic` was refused once, sent again with the login, and the download itself sent it straight away; a non-cookie file got "No cookies found: is it a Netscape cookies.txt?". The Accounts section showed "127.0.0.1 me (password saved) · 1 cookie" (screenshot). Remove deleted the Credential Manager entry. Cleaned up afterwards.

### How 4d works

- **`rdl-core::ftp`:** a `Location` (host, port, path, login, the bound address) from the link; `probe` and `download` give the same `ProbeResult` / `Outcome` / `Progress` as HTTP, so the job, pause, resume and the UI treat both alike. `suppaftp` 12 with its tokio + rustls (aws-lc, as reqwest) features.
- **Links:** `ftp://` is plain; `ftps://` is explicit FTPS (`AUTH TLS`, `PBSZ 0`, `PROT P`), the certificate checked by `rustls-platform-verifier` (the Windows store, as for HTTPS), data connections reusing the control connection's TLS session. Implicit FTPS (port 990) is refused as unsupported. Paths are relative to the login's home folder (RFC 1738; `%2F` at the start makes one absolute); a folder link is refused.
- **Login:** the link's own (`ftp://user:pass@host/…`, percent-decoded) wins; else the host's 4c account (`Network::with_account`); else `anonymous` / `anonymous@`. `Location`'s `Debug` never prints the password.
- **Probe:** connect, (TLS,) log in, `TYPE I`, `SIZE` and `MDTM` (as `last_modified`, so a changed file restarts like HTTP's validators), `QUIT`. `SIZE` refused with `550` = no such file (LinkGrabber: offline); another refusal = the server has no `SIZE`, size unknown.
- **Download:** one connection. The part file is the resume point: with saved progress (and the same size / `MDTM`), `REST <part length>` then `RETR`, appending; otherwise it starts empty. Progress is published as one segment, so the manager saves and restores it as for HTTP. A dropped transfer reconnects and resumes from what was written (up to `max_retries` with backoff, counting only failures without progress); a `REST` refused with 500–504 starts over from 0. Speed limits apply per chunk; pause drops the connection. Timeouts: 15 s to connect, 30 s per reply or read.
- **Network:** proxies don't apply. Control and data connections are made with `TcpSocket` bound to the 4b adapter's address (`Network::local_address`, now public; only addresses of its family are tried), and a down adapter gives `NetworkDown`, which the kill-switch already handles. Passive mode (`EPSV` over IPv6) with the NAT workaround (data goes to the control connection's address, not the one `PASV` names).
- **Errors:** `FtpConnection` (retryable), `FtpReply { code, message }` (4xx retryable, 5xx not; "FTP server replied 530: Login incorrect."), `Ftp` (TLS and protocol trouble).
- **Elsewhere:** `DownloadKind::Ftp` (`downloads.kind = 'ftp'`, already allowed by the schema), chosen by `NewDownload::new` from the scheme, so Add links, the browser bridge and LinkGrabber all route it; `extract_links` (clipboard, LinkGrabber) and the Add links dialog find `ftp(s)://`; removing an FTP download removes its part file; a finished FTP file can be a checksum list.
- **Tests:** location parsing and logins, the one-segment progress, reply mapping; against a local FTP server (`tests/common/ftp.rs`): probe + anonymous download, `MDTM`, missing file, link / account login and the link winning, a transfer cut half-way resuming with `REST` without fetching anything twice, pause and resume in the engine and through the manager (`REST` at the part file's length), a server refusing `REST` starting over, LinkGrabber online / offline with kind `ftp`.
- **Checked in the app (2026-11-01):** LinkGrabber with `ftp://ftp.gnu.org/gnu/emacs/emacs-29.4.tar.xz` (online, 52,210,344 bytes), `ftps://demo:password@test.rebex.net/readme.txt` (explicit FTPS with the Windows store: online, 379 bytes) and a missing GNU file (offline, "FTP server replied 550: Could not get file size."); both downloaded (screenshot). A second copy of the 52 MB file at 2 MB/s, paused at 803,600 bytes (part file the same size), resumed unthrottled and completed with the same SHA-256 as the first. `speedtest.tele2.net` was unreachable for curl too. Cleaned up afterwards.

### How 4e works

- **`rdl-core::sftp`:** a `Location` from the link; `probe` (`stat`: size, `mtime` as `last_modified`; a folder is refused) and `download` give the HTTP types like FTP. `russh` 0.65 (aws-lc, as reqwest) and `russh-sftp` 3.
- **Links:** `sftp://[user[:password]@]host[:port]/path`. The path is absolute; `/~/` at its start means the home folder. Port 22 by default.
- **Login:** the user from the link, else the host's 4c account, else the Windows user name (as OpenSSH). The password is the link's, or the account's when the account is for the same user. The password is tried first, then the unencrypted `id_ed25519`, `id_ecdsa`, `id_rsa` in `%USERPROFILE%\.ssh` (RSA with the best hash the server takes). Refused = `SshLogin` (not retried). `Location`'s `Debug` never prints the password.
- **Host keys (`KnownHosts`):** trust on first use. The first key a `host:port` shows is stored in `known_hosts` (migration 11: host, port, algorithm, `SHA256:` fingerprint, added_at); the same key passes; another is refused with `HostKeyChanged` (old and new fingerprints, and where to forget the old one), before any login is sent. Host certificates are refused. Without a database (tests, `Network::direct`) the store is in memory. `Network::known_hosts()` hands it out.
- **Download:** one SSH connection; open the file, seek to the part file's length, read on. The read / retry / resume loop FTP had is now `rdl-core::single` (`Single`: start, copy, retry, end; plus the bound `tcp` / `tcp_to`), shared by both; FTP's own `REST`-refused fallback stays in `ftp.rs`. Keepalive every 15 s; 30 s per request.
- **Errors:** `SshConnection` (retryable: I/O, disconnects, timeouts, a read cut off), `SshLogin`, `HostKeyChanged`, `SftpStatus { code, message }` (code 2, no such file = LinkGrabber offline), `Sftp` (protocol trouble).
- **Elsewhere:** `DownloadKind::Sftp` (`downloads.kind = 'sftp'`, already allowed) and `DownloadKind::is_plain()`; LinkGrabber checks `sftp://` over SFTP; `extract_links`, Add links and `parse_links` accept it. Commands `known_hosts_list`, `known_host_forget`; Settings → Known SFTP hosts (after Accounts) lists host, algorithm and fingerprint with Forget.
- **rustc note:** an iterator over the key paths held across an `.await` in the login made every task that runs a download fail `Send` ("not general enough", reported at unrelated code). The paths are collected into a `Vec` first.
- **Tests:** link parsing, logins (link, account for the same user only, another site's account ignored), trust on first use / change / forget; against a local SFTP server (`tests/common/sftp.rs`, built on `russh::server` and `russh_sftp::server`, throwaway keys): probe and password download, the host key stored, missing file and folder, no login / wrong password refused, key login with a `/~/` path and a wrong password falling back to the key, a changed host key refused before logging in and accepted after forgetting, a read cut off half-way resuming from the written length, pause and resume, LinkGrabber online / offline with kind `sftp` and the manager with the account's login.
- **Checked in the app (2026-11-02)** against `test.rebex.net` (`demo` / `password`): `readme.txt` and `pub/example/KeyGenerator.png` online (379 and 36,672 bytes), a missing file offline ("SFTP server: File not found. (status 2)"), a wrong password "refused the login"; Rebex's `ssh-ed25519 SHA256:d7Te2DHmvBNSWJNBWik2KbDTjmWtYHe2bvXTMM9lVg4` stored on first use. Both downloaded; the PNG has the same SHA-256 as curl's copy. Settings → Known SFTP hosts showed it (screenshot); Forget emptied the list. No `.ssh` folder on this machine, so no keys were offered. Cleaned up afterwards.

### Release 1.0

- **Version 1.0.0** everywhere: the workspace (`Cargo.toml`, so the app, the installer and `rdl_core::VERSION`), the path dependencies, `ui/package.json` and the extension's manifest.
- **Installer** (`src-tauri/tauri.conf.json`, NSIS): `installMode: "both"`, so it asks between this user (`%LOCALAPPDATA%\Rusty Downloader`, no admin) and everyone (`Program Files`, admin); the license page (`LICENSE`); publisher "Gert Steyn"; the app icon; `windows/sidebar.bmp` (164x314, welcome and finish pages) and `windows/header.bmp` (150x57), drawn from the icon by `tools/dev/installer_images.py` (Pillow; rerun after an icon change). Tauri's finish page already has "Run Rusty Downloader" and "Create desktop shortcut".
- **Uninstall** (`windows/hooks.nsh`, `NSIS_HOOK_PREUNINSTALL`, skipped for updates): closes the app, then runs `rusty-downloader.exe --uninstall [--delete-data]` (`src-tauri/src/uninstall.rs`, checked first thing in `run()`, before Tauri starts). It removes, for the current user: the browser-host keys that point at this install's manifests (a portable copy's stay), `Software\Classes\AppUserModelId\dev.rustydownloader.app`, and the `StartupApproved\Run` mark. With "Delete the application data" ticked it also deletes every stored password (`network::forget_secrets`: `proxy/<id>` and `account/<id>` from the database's ids) and `%APPDATA%\rusty-downloader`; downloaded files are never touched. Tauri's own uninstaller removes the Run value, the shortcuts and `%LOCALAPPDATA%\dev.rustydownloader.app` (the web view). A portable copy ignores `--uninstall`.
- **Portable** (`pnpm portable` after `pnpm build`, `tools/release/portable.ps1`): `target\release\bundle\portable\Rusty Downloader_<version>_x64_portable.zip` with a `Rusty Downloader` folder: the app, `rdl-native-host.exe`, `LICENSE` and `portable.flag` (data in `data\` beside them, as `rdl-core::paths` already did). The main window is now created in code (`create: false` in the config, `create_main_window` in `lib.rs`) so a portable copy puts the web view's data in `data\webview` too. Zip entries are written one by one with `/` (Windows PowerShell's `Compress-Archive` and `ZipFile` write backslashes).
- **Release** (`.github/workflows/release.yml`): pushing a tag `vX.Y.Z` that matches `Cargo.toml` builds the installer, the portable zip and both extension zips (Chromium, Firefox) and publishes a GitHub Release with generated notes. Tests aren't rerun: tag a commit whose CI passed. The repo is private, so downloading needs a GitHub login. CI's installer job also builds and uploads the portable zip.
- **Not done (decided 2026-11-02):** no auto-update (needs a public place for update files, see Open Decisions) and no code signing (SmartScreen warns once: "More info → Run anyway").
- **Scheduler race fixed on the way:** a scheduler pass read the queue limits, and if a limit changed while the pass was still running (under load), it started downloads by the old one: three at once with a limit of 1. This was what failed `checksum_lists_in_the_package_are_used` (and, earlier, `queue_limit_caps_concurrent_downloads`), not the cause `10d5ef3` assumed. A pass now holds `Inner::scheduling` throughout, and `set_queue_limit` / `update_queue` take it too. `queue_limit_caps_concurrent_downloads` reads all statuses in one query, and `a_downloads_own_limit_changes_while_it_runs` uses the slow route so it can't finish before its limit lands; the manager suite then passed 60 runs with four copies at once.
- **Release build warnings:** `BINDINGS_PATH`, `typescript()` and their import are only compiled into debug and test builds.
- **Checked (2026-11-02):** `pnpm build` (7.8 MB installer) and `pnpm portable`. The portable copy wrote everything, `webview` included, under its `data` folder. The installer was installed for all users by hand: `C:\Program Files\Rusty Downloader` with the app, the host and `uninstall.exe`, Add/Remove Programs shows 1.0.0 by Gert Steyn, the Start menu shortcut is in the all-users folder, and the installed app registered the browser host for the user. The installer's pages weren't captured (screenshots of its window came out blank) and the uninstall wasn't run yet.

### Resume here

*Last session ended 2026-11-02, right after releasing 1.0.0. Read this first.*

- **Repo state:** `main` is pushed and CI is green (see "Tests" above). Tag `v1.0.0` is on `2cb15dd`; the release is at https://github.com/Gert-Steyn/rusty-downloader/releases/tag/v1.0.0 (private repo, so downloading needs a GitHub login; GitHub turns the spaces in asset names into dots). The working tree is clean. `gh` is signed in, so CI can be checked from here (`gh run list`, `gh run view --log-failed`).
- **Start here, in this order:**
  1. **Known bug (found 2026-11-02, in 1.0):** a LinkGrabber link confirmed while it's still being checked goes in as a plain HTTP download (its kind is still the default `http`), so a gallery or video page downloads as its HTML page. Seen with a gallery-dl page of 519 sub-galleries whose check took about 40 s: confirmed after 8 s, it saved the 71 KB page. Fix: don't confirm links that are still `checking` (or let them finish checking first), and/or resolve the kind again when such a download starts.
  2. **Planned feature (asked 2026-11-02): gallery subfolders switch.** gallery-dl downloads always get `-D <folder>` (`gallerydl::RunSpec::args`), so every file, sub-galleries included, lands in one flat folder. Add a gallery option, "Keep the site's folders", that passes the package folder as a base folder instead (`-d`, or `-o base-directory=…`), so gallery-dl creates its own subfolders (one per sub-gallery). Its default layout starts with the site name (`<site>\<gallery>\…`), which is redundant inside a named package folder, so the switch should also set a shorter `directory` pattern (just the gallery level), overridable in the options. Removing the download, the file count and the "partial files" cleanup then have to look through subfolders. A flat folder also has a catch worth noting in the UI: gallery-dl skips a file whose name already exists, so two sub-galleries with the same file names (`001.jpg`) keep only the first unless a filename pattern makes them unique.
  3. **Rerun a gallery from the right-click menu** (built 2026-11-06, not yet run in the app). A completed gallery's menu has "Check for new files", and a package with completed galleries has "Check galleries for new files". `Manager::rerun_gallery` (`store::restart_gallery`) queues the download again with its counts reset and its total unknown; it runs into the same `dest_path`, so gallery-dl skips what's on disk and in the site's archive. Tests: `rerunning_a_completed_gallery_fetches_into_the_same_folder` and `rerun_leaves_a_download_that_is_not_completed_alone`. Not built yet: a notification only when something new arrived, a History entry for the rerun (History is keyed on the URL, so the rerun already updates the same row), and a rerun on a schedule. Cookies from the first run are gone (`request_json` is cleared on completion), so only an account's login applies (4c).
  4. **Plan Phase 5, torrents** (see the phase table): librqbit, magnet / `.torrent` associations, seeding limits. Agree a split into milestones with the user first, as for Phase 4. Fixes to 1.0 can ship as 1.0.x (see "Making a release" below).
- **Owed checks by hand** (none block the work above):
  - **Uninstall 1.0.0:** the installed copy (all users, `C:\Program Files\Rusty Downloader`) hasn't been uninstalled yet. Leave "Delete the application data" **unticked**: installed and dev builds share `%APPDATA%\rusty-downloader` (tools, history, settings). Afterwards the browser-host keys and the notification registration should be gone (a `pnpm dev` start puts them back for the dev build).
  - **Installer pages:** screenshots of the installer window came out blank, so the sidebar / header images, the license page and the this-user / everyone page have only been seen by the user.
  - **4b with a real VPN:** connect Tailscale (with an exit node, so internet traffic routes through it; it then has a `100.x` address), choose it in Settings → Network, start a download, disconnect (held, notification), reconnect (resumes, notification). Tailscale is installed but was disconnected.
  - **Phase 3 exit:** an overnight queue that ends in a real shutdown (see "How 3c works"). Then Phase 3 is done.
- **Making a release:** bump the version in the workspace `Cargo.toml` (and the `version = "…"` of the path dependencies in `src-tauri/Cargo.toml` and `crates/rdl-native-host/Cargo.toml`), `ui/package.json` and `extension/src/manifest.json`; commit and push; when CI is green, `git tag -a vX.Y.Z -m "Rusty Downloader X.Y.Z"` and `git push origin vX.Y.Z`. `release.yml` refuses a tag that doesn't match `Cargo.toml`. Locally: `pnpm build` then `pnpm portable` (installer in `target\release\bundle\nsis\`, zip in `target\release\bundle\portable\`; the release build takes about 10 minutes).
- **Categories (2026-11-06; checked in the app: a same-name link takes the folder, confirming it joins the existing package, and moving downloads between packages works, including removing the emptied one):** one name (ignoring case) is one category with one folder. `linkgrabber::category_folder` is the single lookup: a link that joins a name takes its folder, whether the category is still in the LinkGrabber or already confirmed into a package. `confirm` merges a group into an existing package with the same name and folder instead of creating a second one. Changing a category's folder moves every link with its name. Downloads page: right-click a download that hasn't started (or is paused/failed) → "Move to <package>" or "Move to new package…"; a running one can't move. A download that already planned its file keeps that file's folder. Checked by tests only (the LinkGrabber's move menu was checked in the app before this change). Not there: choosing a category or folder when adding links, and moving several checked links at once.
- **Fixes since 4e worth knowing:** `b016ac2` trims debug info in dev builds (`[profile.dev] debug = "line-tables-only"`, none for dependencies): full debug info had grown `target` to 164 GB; it's about 3 GB after a full test build now, and `cargo clean` now and then still helps. `10d5ef3` moved the checksum-list read before the list download is marked done, on a wrong theory; the real cause of that flaky test was the scheduler race fixed in `091d0b4` (see "Release 1.0").
- **Loose ends worth a session of their own:** Firefox run of the extension and a real sign-in download (Phase 2 above); a real sleep / hibernate / shutdown from a queue (3c follow-ups); yt-dlp and gallery-dl under `--limit-rate` (3b follow-ups); clicking the tray menu and notification buttons ("Not verified" below).
- **Smart App Control:** it had switched itself on (enforced) and from 2026-11-01 08:54 blocked locally built proc-macro DLLs and build scripts (os error 4551, Code Integrity events 3077/3033 naming e.g. `schemars_derive-*.dll`), so clippy and then the app's own build failed. The user turned it off the same day; clippy and `pnpm dev` work again. If builds fail with "can't find crate" for a crate that is plainly in `target`, check the `Microsoft-Windows-CodeIntegrity/Operational` log first.
- **CI actions** moved off Node 20 (2026-11-05, `1273642`): `checkout` and `setup-node` @v7, `pnpm/action-setup` @v6 (still installs the pnpm 10 from `packageManager`), `upload-artifact` @v7. CI passed with no annotations; `release.yml` uses the same versions but hasn't run since.
- **Your machine:** Rusty Downloader 1.0.0 is installed for all users and not running. Tools in `%APPDATA%\rusty-downloader\tools` (updated by the app): yt-dlp 2026.10.26, gallery-dl 1.32.19, ffmpeg N-128280 (the previous versions are kept for rollback). The DB is at schema `20261104000011` with only the Main queue, and empty downloads, LinkGrabber, history, proxies, accounts and known hosts. The browser host is registered for this user and its manifests point at the installed copy's host; the last copy started wins, so `pnpm dev` points them at `target\debug` until the installed app starts again. Microsoft Store Python redirects `%APPDATA%`: to read the real database from Python, copy `rusty-downloader.db` and its `-wal` out first.
- **Sanity check before starting:** `cargo test --workspace` (238 pass), `pnpm ext:test` (9), `pnpm ui:check`, then `pnpm dev`. The `test-runner` agent runs all of CI's checks and reports only failures.
- **Haiku agents** (`.claude/agents/`): `test-runner` (CI's checks), `ci-checker` (GitHub Actions runs, read-only), `code-scout` (finds code, returns file:line pointers). Use them for mechanical work; keep design work on the main model.
- **Optional, real network:** `RDL_LIVE_DIR=<some folder> cargo test -p rdl-core --test tools_live -- --ignored --nocapture` (about 135 MB of tools on the first run, reused after).

### Phase 2 – browser integration (done)

2a request context, 2b pipe server, 2c native host, 2d extension, 2e registration; see "How 2a works" to "How 2e works". Still owed: a run in Firefox (load `extension/dist/firefox/manifest.json` from `about:debugging`, grant site access in the options page, then repeat the Edge checks) and a download from a real site that needs sign-in.

### Phase 3 – control (built; exit check owed)

1. ~~**Keep-awake**~~ done as 3a (see "How 3a works").
2. ~~**Speed limits**~~ done as 3b (see "How 3b works").
3. ~~**Scheduler and Queues page**~~ done as 3c (see "How 3c works"). The exit criterion (overnight queue, then shutdown) was checked with shutdown as a dry run; a real shutdown hasn't been run.
4. ~~**Checksums**~~ done as 3d (see "How 3d works").

### Deferred / known gaps (pick up in the milestone noted)

- **Not verified:**
  - The Mica window background has never been seen. It can't be captured by DevTools screenshots.
  - Tray menu items and notification buttons were never clicked (no automation for either); the code paths they call are the same commands the UI uses.
- **Desktop (1g follow-ups):**
  - Clipboard "ask first" offers only inside the app; while the window is hidden there's no Windows notification with an Add button.
  - Notification buttons work while the app runs; clicking an old notification after the app has quit does nothing (needs a COM activator).
  - "Connections per download" applies to downloads that start after the change; running ones keep theirs.
  - History records the URL as given; two links to the same file under different URLs aren't recognised as duplicates (plan §2.3 also wants path + size + hash).
- **HTTP engine:**
  - `Retry-After` on 429 isn't honoured (429 uses normal backoff).
  - Expired links (`LinkExpired`) need plugin refresh (Phase 6).
- **Request context (2a follow-ups):**
  - Not tried against a real site that needs sign-in yet; that needs the extension (2d). The cookie file format was checked against what yt-dlp's and gallery-dl's loaders accept, not by a live run.
  - Cookies sit in the SQLite DB in plain text until the download completes (same protection as the rest of `%APPDATA%`). DPAPI encryption would be the next step if that matters.
  - No UI shows that a link carries browser cookies. A `cookies.txt` per site can now be imported into an account (4c).
  - The yt-dlp cookie file is written back by yt-dlp at exit; harmless, since it's deleted right after.
- **Browser pipe (2b follow-ups):**
  - ~~**Pipe squatting**~~: handled by the host's trust check (2c). The app itself still can't serve the pipe when a squatter took the name first; it logs an error, and the host reports `untrusted`. Telling the user in the app would be better.
  - The pipe keeps Windows' default security (read for everyone, full access for the owner, SYSTEM and administrators). Other users can't send requests, but an explicit DACL for the current user only would be tighter.
  - Any program the user runs can send links, including to the queue, which starts downloading without asking. That's no more than such a program could do itself, but a "confirm downloads from the browser" setting may be wanted.
  - No forcing of a handler (yt-dlp / gallery-dl / plain) from the browser yet; "send page to yt-dlp / gallery-dl" needs it (it's also a 1f gap).
  - `status` lists every package and LinkGrabber link to count them; fine for now, slow with very large lists.
  - The app has no Settings switch to turn the bridge off.
- **Native host (2c follow-ups):**
  - The trust check trusts any `rusty-downloader.exe` in the host's folder; it doesn't check a signature (there is none yet, see Authenticode in Open Decisions).
  - A portable copy has its own host next to it; whichever copy's host the browser registration points at is the one the browser uses (2e should register the copy that does the registering).
  - The host's stderr goes to the browser's log only when the browser runs with logging on; there's no host log file.
- **Extension and registration (2d/2e follow-ups):**
  - **Firefox not run yet** (see Phase 2 above). Its manifest is generated and the APIs used exist there, but nothing was tried live.
  - Small downloads that the browser has fully received when the takeover happens stay in the browser (by design, see "How 2d works"). Chromium's `downloads.onDeterminingFilename` could hold them earlier, at the cost of delaying every download.
  - No modifier key to keep a download in the browser: the downloads API can't see keys. The popup switch and the never-sites list do that job.
  - A download that came from a form POST is fetched again with GET by the app, which may fail.
  - "Send this page" doesn't force yt-dlp or gallery-dl; the LinkGrabber decides (also a 1f gap).
  - Chromium's `filename` is empty when a download is created, so the app names files from the server; Firefox passes the browser's name.
  - Partitioned (CHIPS) cookies and Firefox's first-party isolation aren't handled.
  - Not packaged for any store: Edge/Chrome need developer mode and *Load unpacked* (`extension/dist/chromium`), and branded Chrome 137+ ignores `--load-extension` unless the `DisableLoadExtensionCommandLineSwitch` feature is turned off.
  - The registry entries stay after uninstalling (no NSIS uninstall hook yet); they point at a host that's gone, which browsers report as "not found". A dev build overwrites an installed copy's registration until the installed app starts again.
  - No in-app status for the browser bridge (registered? extension seen?) and no switch to turn it off.
  - Not tried under a real browser yet (needs 2d and 2e), so the job breakaway and the stdin/stdout handling are only tested against a script.
- **SFTP (4e follow-ups):**
  - No SSH agent (Pageant, the Windows OpenSSH agent) and no encrypted keys: only unencrypted key files in `%USERPROFILE%\.ssh` are tried. No choosing a key per account.
  - With no password, your keys are offered to any SFTP server a link names (as OpenSSH does by default).
  - Host keys come from the app's own store, not `~/.ssh/known_hosts`; the first connection to a server trusts whatever it shows.
  - One connection per file, no segmented SFTP; folder links are refused.
  - Keyboard-interactive logins (some servers' password prompts) aren't tried, only the `password` method.
- **FTP (4d follow-ups):**
  - One connection per file; no segmented FTP (several `REST` + `RETR` connections), which servers often limit per user anyway.
  - Implicit FTPS (port 990) isn't supported: `suppaftp`'s implicit connect can't be bound to an adapter.
  - Folder links (`ftp://host/pub/`) are refused; no listing a folder into LinkGrabber.
  - Active mode isn't offered; passive only.
  - A server without `SIZE` gives an unknown size, so a cut-off transfer can't be told from a finished one (as for HTTP without a length).
  - No accepting a self-signed FTPS certificate; such servers fail with the TLS error.
- **Accounts (4c follow-ups):**
  - The login goes to yt-dlp and gallery-dl on the command line, where other programs of the same user (and administrators) can read it in the process list. A `.netrc` written for the run, like the cookie file, would avoid that.
  - Only HTTP Basic is answered; Digest, forms and hoster logins are plugins' work (Phase 6). Basic over plain `http://` sends the password readably, as the site asked.
  - Cookies set by the site during a download aren't stored back into the account, so an account's cookies age; re-import them, or let the extension send fresh ones.
  - One account per site in use (the most specific); no choice per download.
  - The import button needs the Windows file dialog, which automation can't drive: the in-app check called the command with the path. No drag-and-drop of a cookies.txt.
  - Cookie values are never shown, so there's no way to look at or edit single cookies.
- **Adapter binding (4b follow-ups):**
  - **Not checked with a real VPN going up and down** (see "Resume here").
  - Strict on purpose: while bound, loopback (`127.0.0.1`) and LAN addresses the adapter doesn't route can't be reached, so a NAS or a local server is out of reach through a VPN binding. A "local network bypasses the VPN" switch would need care with redirects (a local link redirecting outward must not go unbound).
  - Down is checked every 2 s (no Windows change notifications), so a download can lose up to 2 s of connection attempts before it's held; those segments retry or fail as usual.
  - Without the kill-switch, a download that starts while the adapter is down fails with "is down" and has to be retried by hand.
  - Up with an address isn't the same as working: an adapter that's connected but routes nothing (Tailscale without an exit node, for internet sites) makes downloads fail rather than wait.
  - Hidden virtual adapters ("Local Area Connection* 1", Wi-Fi Direct) are listed too.
  - One adapter for everything; no per-site or per-queue binding.
- **Proxies (4a follow-ups):**
  - The proxy is picked by the download's own link; redirects to another host stay on that connection (a rule for the CDN host doesn't apply mid-redirect).
  - Running downloads keep the connection they started with; yt-dlp and gallery-dl get theirs when they start.
  - No Windows system proxy or PAC file is read, and nothing is imported from the browser. Direct means reqwest's default, which still honours `HTTP_PROXY`-style environment variables, as do the tools.
  - Only SOCKS5 and an HTTP proxy were tried for real; an `https://` proxy (TLS to the proxy) is untested.
  - LinkGrabber links and browser downloads always start as "Automatic"; packages and queues have no connection setting of their own.
  - A failed test shows reqwest's whole cause chain, which is long ("error sending request for url (…): client error (Connect): …").
  - If Credential Manager can't be opened, passwords are kept in memory only and saved ones can't be read; the log says so but the UI doesn't.
- **Checksums (3d follow-ups):**
  - Galleries get no checksums, and lists only apply inside their own package. A list whose entries match nothing is ignored silently.
  - No "checking…" state: a big file sits at 100% while it's hashed. A pause during hashing completes it unchecked.
  - A checksum can't come with a link (browser, LinkGrabber, `NewDownload`); it's set afterwards.
  - Lists with other names (`SHA256SUMS`, `.txt`) aren't recognised; only the four extensions are.
  - yt-dlp downloads are checked against the final file after post-processing, which rarely has a published hash.
- **Queues (3c follow-ups):**
  - Shutdown, sleep and hibernate were only run as dry runs (and their commands are untested); only "close the app" ran for real. Hibernate fails when hibernation is switched off (`powercfg /h on`).
  - Nothing wakes the computer for a queue's window: if it's asleep at the start time, the queue starts when it wakes. A wake timer (`SetWaitableTimer` with resume) would need its own setting.
  - The LinkGrabber and Add links always use Main; a package goes elsewhere afterwards, from its menu. No per-queue speed limit (`queues.speed_limit_bps` is still unused).
  - Queues can't be reordered (they run side by side anyway), and the Downloads page doesn't group or filter by queue.
- **Speed limits (3b follow-ups):**
  - yt-dlp's and gallery-dl's `--limit-rate` were only checked in their arguments, not in a real run. Their share of the global limit is set when they start and doesn't follow later changes, and their bytes don't count against the global bucket, so plain downloads running alongside them aren't slowed to make room.
  - Packages and queues have no limits of their own (the `queues.speed_limit_bps` column is still unused; 3c may want it).
  - The schedule lives in the settings JSON, not in the unused `speed_profiles` table (drop the table when it's next touched).
  - The status bar rounds to two decimals ("2.00 MB/s"), and a limit's MB/s field doesn't accept KB.
- **Keep-awake (3a follow-ups):**
  - Releasing the request couldn't be confirmed from outside on this machine (something else kept it awake, and `powercfg /requests` needs admin); the app's log and the unit test's drop path are the evidence.
  - Only running downloads count. A queue waiting for its scheduled start (3c) or the LinkGrabber checking links doesn't keep the machine awake.
  - No option to keep the screen on, and nothing wakes the machine from sleep for a scheduled start (3c may want a wake timer).
- **Windows 10:** the transparent window shows the desktop through it, because Mica is Windows 11 only. A fallback is needed if Windows 10 ever matters.
- **Tool and plugin kinds:** `http`, `ytdlp` and `gallerydl` downloads run; the rest later.
- **gallery-dl (1f follow-ups):**
  - Rechecking a link whose handler changes (yt-dlp ↔ gallery-dl) keeps the earlier tool's file and package name.
  - Sites that rate-limit (Wikimedia's 429) fail the download; gallery-dl's own `--sleep-429` isn't set by default.
  - Resolving uses default settings plus the link's browser cookies (2a), so a gallery that needs sign-in works when it comes from the browser, but not when its own options (a cookies file) would be the fix.
  - Thumbnails are stored (`Gallery.thumbnails`) but not shown: the CSP only allows local images, and many sites block hotlinking.
  - No TLS-fingerprint impersonation: gallery-dl 1.32.17's `--help` has no impersonation flag.
  - No way to force a link to a handler (yt-dlp / gallery-dl / plain) from the LinkGrabber.
  - No "resolve-only" mode (gallery-dl `-g` URLs into the HTTP engine), plan §5.3.
  - Archive files are per host, not per gallery-dl category.
  - Speed is averaged over finished files (5 s window), so it is jumpy for galleries of big files.
  - The nightly install from `gdl-org/builds` was never run for real (its `digest` field was confirmed).
- **yt-dlp (1e follow-ups):**
  - Cookies are passed when a link carries them (2a), but nothing sends links with cookies until the extension exists; sites that demand sign-in (YouTube from some IPs, Vimeo) still fail for pasted links.
  - No subtitles, thumbnails, SponsorBlock, rate-limit or raw-args options yet (§5.2). The `YtDlpOptions` struct is where they go.
  - No progress during post-processing (merge or audio conversion). The row sits at 100% until it finishes.
  - No "use my own yt-dlp.exe" override.
  - Downloads table "Saved to" for a running yt-dlp row says "name not known yet" until the list reloads, because the UI store only gets the filename from the `resolved` event (also true for HTTP).
  - Playlists expand flat: entries have no formats, so they get every preset and no size estimate until downloaded.

### Implementation notes & gotchas

- **Pinned versions:**
  - `tauri-specta =2.0.0-rc.26`, `specta =2.0.0-rc.26` and `specta-typescript =0.0.13` must move together.
  - Tauri plugins stay on **2.x**, because their latest releases are 3.0 alphas.
- **Bindings:** `ui/src/lib/bindings.ts` is generated. `pnpm dev` rewrites it, or run `cargo test -p rusty-downloader -- --ignored regenerate_bindings`. CI fails if it's stale.
- **Test binaries:** `src-tauri/build.rs` embeds the Common Controls v6 manifest for every target. Without it, test binaries linking Tauri crash with `STATUS_ENTRYPOINT_NOT_FOUND`.
- **sqlx 0.9:** it rejects SQL built at runtime. Use static strings, or `AssertSqlSafe` only for SQL built from our own constants.
- **tauri-specta reserved names:** a type called `Channel` (and probably other Tauri runtime names) fails export with "must be renamed". That's why the tools enum is `ToolChannel`.
- **SQLite ids are reused:** tables use `INTEGER PRIMARY KEY` without `AUTOINCREMENT`, so after deleting the newest row its id comes back. History is keyed on a URL hash for this reason; never key anything long-lived on a download id.
- **Clippy `large_enum_variant`:** `GrabberEvent::Updated` holds a `Box<GrabbedLink>` for this reason; serialisation is unchanged.
- **Svelte 5:** in `$effect`, anything read becomes a dependency. Wrap reset logic in `untrack` (this caused the "folder field wipes links" bug in 1c).
- **Edge and Chrome when testing the extension:**
  - After a page has started one download, further downloads it starts from script are blocked silently ("multiple automatic downloads"). Navigate the tab instead (`cdp:Page.navigate`, or `chrome.tabs.update`).
  - Screenshots of a background tab hang; bring it to the front first (they still hung in the test Edge, so read pages as text).
  - The extension's worker stays alive while DevTools is attached; `cdp:ServiceWorker.enable ;; ServiceWorker.stopAllWorkers` on a page target stops it, to test waking up.
  - `msedge.exe --version` doesn't print a version on Windows; it opens the user's own Edge. Read the version from the exe's file properties instead.
- **Windows PowerShell 5.1:**
  - Commit messages with embedded `"` get split. Write the message to a file and use `git commit -F`.
  - `Invoke-RestMethod` doesn't unroll JSON arrays.
- **Toolchain on this machine:**
  - Rust 1.99 (MSVC), Node 24, pnpm 10, Git, VS 2022 Build Tools.
  - A new shell may need PATH refreshed from the registry.
  - Git identity is set repo-locally. GitHub auth goes through Git Credential Manager.

### How to test in the real app

The native window can't be clicked from automation, but WebView2 exposes Chrome DevTools:

1. **Start a local file server.** Any HTTP server with Range support works. The one used so far served 200 MiB of deterministic bytes, throttled to ~3 MB/s per connection, at `http://127.0.0.1:8765/<name>`.
2. **Launch the app with DevTools enabled:**
   ```powershell
   $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9222 --remote-allow-origins=*'
   pnpm dev
   ```
   (`*` is needed for Node's built-in WebSocket client.)
3. **Drive it over CDP.** `node tools/dev/cdp.mjs eval "<js>"` and `node tools/dev/cdp.mjs shot out.png` do the following for you. Under the hood, connect a WebSocket to the page listed at `http://127.0.0.1:9222/json` and send:
   - `Runtime.evaluate` to click and type in the real UI, or call `window.__TAURI_INTERNALS__.invoke(...)`
   - `Page.captureScreenshot` for screenshots
   
   Delete `document.documentElement.dataset.shell` first, so screenshots get a solid background instead of transparent Mica.
   - From Git Bash, a Windows folder passed inside an `eval` string loses its backslashes. Use forward slashes (`C:/Users/...`).
   - Navigate with the app's own links (`document.querySelector('a[href="/tools"]').click()`); `history.pushState` doesn't route.
4. **Video tests:** use the archive.org and SoundCloud URLs above. YouTube and Vimeo want a sign-in from this machine.
5. **Clean up.** Remove test packages with `remove_package(id, deleteFiles: true)`, and point test folders somewhere disposable, not your Downloads folder.
6. **Clipboard tests use the real Windows clipboard.** Save it first and restore it afterwards. Restoring while watching is on re-adds any link in your own clipboard text.
7. **Browser pipe:** `node tools/dev/pipe.mjs '{"type":"add","links":["http://127.0.0.1:8765/x"],"target":"queue"}'` plays the native host. Set a disposable download folder first: `queue` adds go to the default folder.
8. **Reading the DB while the app runs:** another process sees only the state from before the app started (its WAL changes aren't visible from outside). Copy `rusty-downloader.db` and `-wal` somewhere else and read the copy, after closing the app.
9. **Stopping `pnpm dev`** from a background shell can leave Vite on port 1420, and the next start fails with "Port 1420 is already in use". Stop that `node … vite dev` process first.
10. **Extension in Edge:** `pnpm ext:build`, then start Edge with a throwaway profile (`--user-data-dir=<temp folder>`), `--load-extension=<repo>\extension\dist\chromium`, `--disable-features=DisableLoadExtensionCommandLineSwitch`, `--remote-debugging-port=9333` and `--no-first-run`. Drive it with `node tools/dev/edge.mjs <target> "<js>"` (see its header). The app must be running from a build with the host next to it (`pnpm dev` does that). A download Edge keeps lands in your real Downloads folder even with a throwaway profile, so delete test files there afterwards. Close the test Edge with `Browser.close` over DevTools.

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
| External tools | `yt-dlp.exe`, `gallery-dl.exe`, `ffmpeg.exe`, `7z.exe`, downloaded on first use and auto-updated by the app |

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
- Explorer context menu "Download links from file with Rusty Downloader" for `.txt` link lists. This uses the classic registry verb, which shows under *Show more options* on Windows 11. A modern `IExplorerCommand` menu needs package identity, so it's deferred.

---

## 3. Architecture

```
┌──────────────────────── Tauri app (src-tauri) ────────────────────────┐
│  Svelte UI (WebView2)  ◄── typed commands/events (tauri-specta) ──►   │
│                                                                        │
│  App layer: commands, event batching (≤10 Hz), tray, notifications,   │
│             clipboard watcher, single-instance, deep links             │
│                               │                                        │
│  ┌────────────────────── rdl-core (no Tauri dep) ──────────────────┐ │
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
             │ named pipe \\.\pipe\rusty-downloader       │ HTTP/WS (optional)
   ┌─────────┴─────────┐                 ┌───────┴────────┐
   │ rdl-native-host  │◄─ stdio ─ Browser extension (MV3) │ rdl-remote (axum) │
   └───────────────────┘                 └────────────────┘
```

### 3.1 Repository layout (Cargo workspace + pnpm)
```
rusty-downloader/
├─ Cargo.toml                  # workspace
├─ crates/
│  ├─ rdl-bridge/             # named-pipe protocol shared by the app and the native host
│  ├─ rdl-core/               # engine, storage, handlers, scheduler
│  ├─ rdl-plugins/            # Rhai host + plugin API
│  ├─ rdl-native-host/        # browser native-messaging bridge exe
│  └─ rdl-remote/             # axum server for remote control
├─ src-tauri/                  # Tauri shell, commands, tray, OS integration
├─ ui/                         # SvelteKit (adapter-static, SPA, ssr=false)
├─ extension/                  # MV3 extension (Chromium + Firefox manifests)
├─ plugins/                    # bundled .rhai hoster plugins
├─ tools/                      # dev and build scripts (tools/dev/cdp.mjs drives the running app)
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
2. **Allocate**: create `<name>.rdl.part` and pre-size it with `set_len`. Segments write with positional writes (`FileExt::seek_write`), so no locking is needed between segments.
3. **Segments**: start with N (default 8, min segment 1 MiB). **Dynamic splitting** (IDM-style): when a connection finishes, split the largest remaining segment in half and start a new connection on it, until segments reach the minimum size.
4. **Persistence**: segment offsets are flushed to SQLite every ~2 s and on pause/exit. A small sidecar `.rdl.meta` file allows recovery if the DB is lost.
5. **Resume validation**: resume uses `If-Range` with the ETag/Last-Modified. If the server returns 200 instead of 206, or the size/ETag changed, the user chooses restart or keep.
6. **Non-range servers**: a single connection. Resume isn't possible, and the UI says so.
7. **Expiring links**: on 403/410 mid-download, ask the resolver/plugin to **refresh** the URL, then continue at the same offsets.
8. **Finish**: flush and fsync, verify the length, run the checksum if one is known, then rename atomically to the final name. Collisions follow the setting: rename `(1)`, overwrite, skip, or ask.
9. **Headers**: per-job custom headers, referer, user agent, cookies (from the extension or jar), and Basic/Digest auth.

---

## 5. yt-dlp & gallery-dl Integration

### 5.1 Tool management
- Downloaded on first use into `<data>/tools/` (so `<portable>/data/tools/` for portable builds). The installer may later seed the same folder.
- **Tool updater**: checks the releases of `yt-dlp/yt-dlp` (GitHub) and `mikf/gallery-dl` (Codeberg since 2026) on startup (at most daily) and on demand. Downloads into a temp folder, verifies the published checksums (yt-dlp `SHA2-256SUMS`), and swaps atomically. Keeps the previous version for rollback. Channel per tool: stable / nightly.
- `ffmpeg.exe` (downloaded on demand, see §0) is used for yt-dlp merges and gallery-dl ugoira conversion. Its folder is passed explicitly (`--ffmpeg-location`).
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

- Plugins are `.rhai` files in `plugins/` (bundled) and `%APPDATA%\rusty-downloader\plugins\` (user). They hot-reload on change.
- Manifest header: `name`, `version`, `url_patterns` (regex), `needs_account`, `max_connections`, `min_wait`.
- Entry points: `check(url)` → availability/size/name, `resolve(url, account)` → direct links + headers + optional hash, `login(account)` → session cookies, and `refresh(url)` for expired links.
- Host API (sandboxed, no filesystem or process access): `http.get/post` (shares the cookie jar and proxy), `html.select`, `regex`, `json`, `sleep`, `log`, `wait_countdown(secs)` (shown in the UI), `open_browser(url)` for manual captcha.
- Limits: max operations, timeout per call, memory cap.
- Resolution order: user plugin → bundled plugin → yt-dlp (if `--simulate` succeeds) → gallery-dl (if an extractor matches) → plain HTTP probe.

---

## 7. Browser Integration

- **Extension (Manifest V3)**, Chromium (Chrome/Edge/Brave) plus a Firefox manifest variant:
  - Intercepts downloads (`chrome.downloads.onCreated`) that match rules: min size, extensions, an always/never site list, and a modifier key to bypass. It cancels the browser download and sends URL, referer, filename, cookies (`chrome.cookies.getAll` for that URL) and user agent.
  - Context menus: *Download with Rusty Downloader*, *Send all links on page to LinkGrabber*, *Send selection links*, *Send page to gallery-dl / yt-dlp*.
  - A popup shows connection status and lets you pause interception.
- **Native messaging host** (`rdl-native-host.exe`): registered under `HKCU\Software\Google\Chrome\NativeMessagingHosts`, `...\Microsoft\Edge\...` and `...\Mozilla\NativeMessagingHosts` by the installer. Relays JSON over a named pipe to the app and starts the app if it isn't running.
- **Clipboard monitoring** (built in 1d): polls `GetClipboardSequenceNumber` every 400 ms, a cheap counter read, and only reads the clipboard when it changes. This needs no hidden window or message loop. Whatever is on the clipboard at startup is ignored. Links go straight to the LinkGrabber with a toast, and there's an on/off switch (default on). Still to come: configurable URL patterns, and an "ask first" mode.
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

Locations: installed builds use `%APPDATA%\rusty-downloader\` (db, logs, plugins, archives, cookies). Portable builds use `.\data\`, detected by a `portable.flag` file next to the exe.

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

- **Build**: `pnpm build` produces the NSIS installer and `pnpm portable` the **portable zip** (exe + host + `portable.flag`; tools download on first use). See "Release 1.0".
- **Installer extras**: native-host registry entries, `magnet:` handler, `.torrent` association, optional autostart, WebView2 bootstrapper.
- **App updates**: `tauri-plugin-updater` with a minisign key (kept in GitHub secrets) and `latest.json` on GitHub Releases. Because the repo is private, the update endpoint needs a public release mirror or a token-free proxy (see Open Decisions).
- **Authenticode signing**: optional for personal use. Without it, expect SmartScreen warnings.
- **CI (GitHub Actions, windows-latest)**: `cargo fmt --check`, `clippy -D warnings`, `cargo test`, `pnpm check`, extension tests, plus installer and portable artefacts on every push. On tag `v*` (`release.yml`): build and publish the release (no updater bundle yet).

---

## 11. Testing Strategy

- **Unit**: segment split math, filename parsing (Content-Disposition, sanitisation of Windows-reserved names and lengths), retry policy, scheduler time logic, gallery-dl/yt-dlp output parsers (fixtures of real output).
- **Integration**: an in-process `axum` test server that simulates range support, no range, slow/throttled transfers, dropped connections mid-segment, changed ETag, redirects, 429 with `Retry-After`, expiring URLs (403 then refresh), and huge sparse files.
- **Tool tests** (opt-in, network): smoke runs of the bundled yt-dlp/gallery-dl against a few stable public URLs.
- **UI**: Vitest for stores/components. A Playwright smoke test against the static UI with a mocked IPC layer.
- **Soak**: 500 queued downloads, pause/resume storms, kill the app mid-download, then confirm resume works.

---

## 12. Roadmap

| Phase | Scope | Exit criteria | Status |
|---|---|---|---|
| **0. Scaffold** | Workspace, Tauri + SvelteKit shell, SQLite migrations, tracing logs, CI, Fluent theme base | App launches, CI green | ✅ Done |
| **1. MVP** | Segmented HTTP engine (resume, retries, dynamic splitting, headers/cookies/auth), queues + concurrency, packages, LinkGrabber (paste/clipboard/drag-drop), yt-dlp + gallery-dl (resolve, download, progress, common flags, impersonation, archives) + tool updater, tray, toasts, history/duplicate detection, single instance, settings | Used as daily driver for HTTP, video and galleries | ✅ Done |
| **2. Browser** | MV3 extension (Chromium + Firefox), native host, download interception, context menus, cookie hand-off | Browser downloads flow into the app with cookies | ✅ Done in Edge (Firefox not run) |
| **3. Control** | Scheduler, speed limiting + profiles, keep-awake, on-complete actions, checksum verification | Overnight queue then shutdown works | ✅ Built (real shutdown not run yet) |
| **4. Network & protocols** | Proxies + rules, interface binding + kill-switch, accounts in the keyring, FTP/FTPS, SFTP | Downloads via SOCKS5/VPN interface, FTP/SFTP resume | ✅ built (4a–4e, see Phase 4 milestones); the VPN check by hand is owed |
| **5. Torrent** | librqbit integration, magnet/.torrent associations, seeding limits (ratio/time) | Torrent download + seed from the same UI | ⏳ |
| **6. Plugins & extract** | Rhai plugin host + API, 2–3 reference plugins, auto-extract with password lists | A hoster resolved via plugin, archive auto-extracted | ⏳ |
| **7. Remote & release polish** | axum remote UI/API, portable build, auto-update pipeline, installer polish | Install, auto-update and phone control work end to end | ⏳ Installer, portable build and releases done in 1.0; remote and auto-update left |

### Phase 4 milestones

Agreed 2026-11-01. Proxies come first because they bring the "which connection does this request use" plumbing and the keyring that 4b and 4c reuse; FTP and SFTP use 4c's accounts.

| # | Milestone | Scope | Status |
|---|---|---|---|
| 4a | Proxies | HTTP / HTTPS / SOCKS5 profiles (password in Windows Credential Manager), host-pattern rules to a profile or direct, a global default and a per-download override. Used by downloads, LinkGrabber checks and tool updates; `--proxy` for yt-dlp and gallery-dl. Network page in Settings. | ✅ `fa6a8b2` |
| 4b | Interface binding + kill-switch | Bind to a chosen adapter (e.g. the VPN's), `--source-address` for the tools; the kill-switch pauses everything when that adapter goes down or its IP changes, and resumes when it's back. | ✅ `bb5c13c` (real VPN check owed) |
| 4c | Accounts & cookies | Site logins in the keyring, `cookies.txt` import per site (merged with the browser's cookies), logins handed to yt-dlp / gallery-dl and used for HTTP basic auth. | ✅ `fa1f137` |
| 4d | FTP / FTPS | `suppaftp`: size probe, `REST` resume, credentials from the link or an account; LinkGrabber accepts `ftp://`. | ✅ `c731b94` |
| 4e | SFTP | `russh` + `russh-sftp`: password or key login, trust-on-first-use host keys, resume from an offset. | ✅ `2eaf0fb` |

### Phase 1 milestones

Each milestone is a commit or series of commits that leaves the app working.

| # | Milestone | Scope | Status |
|---|---|---|---|
| 1a | HTTP engine | Headless, in `rdl-core`: probe, filename handling, segmented download with dynamic splitting, pause/resume from segment state, `If-Range` validation, retries with backoff. Tested against a local test server. | ✅ `fe0f510` |
| 1b | Download manager | Packages, downloads and queues persisted in SQLite. Per-queue concurrency, start/pause/remove, segment state saved every ~2 s, resume after restart, event stream. | ✅ `f2f4992` |
| 1c | Downloads UI | Typed IPC (tauri-specta), batched progress events, virtualised downloads table with package rows, add-links dialog, details panel. | ✅ `bea4e42` |
| 1d | LinkGrabber | Paste / clipboard watcher / drag-drop, background probing, availability badges, package grouping, confirm to queue. | ✅ `adba9eb` |
| 1e | Tool manager + yt-dlp | Tool layout, version check/update with checksum verification, yt-dlp resolve (`-J`), format picker, JSON progress parsing. | ✅ `462a74b` |
| 1f | gallery-dl | Resolve with `-j`, native download with stdout progress, flag schema + `--help` self-check, managed download archives, impersonation options. | ✅ `ba04aa8` |

| 1g | Desktop polish | Tray icon/menu, toasts, history + duplicate warnings, settings page (folders, connections, clipboard, startup), right-click menu and drag-to-reorder. | ✅ `804ae23` |

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

1. **Updater hosting**: separate public releases repo vs. private repo + token. Recommended: public releases-only repo. Left out of 1.0 (2026-11-02): no auto-update yet.
2. **Rhai vs Lua (`mlua`)**: Rhai is chosen for its pure-Rust build and sandboxing. Lua is the fallback if plugin performance or familiarity becomes an issue.
3. **gallery-dl default mode**: native download (recommended) vs resolve-only via the HTTP engine, per site.
4. **Default concurrency**: 3 active × 8 segments. Tune after real-world testing.
5. **Authenticode certificate**: skip for personal use, or buy one (or use Azure Trusted Signing) if it's ever distributed. 1.0 shipped unsigned (2026-11-02).

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
- Name (decided after Phase 0): **Rusty Downloader**, repo `Gert-Steyn/rusty-downloader`, crate prefix `rdl-`. Renamed from the working name `win-pydl`.
