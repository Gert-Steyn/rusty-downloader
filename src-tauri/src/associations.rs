//! Windows' handling of `magnet:` links and `.torrent` files, per user
//! (`HKCU\Software\Classes` and the Default apps list; no admin), the same
//! way the browser host is registered.
//!
//! Done on every start while the `handle_torrents` setting is on, and undone
//! when it's turned off or the app is uninstalled. Two parts:
//! - Our two ProgIDs (`RustyDownloader.Magnet`, `RustyDownloader.Torrent`)
//!   and a capabilities entry, so Windows' "Default apps" page lists us.
//! - The plain `magnet` protocol and `.torrent` default, but only where no
//!   other app has claimed them. Windows' own choice (`UserChoice`) is hashed
//!   and never written here; "Choose default apps" is how the user picks us
//!   over another client.
//!
//! The registry code is thin; the decisions (what a command means, who owns
//! a type) are pure functions with tests.

use std::path::Path;

use serde::Serialize;
use specta::Type;
use windows_registry::{CURRENT_USER, Key, LOCAL_MACHINE};

use crate::commands::{CommandError, CommandResult};

pub const MAGNET_PROGID: &str = "RustyDownloader.Magnet";
pub const TORRENT_PROGID: &str = "RustyDownloader.Torrent";
const APP_NAME: &str = "Rusty Downloader";
const APP_DESCRIPTION: &str = "Downloads files from web links, magnet links and torrents.";

const CLASSES: &str = r"Software\Classes";
const MAGNET_CLASS: &str = r"Software\Classes\magnet";
const MAGNET_COMMAND: &str = r"Software\Classes\magnet\shell\open\command";
const TORRENT_EXT: &str = r"Software\Classes\.torrent";
const TORRENT_OPEN_WITH: &str = r"Software\Classes\.torrent\OpenWithProgids";
const CAPABILITIES: &str = r"Software\RustyDownloader\Capabilities";
const REGISTERED_APPS: &str = r"Software\RegisteredApplications";
const USER_CHOICE_MAGNET: &str =
    r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\magnet\UserChoice";
const USER_CHOICE_TORRENT: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.torrent\UserChoice";

/// The roots the registry is read from and written to. Production passes
/// HKCU and HKLM; a test passes two scratch keys.
pub struct Hives<'a> {
    /// Everything is written here.
    pub user: &'a Key,
    /// Read only: a machine-wide claim by another app keeps us out.
    pub machine: &'a Key,
}

/// Who handles a link or file type, as far as the registry says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum Handler {
    /// This copy of the app.
    Ours,
    /// Another app.
    Other,
    /// Nothing is registered.
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AssociationStatus {
    pub magnet: Handler,
    pub torrent: Handler,
}

/// The command Windows runs for a link or file: the app with the path as its
/// one argument.
pub fn open_command(exe: &Path) -> String {
    format!("\"{}\" \"%1\"", exe.display())
}

/// Whether a registered command runs `exe`. The program is the first word,
/// quoted or not; the comparison ignores case and treats `/` as `\`.
pub fn command_runs(command: &str, exe: &Path) -> bool {
    let command = command.trim_start();
    let program = match command.strip_prefix('"') {
        Some(rest) => rest.split('"').next().unwrap_or_default(),
        None => command.split_whitespace().next().unwrap_or_default(),
    };
    same_path(program, exe)
}

