# Rusty Downloader — Project Plan

A full-featured download manager for Windows, modeled on **JDownloader**. It has a Rust engine, a Tauri 2 + SvelteKit UI, and built-in **yt-dlp** and **gallery-dl** support.

> Naming: the product is **Rusty Downloader** (binary, package, data folder: `rusty-downloader`; app identifier `dev.rustydownloader.app`). Internal crates use the short prefix `rdl-` (`rdl-core`, later `rdl-plugins`, `rdl-native-host`, `rdl-remote`). Repo: `Gert-Steyn/rusty-downloader`.

---

## 0. Current Status (updated 2026-10-25)

**Where we are:** Phases 0, 1 and **2 (browser integration, 2a–2e) are done**; only a live Firefox run is still owed (see the 2d/2e follow-ups). **Next up: Phase 3 (scheduler, speed limits, keep-awake, on-complete actions, checksums).**

The app works as an HTTP(S), video and gallery download manager that lives in the tray. You can paste, drop or copy links into the LinkGrabber (optionally "ask first", with ignore patterns), review and rename them, pick a video format or gallery options, and start them. Links you downloaded before are flagged. Plain files download segmented and resumable. Pages from sites yt-dlp supports are recognised automatically, and playlists expand into one link per video. Pages yt-dlp doesn't know (or can't read) go to gallery-dl, and a gallery becomes one download into its own folder. yt-dlp, gallery-dl and ffmpeg install themselves on first use. Finished downloads go into History; Windows notifications report finished packages and failures. A browser extension (Edge, Chrome, Brave; Firefox untested) sends links, selections, pages and matching downloads to the app with the browser's cookies, starting the app if needed. Settings cover the download folder, limits, clipboard, start with Windows and the tray. Everything survives an app restart.

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
| 2d – Extension | (uncommitted) | MV3 extension (`extension/`, plain JS, Chromium + generated Firefox build): right-click menus, download takeover by type/size/site rules, cookies per link site, popup with status and switch, options page; pinned id; unit tests in CI |
| 2e – Registration | (uncommitted) | App registers the host for Chrome, Edge, Brave, Chromium and Firefox on every start; `pnpm dev` builds the host, `pnpm build` bundles it next to the app |

Tests: 167 Rust tests and 9 extension tests (`pnpm ext:test`) (unit tests; engine, manager and LinkGrabber suites against a local misbehaving HTTP server; tool-manager suite against fake GitHub and Codeberg APIs; gallery-dl suite against a fake `gallery-dl.cmd`; pipe suite against real named pipes; native-host suite driving the real host binary over stdin/stdout; app tests). They include a check that `ui/src/lib/bindings.ts` is up to date. Five opt-in network tests in `crates/rdl-core/tests/tools_live.rs` install the real tools, pause/resume a real yt-dlp download, check the gallery-dl flag schema against the real `--help`, and resolve and download a real gallery twice (`RDL_LIVE_DIR=<folder> RDL_LIVE_GALLERY=https://commons.wikimedia.org/wiki/Category:Big_Buck_Bunny cargo test -p rdl-core --test tools_live -- --ignored --test-threads 1`). `svelte-check` passes with 0 errors or warnings. Every milestone was also verified end to end in the running app (see "How to test in the real app" below). CI on GitHub passes (checked by hand after 1f).

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

### How 1g works

