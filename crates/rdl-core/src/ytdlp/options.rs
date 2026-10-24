//! Format presets and how they map to yt-dlp flags.

use serde::{Deserialize, Serialize};

use super::parse::FormatInfo;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum FormatPreset {
    /// Best video and best audio, merged.
    #[default]
    Best,
    Max2160,
    Max1080,
    Max720,
    Max480,
    AudioMp3,
    AudioM4a,
    /// The `custom_format` string as `-f`.
    Custom,
}

impl FormatPreset {
    fn max_height(self) -> Option<u32> {
        match self {
            FormatPreset::Max2160 => Some(2160),
            FormatPreset::Max1080 => Some(1080),
            FormatPreset::Max720 => Some(720),
            FormatPreset::Max480 => Some(480),
            _ => None,
        }
    }

    pub fn is_audio(self) -> bool {
        matches!(self, FormatPreset::AudioMp3 | FormatPreset::AudioM4a)
    }

    /// Extension the result will most likely have, for display before download.
    pub fn likely_ext(self) -> Option<&'static str> {
        match self {
            FormatPreset::AudioMp3 => Some("mp3"),
            FormatPreset::AudioM4a => Some("m4a"),
            _ => None,
        }
    }
}

/// Per-download yt-dlp settings, stored as `handler_opts_json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", default)]
pub struct YtDlpOptions {
    pub preset: FormatPreset,
    /// Used when `preset` is `Custom`.
    pub custom_format: Option<String>,
}

impl YtDlpOptions {
    pub fn format_args(&self) -> Vec<String> {
        let height = |h: u32| format!("bv*[height<={h}]+ba/b[height<={h}]/bv*+ba/b");
        let args: Vec<&str> = match self.preset {
            FormatPreset::Best => vec!["-f", "bv*+ba/b"],
            FormatPreset::AudioMp3 => {
                vec![
                    "-f",
                    "ba/b",
                    "-x",
                    "--audio-format",
                    "mp3",
                    "--audio-quality",
                    "0",
                ]
            }
            FormatPreset::AudioM4a => vec!["-f", "ba[ext=m4a]/ba/b", "-x", "--audio-format", "m4a"],
            FormatPreset::Custom => match self.custom_format.as_deref().map(str::trim) {
                Some(f) if !f.is_empty() => return vec!["-f".into(), f.into()],
                _ => vec!["-f", "bv*+ba/b"],
            },
            preset => {
                let h = preset.max_height().expect("height presets have a height");
                return vec!["-f".into(), height(h)];
            }
        };
        args.into_iter().map(str::to_owned).collect()
    }

    /// Rough size of what this preset will fetch, from the formats `-J` listed.
    pub fn estimate_size(&self, formats: &[FormatInfo]) -> Option<u64> {
        let audio = formats
            .iter()
            .filter(|f| f.has_audio() && !f.has_video())
            .max_by(|a, b| a.tbr.unwrap_or(0.0).total_cmp(&b.tbr.unwrap_or(0.0)));
        if self.preset.is_audio() {
            return audio.and_then(|a| a.size);
        }
        if self.preset == FormatPreset::Custom {
            return None;
        }
        let limit = self.preset.max_height().unwrap_or(u32::MAX);
        let video = formats
            .iter()
            .filter(|f| f.has_video() && f.height.is_some_and(|h| h <= limit))
            .max_by(|a, b| {
                (a.height, a.tbr.unwrap_or(0.0))
                    .partial_cmp(&(b.height, b.tbr.unwrap_or(0.0)))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })?;
        if video.has_audio() {
            return video.size;
        }
        Some(video.size? + audio.and_then(|a| a.size).unwrap_or(0))
    }
}

/// A name the user typed may already carry an extension; yt-dlp adds its own.
pub(super) fn strip_media_ext(name: &str) -> &str {
    const MEDIA: [&str; 12] = [
        "mp4", "mkv", "webm", "mov", "avi", "flv", "m4a", "mp3", "opus", "ogg", "wav", "aac",
    ];
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && MEDIA.contains(&&*ext.to_ascii_lowercase()) => {
            stem
        }
        _ => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(
        id: &str,
        height: Option<u32>,
        video: bool,
        audio: bool,
        size: u64,
        tbr: f64,
    ) -> FormatInfo {
        FormatInfo {
            format_id: id.into(),
            ext: "mp4".into(),
            height,
            vcodec: Some(if video { "avc1" } else { "none" }.into()),
            acodec: Some(if audio { "mp4a" } else { "none" }.into()),
            size: Some(size),
            tbr: Some(tbr),
            note: None,
        }
    }

    #[test]
    fn presets_map_to_flags() {
        let opts = |preset| YtDlpOptions {
            preset,
            custom_format: None,
        };
        assert_eq!(opts(FormatPreset::Best).format_args(), ["-f", "bv*+ba/b"]);
        assert_eq!(
            opts(FormatPreset::Max720).format_args(),
            ["-f", "bv*[height<=720]+ba/b[height<=720]/bv*+ba/b"]
        );
        assert!(
            opts(FormatPreset::AudioMp3)
                .format_args()
                .contains(&"mp3".to_owned())
        );
        let custom = YtDlpOptions {
            preset: FormatPreset::Custom,
            custom_format: Some(" 22/18 ".into()),
        };
        assert_eq!(custom.format_args(), ["-f", "22/18"]);
        let empty_custom = YtDlpOptions {
            preset: FormatPreset::Custom,
            custom_format: Some(" ".into()),
        };
        assert_eq!(empty_custom.format_args(), ["-f", "bv*+ba/b"]);
    }

    #[test]
    fn estimates_sizes_per_preset() {
        let formats = [
            fmt("140", None, false, true, 3_000, 128.0),
            fmt("139", None, false, true, 1_000, 48.0),
            fmt("137", Some(1080), true, false, 50_000, 4000.0),
            fmt("136", Some(720), true, false, 20_000, 2000.0),
            fmt("18", Some(360), true, true, 9_000, 500.0),
        ];
        let est = |preset| {
            YtDlpOptions {
                preset,
                custom_format: None,
            }
            .estimate_size(&formats)
        };
        assert_eq!(est(FormatPreset::Best), Some(53_000));
        assert_eq!(est(FormatPreset::Max720), Some(23_000));
        assert_eq!(est(FormatPreset::Max480), Some(9_000));
        assert_eq!(est(FormatPreset::AudioM4a), Some(3_000));
        assert_eq!(est(FormatPreset::Custom), None);
    }

    #[test]
    fn strips_only_media_extensions() {
        assert_eq!(strip_media_ext("Clip.MP4"), "Clip");
        assert_eq!(strip_media_ext("Version 1.5"), "Version 1.5");
        assert_eq!(strip_media_ext(".mp4"), ".mp4");
    }
}
