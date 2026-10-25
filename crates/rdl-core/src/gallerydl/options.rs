//! Per-download gallery-dl settings, and the flag schema the options drawer
//! is built from.
//!
//! The schema (`flags.json`) lists the command-line options worth a control,
//! grouped as in gallery-dl's `--help`. Settings refer to flags by schema id,
//! so a flag that a newer gallery-dl drops (or renames) is simply left out:
//! the `--help` self-check passes the flags the installed version knows.

use std::collections::{BTreeMap, HashSet};
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum FlagKind {
    /// On or off; on passes the flag (and `template`, if any).
    Switch,
    Text,
    Number,
    Choice,
    /// One value per line; the flag repeats for each.
    Lines,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct FlagChoice {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct FlagDef {
    /// Key in [`GalleryDlOptions::flags`].
    pub id: String,
    /// The command-line option, as `--help` lists it.
    pub flag: String,
    pub kind: FlagKind,
    pub label: String,
    #[serde(default)]
    pub help: Option<String>,
    #[serde(default)]
    pub placeholder: Option<String>,
    /// The argument after `flag`, with `{}` standing for the value
    /// (`browser={}` for `-o`). Without it the value is passed as is.
    #[serde(default)]
    pub template: Option<String>,
    #[serde(default)]
    pub choices: Vec<FlagChoice>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct FlagGroup {
    pub id: String,
    pub label: String,
    pub flags: Vec<FlagDef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct FlagSchema {
    pub version: u32,
    pub groups: Vec<FlagGroup>,
}

impl FlagSchema {
    fn find(&self, id: &str) -> Option<&FlagDef> {
        self.groups
            .iter()
            .flat_map(|g| &g.flags)
            .find(|f| f.id == id)
    }
}

/// The schema shipped with the app.
pub fn schema() -> &'static FlagSchema {
    static SCHEMA: LazyLock<FlagSchema> = LazyLock::new(|| {
        serde_json::from_str(include_str!("flags.json")).expect("flags.json is valid")
    });
    &SCHEMA
}

/// Per-download gallery-dl settings, stored as `handler_opts_json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct GalleryDlOptions {
    /// Skip files this site's download archive says were fetched before, so
    /// downloading a gallery again only gets what's new.
    pub archive: bool,
    /// Load the user's own gallery-dl config file (off: `--config-ignore`,
    /// so runs behave the same everywhere).
    pub user_config: bool,
    /// Values by schema flag id. Switches are `"true"` when on.
    pub flags: BTreeMap<String, String>,
    /// Extra arguments, split like a command line (quotes group words).
    pub raw_args: Option<String>,
}

impl Default for GalleryDlOptions {
    fn default() -> Self {
        Self {
            archive: true,
            user_config: false,
            flags: BTreeMap::new(),
            raw_args: None,
        }
    }
}

impl GalleryDlOptions {
    /// Whether these settings may fetch fewer files than resolving counted:
    /// any Selection option set (`--range`, filters, size and date limits),
    /// or raw arguments, which could hold anything.
    pub fn narrows(&self, schema: &FlagSchema) -> bool {
        let selection = schema.groups.iter().filter(|g| g.id == "selection");
        let set = selection.flat_map(|g| &g.flags).any(|def| {
            self.flags
                .get(&def.id)
                .is_some_and(|v| !v.trim().is_empty() && v != "false")
        });
        set || self
            .raw_args
            .as_deref()
            .is_some_and(|r| !r.trim().is_empty())
    }

    /// Command-line arguments for the chosen flags, in schema order, then the
    /// raw ones. With `available`, flags the installed version lacks are
    /// dropped instead of failing the run.
    pub fn flag_args(
        &self,
        schema: &FlagSchema,
        available: Option<&HashSet<String>>,
    ) -> Result<Vec<String>> {
        let mut args = Vec::new();
        for def in schema.groups.iter().flat_map(|g| &g.flags) {
            let Some(value) = self.flags.get(&def.id).map(|v| v.trim()) else {
                continue;
            };
            if value.is_empty() || available.is_some_and(|a| !a.contains(&def.flag)) {
                continue;
            }
            let arg = |v: &str| match &def.template {
                Some(t) => t.replace("{}", v),
                None => v.to_owned(),
            };
            match def.kind {
                FlagKind::Switch => {
                    if value == "true" {
                        args.push(def.flag.clone());
                        if let Some(t) = &def.template {
                            args.push(t.clone());
                        }
                    }
                }
                FlagKind::Lines => {
                    for line in value.lines().map(str::trim).filter(|l| !l.is_empty()) {
                        args.push(def.flag.clone());
                        args.push(arg(line));
                    }
                }
                FlagKind::Text | FlagKind::Number | FlagKind::Choice => {
                    args.push(def.flag.clone());
                    args.push(arg(value));
                }
            }
        }
        if let Some(raw) = &self.raw_args {
            args.extend(split_args(raw)?);
        }
        Ok(args)
    }

    /// Check what the UI sent: known flags, numbers that are numbers, choices
    /// from the list, and raw arguments that split.
    pub fn validate(&self, schema: &FlagSchema) -> Result<()> {
        for (id, value) in &self.flags {
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            let Some(def) = schema.find(id) else {
                return Err(Error::InvalidInput(format!(
                    "Unknown gallery-dl option {id}."
                )));
            };
            let ok = match def.kind {
                FlagKind::Switch => value == "true" || value == "false",
                FlagKind::Number => value.parse::<f64>().is_ok(),
                FlagKind::Choice => def.choices.iter().any(|c| c.value == value),
                FlagKind::Text | FlagKind::Lines => true,
            };
            if !ok {
                return Err(Error::InvalidInput(format!(
                    "\"{value}\" is not a valid value for {}.",
                    def.label
                )));
            }
        }
        if let Some(raw) = &self.raw_args {
            split_args(raw)?;
        }
        Ok(())
    }
}

/// Split a command line into arguments. Whitespace separates; double or
/// single quotes group (and are removed). Backslashes are literal, as in
/// Windows paths.
pub fn split_args(line: &str) -> Result<Vec<String>> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_arg = false;
    let mut quote: Option<char> = None;
    for c in line.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => current.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                in_arg = true;
            }
            None if c.is_whitespace() => {
                if in_arg {
                    args.push(std::mem::take(&mut current));
                    in_arg = false;
                }
            }
            None => {
                current.push(c);
                in_arg = true;
            }
        }
    }
    if let Some(q) = quote {
        return Err(Error::InvalidInput(format!(
            "The extra arguments have an unclosed {q} quote."
        )));
    }
    if in_arg {
        args.push(current);
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(flags: &[(&str, &str)]) -> GalleryDlOptions {
        GalleryDlOptions {
            flags: flags
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            ..GalleryDlOptions::default()
        }
    }

    #[test]
    fn the_shipped_schema_parses_with_unique_ids() {
        let s = schema();
        let ids: Vec<&str> = s
            .groups
            .iter()
            .flat_map(|g| &g.flags)
            .map(|f| &*f.id)
            .collect();
        let unique: HashSet<&str> = ids.iter().copied().collect();
        assert_eq!(ids.len(), unique.len());
        for def in s.groups.iter().flat_map(|g| &g.flags) {
            assert!(def.flag.starts_with('-'), "{}", def.id);
            assert_eq!(
                def.kind == FlagKind::Choice,
                !def.choices.is_empty(),
                "{}",
                def.id
            );
        }
    }

    #[test]
    fn maps_settings_to_arguments_in_schema_order() {
        let o = GalleryDlOptions {
            raw_args: Some(r#"--sleep-429 30 -o "extractor.foo=a b""#.into()),
            ..opts(&[
                ("range", " 1-5 "),
                ("no-part", "true"),
                ("no-skip", "false"),
                ("browser", "firefox"),
                ("browser-user-agent", "true"),
                ("option", "a=1\n\n b=2 "),
                ("filter", ""),
            ])
        };
        let args = o.flag_args(schema(), None).unwrap();
        assert_eq!(
            args,
            [
                "--no-part",
                "--range",
                "1-5",
                "-o",
                "browser=firefox",
                "--user-agent",
                "browser",
                "--option",
                "a=1",
                "--option",
                "b=2",
                "--sleep-429",
                "30",
                "-o",
                "extractor.foo=a b",
            ]
        );
    }

    #[test]
    fn flags_missing_from_the_installed_version_are_dropped() {
        let o = opts(&[("range", "1-5"), ("ugoira", "webm")]);
        let available: HashSet<String> = ["--range".to_owned()].into();
        assert_eq!(
            o.flag_args(schema(), Some(&available)).unwrap(),
            ["--range", "1-5"]
        );
    }

    #[test]
    fn validates_values() {
        assert!(opts(&[("retries", "3")]).validate(schema()).is_ok());
        assert!(opts(&[("retries", "lots")]).validate(schema()).is_err());
        assert!(opts(&[("ugoira", "avi")]).validate(schema()).is_err());
        assert!(opts(&[("nonsense", "1")]).validate(schema()).is_err());
        assert!(opts(&[("nonsense", "")]).validate(schema()).is_ok());
        let bad_raw = GalleryDlOptions {
            raw_args: Some("--filter \"x".into()),
            ..GalleryDlOptions::default()
        };
        assert!(bad_raw.validate(schema()).is_err());
    }

    #[test]
    fn splits_command_lines() {
        assert_eq!(
            split_args(r#"  -o 'a=b c' --x C:\dir\f.txt "" end"#).unwrap(),
            ["-o", "a=b c", "--x", r"C:\dir\f.txt", "", "end"]
        );
        assert_eq!(split_args("a\"b c\"d").unwrap(), ["ab cd"]);
        assert!(split_args("").unwrap().is_empty());
        assert!(split_args("'open").is_err());
    }

    #[test]
    fn selection_options_and_raw_args_narrow_the_gallery() {
        let s = schema();
        assert!(!GalleryDlOptions::default().narrows(s));
        assert!(!opts(&[("browser", "firefox"), ("range", " ")]).narrows(s));
        assert!(opts(&[("range", "1-5")]).narrows(s));
        assert!(opts(&[("filesize-min", "500k")]).narrows(s));
        let raw = GalleryDlOptions {
            raw_args: Some("--sleep-429 30".into()),
            ..GalleryDlOptions::default()
        };
        assert!(raw.narrows(s));
    }

    #[test]
    fn old_settings_without_new_fields_get_defaults() {
        let o: GalleryDlOptions = serde_json::from_str(r#"{"flags":{"range":"1"}}"#).unwrap();
        assert!(o.archive);
        assert_eq!(o.flags["range"], "1");
    }
}