- **Settings** (`rdl-core::settings::Settings`): one JSON value under the `app` key (missing fields get defaults; the old `linkgrabber.clipboard_watch` key is read once as a fallback). `AppState::update_settings` validates (limits 1–20 active, 1–32 connections, absolute folder, trimmed patterns), saves and applies: the main queue's `max_active`, `Manager::set_max_connections` (new downloads only), and the LinkGrabber's duplicate lookups. "Start with Windows" goes through `tauri-plugin-autostart` (HKCU `Run` entry with `--minimized`), and `get_settings` reports what Windows actually has. The Settings page saves each change at once and reloads on every visit, since the LinkGrabber's clipboard switch also writes settings.
- **Default folder:** `AppState::download_dir` (the chosen folder, else the user's Downloads) is used by Add links and LinkGrabber confirm; `system_downloads_dir` is the plain Downloads folder for the Settings page.
- **History** (`rdl-core::history`, migration 5 adds `name`, `kind`, `package`): the job runner records every completed download (gallery: its folder). Rows are keyed by SHA-256 of the URL without its `#fragment`, never by download id. `exists` is checked when read. The History page searches name/link/folder/package (LIKE wildcards escaped), opens, reveals, forgets and clears.
- **Duplicate warnings:** with `warn_duplicates` on, every `GrabbedLink` carries `previous` (last finish time, path, whether it still exists), filled in list/add/update/playlist expansion. The UI shows "Downloaded 3 days ago" and unticks the link once if the file is still there; re-ticking sticks.
- **Clipboard:** links matching an ignore pattern (`*` wildcard, else substring, case-insensitive; `settings::wildcard_match`) are dropped. With "ask first" the watcher emits a `ClipboardOffer` event and the toast gets an Add button (10 s).
- **Tray** (`src-tauri/src/tray.rs`): menu Open / Add links from clipboard / Pause all / Resume all / Quit; left click opens the window; the tooltip shows running downloads and combined speed (from download events, once a second). The main window starts hidden (`visible: false`) and is shown in setup unless launched with `--minimized` and "Start in the tray" is on. With "close to tray" on, closing hides the window; a second launch (single instance) or the tray brings it back.
- **Notifications** (`src-tauri/src/notify.rs`): WinRT toasts via `tauri-winrt-notification`. Unpackaged apps need a known AppUserModelID, so at start the app registers `HKCU\Software\Classes\AppUserModelId\dev.rustydownloader.app` (name and an icon written to `<data>/notification-icon.png`). A package finishing → "Download finished" (Open, Show in folder); failures within 3 s → one toast (Retry / Retry all). Nothing is sent while the window has focus. Toasts are shown one at a time: two in the same instant collided in Windows' notification store (`SQLITE_CONSTRAINT_UNIQUE`, 0x87AF0813) and one was lost.
- **Downloads table:** right-click opens a `ContextMenu` (download: pause/resume/retry, open, show in folder, copy link, remove; package: pause/resume all, open folder, copy links, move to top/bottom, remove). Package rows drag with a custom data type (`application/x-rdl-package`, ignored by the link-drop overlay); dropping on a package puts the dragged one before it, below the list puts it last. `Manager::move_package` renumbers `packages.position` and emits `PackagesReordered`; the scheduler starts packages in that order.

### Resume here

- **Repo state:** 2c is committed and pushed (`f7df5be`). 2d and 2e are written and tested but not committed yet.
- **Checked in the app for 1g (2026-10-25)** (2b's checks are in "How 2b works"):
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
- **Your machine:** yt-dlp 2026.10.21, gallery-dl 1.32.17 and ffmpeg N-128018 are in `%APPDATA%\rusty-downloader\tools`. The app's DB is at schema `20261027000006` with empty downloads, LinkGrabber and history. The notification registration (`HKCU\Software\Classes\AppUserModelId\dev.rustydownloader.app`) and the native-host registration (`HKCU\Software\…\NativeMessagingHosts\dev.rustydownloader.host`, currently pointing at the debug host in `target\debug`) stay; the app writes both on every start.
- **Sanity check before starting:** `cargo test --workspace` (167 pass), `pnpm ext:test` (9), `pnpm -C ui check`, then `pnpm dev` and open Settings.
- **Optional, real network:** `RDL_LIVE_DIR=<some folder> cargo test -p rdl-core --test tools_live -- --ignored --nocapture` (about 135 MB of tools on the first run, reused after).

### Phase 2 – browser integration (done)

2a request context, 2b pipe server, 2c native host, 2d extension, 2e registration; see "How 2a works" to "How 2e works". Still owed: a run in Firefox (load `extension/dist/firefox/manifest.json` from `about:debugging`, grant site access in the options page, then repeat the Edge checks) and a download from a real site that needs sign-in.

### Next: Phase 3 – control

Scheduler (start/stop times per queue, on-complete action), speed limiting with time profiles, keep-awake while downloading, checksum verification (§2.3). The Queues page is still a placeholder.

### Deferred / known gaps (pick up in the milestone noted)

- **Not verified:**
  - The Mica window background has never been seen. It can't be captured by DevTools screenshots.
  - Tray menu items and notification buttons were never clicked (no automation for either); the code paths they call are the same commands the UI uses.
- **Desktop (1g follow-ups):**
  - Clipboard "ask first" offers only inside the app; while the window is hidden there's no Windows notification with an Add button.
  - Notification buttons work while the app runs; clicking an old notification after the app has quit does nothing (needs a COM activator).
  - "Connections per download" applies to downloads that start after the change; running ones keep theirs.
  - The Queues page is still a placeholder (scheduler and speed limits are Phase 3).
  - History records the URL as given; two links to the same file under different URLs aren't recognised as duplicates (plan §2.3 also wants path + size + hash).
- **HTTP engine:**
  - `Retry-After` on 429 isn't honoured (429 uses normal backoff).
  - Expired links (`LinkExpired`) need plugin refresh (Phase 6).
- **Request context (2a follow-ups):**
  - Not tried against a real site that needs sign-in yet; that needs the extension (2d). The cookie file format was checked against what yt-dlp's and gallery-dl's loaders accept, not by a live run.
  - Cookies sit in the SQLite DB in plain text until the download completes (same protection as the rest of `%APPDATA%`). DPAPI encryption would be the next step if that matters.
  - No UI shows that a link carries browser cookies, and there's no way to add or clear them by hand (a `cookies.txt` import belongs to Phase 4 accounts).
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

| Phase | Scope | Exit criteria | Status |
|---|---|---|---|
| **0. Scaffold** | Workspace, Tauri + SvelteKit shell, SQLite migrations, tracing logs, CI, Fluent theme base | App launches, CI green | ✅ Done (CI never checked) |
| **1. MVP** | Segmented HTTP engine (resume, retries, dynamic splitting, headers/cookies/auth), queues + concurrency, packages, LinkGrabber (paste/clipboard/drag-drop), yt-dlp + gallery-dl (resolve, download, progress, common flags, impersonation, archives) + tool updater, tray, toasts, history/duplicate detection, single instance, settings | Used as daily driver for HTTP, video and galleries | ✅ Done |
| **2. Browser** | MV3 extension (Chromium + Firefox), native host, download interception, context menus, cookie hand-off | Browser downloads flow into the app with cookies | ✅ Done in Edge (Firefox not run) |
| **3. Control** | Scheduler, speed limiting + profiles, keep-awake, on-complete actions, checksum verification | Overnight queue then shutdown works | ▶️ Next |
| **4. Network & protocols** | Proxies + rules, interface binding + kill-switch, accounts in the keyring, FTP/FTPS, SFTP | Downloads via SOCKS5/VPN interface, FTP/SFTP resume | ⏳ |
| **5. Torrent** | librqbit integration, magnet/.torrent associations, seeding limits (ratio/time) | Torrent download + seed from the same UI | ⏳ |
| **6. Plugins & extract** | Rhai plugin host + API, 2–3 reference plugins, auto-extract with password lists | A hoster resolved via plugin, archive auto-extracted | ⏳ |
| **7. Remote & release polish** | axum remote UI/API, portable build, auto-update pipeline, installer polish | Install, auto-update and phone control work end to end | ⏳ |

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

1. **Updater hosting**: separate public releases repo vs. private repo + token. Recommended: public releases-only repo.
2. **Rhai vs Lua (`mlua`)**: Rhai is chosen for its pure-Rust build and sandboxing. Lua is the fallback if plugin performance or familiarity becomes an issue.
3. **gallery-dl default mode**: native download (recommended) vs resolve-only via the HTTP engine, per site.
4. **Default concurrency**: 3 active × 8 segments. Tune after real-world testing.
5. **Authenticode certificate**: skip for personal use, or buy one (or use Azure Trusted Signing) if it's ever distributed.

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