fn same_path(a: &str, b: &Path) -> bool {
    fn normal(s: &str) -> String {
        s.trim().replace('/', r"\").to_ascii_lowercase()
    }
    let a = normal(a);
    !a.is_empty() && a == normal(&b.display().to_string())
}

fn is_progid(value: &str, ours: &str) -> bool {
    value.trim().eq_ignore_ascii_case(ours)
}

/// The first claim that exists, in order of precedence; none is `None`.
fn first_claim(claims: impl IntoIterator<Item = Option<Handler>>) -> Handler {
    claims.into_iter().flatten().next().unwrap_or(Handler::None)
}

/// A string value, if the key and value exist and aren't blank. `""` is the
/// default value.
fn read(hive: &Key, path: &str, name: &str) -> Option<String> {
    hive.open(path)
        .ok()?
        .get_string(name)
        .ok()
        .filter(|s| !s.trim().is_empty())
}

/// A key open for writing, if it exists.
fn writable(hive: &Key, path: &str) -> Option<Key> {
    hive.options().write().open(path).ok()
}

/// Who handles magnet links and `.torrent` files now. The user's choice wins
/// over the registered handler, and the user's hive over the machine's.
pub fn status(hives: &Hives, exe: &Path) -> AssociationStatus {
    let magnet = first_claim([
        read(hives.user, USER_CHOICE_MAGNET, "ProgId")
            .map(|p| handler_of_progid(&p, MAGNET_PROGID)),
        read(hives.user, MAGNET_COMMAND, "").map(|c| handler_of_command(&c, exe)),
        read(hives.machine, MAGNET_COMMAND, "").map(|c| handler_of_command(&c, exe)),
    ]);
    let torrent = first_claim([
        read(hives.user, USER_CHOICE_TORRENT, "ProgId")
            .map(|p| handler_of_progid(&p, TORRENT_PROGID)),
        read(hives.user, TORRENT_EXT, "").map(|p| handler_of_progid(&p, TORRENT_PROGID)),
        read(hives.machine, TORRENT_EXT, "").map(|p| handler_of_progid(&p, TORRENT_PROGID)),
    ]);
    AssociationStatus { magnet, torrent }
}

fn handler_of_command(command: &str, exe: &Path) -> Handler {
    if command_runs(command, exe) {
        Handler::Ours
    } else {
        Handler::Other
    }
}

fn handler_of_progid(progid: &str, ours: &str) -> Handler {
    if is_progid(progid, ours) {
        Handler::Ours
    } else {
        Handler::Other
    }
}

/// The protocol key may be ours (or absent), no other app may have one
/// machine-wide, and the user must not have chosen another app.
fn magnet_is_free(hives: &Hives, exe: &Path) -> bool {
    let ours_or_absent = hives.user.open(MAGNET_CLASS).is_err()
        || read(hives.user, MAGNET_COMMAND, "").is_some_and(|c| command_runs(&c, exe));
    let machine_free = hives.machine.open(MAGNET_CLASS).is_err();
    let choice_free =
        read(hives.user, USER_CHOICE_MAGNET, "ProgId").is_none_or(|p| is_progid(&p, MAGNET_PROGID));
    ours_or_absent && machine_free && choice_free
}

/// The same rule as [`magnet_is_free`] for the `.torrent` default.
fn torrent_is_free(hives: &Hives) -> bool {
    let default_ok =
        read(hives.user, TORRENT_EXT, "").is_none_or(|p| is_progid(&p, TORRENT_PROGID));
    let machine_free = read(hives.machine, TORRENT_EXT, "").is_none();
    let choice_free = read(hives.user, USER_CHOICE_TORRENT, "ProgId")
        .is_none_or(|p| is_progid(&p, TORRENT_PROGID));
    default_ok && machine_free && choice_free
}

/// Write everything that makes this copy a candidate handler, and take the
/// plain protocol and `.torrent` default where that's free. Returns a note
/// per decision, for the log.
pub fn register(hives: &Hives, exe: &Path) -> Result<Vec<String>, String> {
    let user = hives.user;
    let icon = format!("{},0", exe.display());
    let command = open_command(exe);
    let mut notes = Vec::new();

    write_progid(user, MAGNET_PROGID, "Magnet link", true, &icon, &command)?;
    write_progid(user, TORRENT_PROGID, "Torrent file", false, &icon, &command)?;

    put(user, CAPABILITIES, "ApplicationName", APP_NAME)?;
    put(
        user,
        CAPABILITIES,
        "ApplicationDescription",
        APP_DESCRIPTION,
    )?;
    put(user, CAPABILITIES, "ApplicationIcon", &icon)?;
    put(
        user,
        &format!(r"{CAPABILITIES}\URLAssociations"),
        "magnet",
        MAGNET_PROGID,
    )?;
    put(
        user,
        &format!(r"{CAPABILITIES}\FileAssociations"),
        ".torrent",
        TORRENT_PROGID,
    )?;
    put(user, REGISTERED_APPS, APP_NAME, CAPABILITIES)?;
    put(user, TORRENT_OPEN_WITH, TORRENT_PROGID, "")?;
    notes.push("Default apps entry".to_owned());

    if torrent_is_free(hives) {
        put(user, TORRENT_EXT, "", TORRENT_PROGID)?;
        notes.push(".torrent files: this app".to_owned());
    } else {
        notes.push(".torrent files: left to another app".to_owned());
    }

    if magnet_is_free(hives, exe) {
        put(user, MAGNET_CLASS, "", "URL:Magnet link")?;
        put(user, MAGNET_CLASS, "URL Protocol", "")?;
        put(user, MAGNET_COMMAND, "", &command)?;
        notes.push("magnet links: this app".to_owned());
    } else {
        notes.push("magnet links: left to another app".to_owned());
    }

    notify_shell();
    Ok(notes)
}

/// Remove what [`register`] wrote for this copy. Each step is best effort:
/// a key that isn't there is fine. Another copy's entries stay (the ProgIDs
/// are shared by name, so the last copy registered owns them).
pub fn unregister(hives: &Hives, exe: &Path) {
    let user = hives.user;
    for id in [MAGNET_PROGID, TORRENT_PROGID] {
        let _ = user.remove_tree(format!(r"{CLASSES}\{id}"));
    }
    let _ = user.remove_tree(CAPABILITIES);
    if let Some(key) = writable(user, REGISTERED_APPS) {
        let _ = key.remove_value(APP_NAME);
    }
    if let Some(key) = writable(user, TORRENT_OPEN_WITH) {
        let _ = key.remove_value(TORRENT_PROGID);
    }
    if read(user, TORRENT_EXT, "").is_some_and(|p| is_progid(&p, TORRENT_PROGID))
        && let Some(key) = writable(user, TORRENT_EXT)
    {
        let _ = key.remove_value("");
    }
    if read(user, MAGNET_COMMAND, "").is_some_and(|c| command_runs(&c, exe)) {
        let _ = user.remove_tree(MAGNET_CLASS);
    }
    notify_shell();
}

fn write_progid(
    user: &Key,
    id: &str,
    name: &str,
    url_protocol: bool,
    icon: &str,
    command: &str,
) -> Result<(), String> {
    let root = format!(r"{CLASSES}\{id}");
    put(user, &root, "", name)?;
    if url_protocol {
        put(user, &root, "URL Protocol", "")?;
    }
    put(user, &format!(r"{root}\DefaultIcon"), "", icon)?;
    put(user, &format!(r"{root}\shell\open\command"), "", command)
}

/// Create `path` under `hive` and set one string value in it.
fn put(hive: &Key, path: &str, name: &str, value: &str) -> Result<(), String> {
    hive.create(path)
        .and_then(|key| key.set_string(name, value))
        .map_err(|e| format!(r"{path}: {e}"))
}

/// Tell the shell the associations changed, so Explorer and the Default apps
/// page catch up without a sign-out.
#[allow(unsafe_code)]
fn notify_shell() {
    use windows_sys::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
    // SAFETY: a plain notification with no item pointers.
    unsafe {
        SHChangeNotify(
            SHCNE_ASSOCCHANGED as _,
            SHCNF_IDLIST as _,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
}

fn with_hives<T>(f: impl FnOnce(&Hives) -> T) -> T {
    f(&Hives {
        user: CURRENT_USER,
        machine: LOCAL_MACHINE,
    })
}

/// Register the running copy. Errors only cost the associations, so they
/// are logged.
pub fn register_here() {
    let Ok(exe) = std::env::current_exe() else {
        tracing::warn!(
            "can't find the app's file; magnet links and .torrent files stay unregistered"
        );
        return;
    };
    match with_hives(|hives| register(hives, &exe)) {
        Ok(notes) => tracing::info!(?notes, "registered for magnet links and .torrent files"),
        Err(e) => {
            tracing::warn!(error = %e, "could not register for magnet links and .torrent files")
        }
    }
}

/// Remove the running copy's registrations.
pub fn unregister_here() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    with_hives(|hives| unregister(hives, &exe));
    tracing::info!("removed the magnet link and .torrent registrations");
}

/// Follow the setting: register when it's on, remove when it's off.
pub fn apply(enabled: bool) {
    if enabled {
        register_here();
    } else {
        unregister_here();
    }
}

/// Who handles magnet links and `.torrent` files now, for Settings.
#[tauri::command]
#[specta::specta]
pub fn torrent_association_status() -> AssociationStatus {
    let Ok(exe) = std::env::current_exe() else {
        return AssociationStatus {
            magnet: Handler::None,
            torrent: Handler::None,
        };
    };
    with_hives(|hives| status(hives, &exe))
}

/// Open Windows' Default apps page, where the user can pick this app.
#[tauri::command]
#[specta::specta]
pub fn open_default_apps() -> CommandResult<()> {
    tauri_plugin_opener::open_url("ms-settings:defaultapps", None::<&str>)
        .map_err(|e| CommandError(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = r"C:\Program Files\Rusty Downloader\rusty-downloader.exe";

    #[test]
    fn open_command_passes_the_link_as_one_quoted_argument() {
        assert_eq!(
            open_command(Path::new(EXE)),
            r#""C:\Program Files\Rusty Downloader\rusty-downloader.exe" "%1""#
        );
    }

    #[test]
    fn a_command_is_ours_when_its_program_is_this_exe() {
        let exe = Path::new(EXE);
        let ours = open_command(exe);
        assert!(command_runs(&ours, exe));
        // Case and slashes don't matter; a different program does.
        assert!(command_runs(
            r#""c:/program files/rusty downloader/Rusty-Downloader.EXE" "%1""#,
            exe
        ));
        assert!(!command_runs(r#""C:\Other\qbittorrent.exe" "%1""#, exe));
        // Unquoted and quoted programs both read.
        assert!(command_runs(
            r"C:\Tools\rd.exe %1",
            Path::new(r"C:\Tools\rd.exe")
        ));
        // A prefix of the path is a different program.
        assert!(!command_runs(
            r#""C:\Program Files\Rusty Downloader\rusty-downloader.exe.old" "%1""#,
            exe
        ));
        assert!(!command_runs("", exe));
        assert!(!command_runs(r#"""  "%1""#, exe));
    }

    #[test]
    fn the_first_claim_in_precedence_order_wins() {
        assert_eq!(first_claim([None, None]), Handler::None);
        assert_eq!(
            first_claim([None, Some(Handler::Other), Some(Handler::Ours)]),
            Handler::Other
        );
        assert_eq!(
            first_claim([Some(Handler::Ours), Some(Handler::Other)]),
            Handler::Ours
        );
    }

    #[test]
    fn progids_compare_ignoring_case_and_spaces() {
        assert_eq!(
            handler_of_progid("rustydownloader.magnet ", MAGNET_PROGID),
            Handler::Ours
        );
        assert_eq!(
            handler_of_progid("qBittorrent.magnet", MAGNET_PROGID),
            Handler::Other
        );
        assert_eq!(
            handler_of_progid(TORRENT_PROGID, MAGNET_PROGID),
            Handler::Other
        );
    }

    #[test]
    fn the_ours_and_theirs_split_follows_the_registered_command() {
        let exe = Path::new(EXE);
        assert_eq!(handler_of_command(&open_command(exe), exe), Handler::Ours);
        assert_eq!(
            handler_of_command(r#""C:\qBit\qbittorrent.exe" "%1""#, exe),
            Handler::Other
        );
    }

    #[test]
    fn the_registered_names_are_what_windows_expects() {
        // Capabilities and the ProgIDs must agree with each other.
        assert!(CAPABILITIES.ends_with(r"\Capabilities"));
        assert_eq!(MAGNET_CLASS, r"Software\Classes\magnet");
        assert!(MAGNET_COMMAND.starts_with(MAGNET_CLASS));
        assert!(TORRENT_OPEN_WITH.starts_with(TORRENT_EXT));
        assert!(!MAGNET_PROGID.contains(' ') && !TORRENT_PROGID.contains(' '));
    }

    /// Writes under a scratch key and removes it again. Run with
    /// `cargo test -p rusty-downloader associations -- --ignored`.
    #[test]
    #[ignore = "writes HKCU\\Software\\RustyDownloaderTest"]
    fn registers_defers_to_other_apps_and_unregisters() {
        const ROOT: &str = r"Software\RustyDownloaderTest";
        let exe = Path::new(EXE);
        let user = CURRENT_USER.create(format!(r"{ROOT}\User")).unwrap();
        let machine = CURRENT_USER.create(format!(r"{ROOT}\Machine")).unwrap();
        let hives = Hives {
            user: &user,
            machine: &machine,
        };
        let none = AssociationStatus {
            magnet: Handler::None,
            torrent: Handler::None,
        };
        assert_eq!(status(&hives, exe), none);

        // Another app is installed for everyone: it keeps magnet links.
        machine
            .create(MAGNET_COMMAND)
            .unwrap()
            .set_string("", r#""C:\Other\other.exe" "%1""#)
            .unwrap();
        let notes = register(&hives, exe).unwrap();
        assert!(
            notes.iter().any(|n| n.starts_with("magnet links: left")),
            "{notes:?}"
        );
        assert!(
            user.open(MAGNET_CLASS).is_err(),
            "our protocol key is not written"
        );
        assert_eq!(
            status(&hives, exe),
            AssociationStatus {
                magnet: Handler::Other,
                torrent: Handler::Ours
            }
        );

        // Once it's gone, we take the protocol too.
        machine.remove_tree(MAGNET_CLASS).unwrap();
        register(&hives, exe).unwrap();
        assert_eq!(
            status(&hives, exe),
            AssociationStatus {
                magnet: Handler::Ours,
                torrent: Handler::Ours
            }
        );
        assert_eq!(
            user.open(REGISTERED_APPS)
                .unwrap()
                .get_string(APP_NAME)
                .unwrap(),
            CAPABILITIES
        );

        unregister(&hives, exe);
        assert_eq!(status(&hives, exe), none);
        assert!(user.open(CAPABILITIES).is_err());
        assert!(user.open(format!(r"{CLASSES}\{MAGNET_PROGID}")).is_err());
        assert!(
            user.open(REGISTERED_APPS)
                .unwrap()
                .get_string(APP_NAME)
                .is_err()
        );

        drop((user, machine));
        CURRENT_USER.remove_tree(ROOT).unwrap();
    }
}
