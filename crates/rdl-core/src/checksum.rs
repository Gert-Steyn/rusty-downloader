//! Checksums of finished files: MD5, SHA-1, SHA-256 and CRC32, typed in by
//! the user or read from `.sfv` / `.md5` / `.sha1` / `.sha256` files.

use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum ChecksumAlgo {
    Md5,
    Sha1,
    Sha256,
    Crc32,
}

impl ChecksumAlgo {
    pub fn as_str(self) -> &'static str {
        match self {
            ChecksumAlgo::Md5 => "md5",
            ChecksumAlgo::Sha1 => "sha1",
            ChecksumAlgo::Sha256 => "sha256",
            ChecksumAlgo::Crc32 => "crc32",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "md5" => ChecksumAlgo::Md5,
            "sha1" => ChecksumAlgo::Sha1,
            "sha256" => ChecksumAlgo::Sha256,
            "crc32" => ChecksumAlgo::Crc32,
            _ => return None,
        })
    }

    /// Length of the value in hex digits.
    pub fn hex_len(self) -> usize {
        match self {
            ChecksumAlgo::Md5 => 32,
            ChecksumAlgo::Sha1 => 40,
            ChecksumAlgo::Sha256 => 64,
            ChecksumAlgo::Crc32 => 8,
        }
    }

    /// The algorithm a value of `len` hex digits belongs to.
    pub fn for_len(len: usize) -> Option<Self> {
        [Self::Md5, Self::Sha1, Self::Sha256, Self::Crc32]
            .into_iter()
            .find(|a| a.hex_len() == len)
    }

    /// The algorithm of a checksum list, by its file name (`.sfv` is CRC32).
    pub fn for_list(file_name: &str) -> Option<Self> {
        let ext = Path::new(file_name).extension()?.to_str()?;
        Some(match ext.to_ascii_lowercase().as_str() {
            "sfv" => ChecksumAlgo::Crc32,
            "md5" => ChecksumAlgo::Md5,
            "sha1" => ChecksumAlgo::Sha1,
            "sha256" => ChecksumAlgo::Sha256,
            _ => return None,
        })
    }
}

/// A download's expected checksum, and whether its file matched it
/// (`None`: not checked yet).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Checksum {
    pub algo: ChecksumAlgo,
    /// Lowercase hex.
    pub value: String,
    pub verified: Option<bool>,
}

fn is_hex(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The checksum in pasted text: the first hex word of a known length (of
/// `algo`'s length, when given), so `SHA256: ab12…`, `ab12… *file.iso` and
/// a bare value all work. Returns it lowercased with its algorithm.
pub fn parse_value(algo: Option<ChecksumAlgo>, text: &str) -> Result<(ChecksumAlgo, String)> {
    let found = text
        .split(|c: char| c.is_whitespace() || matches!(c, ':' | '=' | '*' | '(' | ')' | ','))
        .filter(|word| is_hex(word))
        .find_map(|word| {
            let found = ChecksumAlgo::for_len(word.len())?;
            algo.is_none_or(|a| a == found)
                .then(|| (found, word.to_ascii_lowercase()))
        });
    match found {
        Some(found) => Ok(found),
        None => Err(Error::InvalidInput(match algo {
            Some(a) => format!(
                "That isn't a {} checksum: expected {} hex digits.",
                label(a),
                a.hex_len()
            ),
            None => "That isn't a checksum: expected 8 (CRC32), 32 (MD5), 40 (SHA-1) or 64 (SHA-256) hex digits.".into(),
        })),
    }
}

/// The name people know the algorithm by.
pub fn label(algo: ChecksumAlgo) -> &'static str {
    match algo {
        ChecksumAlgo::Md5 => "MD5",
        ChecksumAlgo::Sha1 => "SHA-1",
        ChecksumAlgo::Sha256 => "SHA-256",
        ChecksumAlgo::Crc32 => "CRC32",
    }
}

enum Hasher {
    Md5(md5::Md5),
    Sha1(sha1::Sha1),
    Sha256(sha2::Sha256),
    Crc32(crc32fast::Hasher),
}

impl Hasher {
    fn new(algo: ChecksumAlgo) -> Self {
        match algo {
            ChecksumAlgo::Md5 => Hasher::Md5(md5::Digest::new()),
            ChecksumAlgo::Sha1 => Hasher::Sha1(sha1::Digest::new()),
            ChecksumAlgo::Sha256 => Hasher::Sha256(sha2::Digest::new()),
            ChecksumAlgo::Crc32 => Hasher::Crc32(crc32fast::Hasher::new()),
        }
    }

    fn update(&mut self, data: &[u8]) {
        match self {
            Hasher::Md5(h) => md5::Digest::update(h, data),
            Hasher::Sha1(h) => sha1::Digest::update(h, data),
            Hasher::Sha256(h) => sha2::Digest::update(h, data),
            Hasher::Crc32(h) => h.update(data),
        }
    }

    fn finish(self) -> String {
        fn hex(bytes: &[u8]) -> String {
            bytes.iter().map(|b| format!("{b:02x}")).collect()
        }
        match self {
            Hasher::Md5(h) => hex(&md5::Digest::finalize(h)),
            Hasher::Sha1(h) => hex(&sha1::Digest::finalize(h)),
            Hasher::Sha256(h) => hex(&sha2::Digest::finalize(h)),
            Hasher::Crc32(h) => format!("{:08x}", h.finalize()),
        }
    }
}

/// Lowercase hex checksum of `data`.
pub fn of_bytes(algo: ChecksumAlgo, data: &[u8]) -> String {
    let mut hasher = Hasher::new(algo);
    hasher.update(data);
    hasher.finish()
}

/// Lowercase hex checksum of a file, read on a blocking thread. `None` when
/// `cancel` fired first.
pub async fn of_file(
    algo: ChecksumAlgo,
    path: &Path,
    cancel: &CancellationToken,
) -> Result<Option<String>> {
    let path = path.to_owned();
    let cancel = cancel.clone();
    tokio::task::spawn_blocking(move || {
        let mut file = std::fs::File::open(&path)?;
        let mut hasher = Hasher::new(algo);
        let mut buf = vec![0u8; 1024 * 1024];
        loop {
            if cancel.is_cancelled() {
                return Ok(None);
            }
            match file.read(&mut buf)? {
                0 => return Ok(Some(hasher.finish())),
                n => hasher.update(&buf[..n]),
            }
        }
    })
    .await
    .map_err(|e| Error::Io(std::io::Error::other(e)))?
}

/// One entry of a checksum list: the file's name (without any folder) and
/// its lowercase value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListEntry {
    pub name: String,
    pub value: String,
}

/// Read a checksum list: `hash *name` / `hash  name` (md5sum and friends),
/// `SHA256 (name) = hash` (BSD), or `name hash` (SFV). Comment lines (`;`,
/// `#`) and lines without a value of `algo`'s length are skipped.
pub fn parse_list(algo: ChecksumAlgo, text: &str) -> Vec<ListEntry> {
    let ok = |v: &str| v.len() == algo.hex_len() && is_hex(v);
    let entry = |name: &str, value: &str| {
        let name = name.trim().trim_start_matches('*');
        let name = name.rsplit(['/', '\\']).next().unwrap_or(name).trim();
        (!name.is_empty() && ok(value)).then(|| ListEntry {
            name: name.to_owned(),
            value: value.to_ascii_lowercase(),
        })
    };
    text.lines()
        .map(|line| line.trim().trim_start_matches('\u{feff}'))
        .filter(|line| !line.is_empty() && !line.starts_with(';') && !line.starts_with('#'))
        .filter_map(|line| {
            // BSD: `ALGO (name) = value`.
            if let (Some(open), Some(eq)) = (line.find(" ("), line.rfind(") = ")) {
                if open < eq {
                    return entry(&line[open + 2..eq], line[eq + 4..].trim());
                }
            }
            let (first, rest) = line.split_once(char::is_whitespace)?;
            if ok(first) {
                return entry(rest, first);
            }
            let (name, last) = line.rsplit_once(char::is_whitespace)?;
            entry(name, last)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        let cases = [
            (ChecksumAlgo::Md5, "", "d41d8cd98f00b204e9800998ecf8427e"),
            (ChecksumAlgo::Md5, "abc", "900150983cd24fb0d6963f7d28e17f72"),
            (
                ChecksumAlgo::Sha1,
                "abc",
                "a9993e364706816aba3e25717850c26c9cd0d89d",
            ),
            (
                ChecksumAlgo::Sha256,
                "abc",
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            ),
            (ChecksumAlgo::Crc32, "123456789", "cbf43926"),
            (ChecksumAlgo::Crc32, "", "00000000"),
        ];
        for (algo, input, want) in cases {
            assert_eq!(
                of_bytes(algo, input.as_bytes()),
                want,
                "{algo:?} of {input:?}"
            );
        }
    }

    #[tokio::test]
    async fn files_hash_like_bytes_and_cancel_returns_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data.bin");
        let data: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &data).unwrap();
        let cancel = CancellationToken::new();
        for algo in [
            ChecksumAlgo::Md5,
            ChecksumAlgo::Sha1,
            ChecksumAlgo::Sha256,
            ChecksumAlgo::Crc32,
        ] {
            let got = of_file(algo, &path, &cancel).await.unwrap();
            assert_eq!(got.as_deref(), Some(of_bytes(algo, &data).as_str()));
        }
        cancel.cancel();
        assert_eq!(
            of_file(ChecksumAlgo::Sha256, &path, &cancel).await.unwrap(),
            None
        );
    }

    #[test]
    fn pasted_values_are_recognised_by_length() {
        let sha = "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD";
        assert_eq!(
            parse_value(None, &format!("SHA256: {sha}")).unwrap(),
            (ChecksumAlgo::Sha256, sha.to_ascii_lowercase())
        );
        assert_eq!(
            parse_value(None, "900150983cd24fb0d6963f7d28e17f72 *file.iso").unwrap(),
            (ChecksumAlgo::Md5, "900150983cd24fb0d6963f7d28e17f72".into())
        );
        assert_eq!(
            parse_value(None, " CBF43926 ").unwrap(),
            (ChecksumAlgo::Crc32, "cbf43926".into())
        );
        // An algorithm given: only its length counts.
        assert!(parse_value(Some(ChecksumAlgo::Sha1), "cbf43926").is_err());
        assert_eq!(
            parse_value(
                Some(ChecksumAlgo::Crc32),
                "deadbeef 900150983cd24fb0d6963f7d28e17f72"
            )
            .unwrap()
            .1,
            "deadbeef"
        );
        assert!(parse_value(None, "not a checksum").is_err());
        assert!(parse_value(None, "xyz123").is_err());
    }

    #[test]
    fn list_algorithm_comes_from_the_extension() {
        assert_eq!(
            ChecksumAlgo::for_list("disc.SFV"),
            Some(ChecksumAlgo::Crc32)
        );
        assert_eq!(ChecksumAlgo::for_list("a.md5"), Some(ChecksumAlgo::Md5));
        assert_eq!(ChecksumAlgo::for_list("a.sha1"), Some(ChecksumAlgo::Sha1));
        assert_eq!(
            ChecksumAlgo::for_list("SHA256SUMS.sha256"),
            Some(ChecksumAlgo::Sha256)
        );
        assert_eq!(ChecksumAlgo::for_list("a.txt"), None);
        assert_eq!(ChecksumAlgo::for_list("SHA256SUMS"), None);
    }

    #[test]
    fn sfv_lists() {
        let text = "\u{feff}; Generated by QuickSFV\r\n; \r\n\
                    part1.rar 1A2B3C4D\r\n\
                    My Files\\part 2.rar   cbf43926\r\n\
                    broken.rar XYZ\r\n";
        assert_eq!(
            parse_list(ChecksumAlgo::Crc32, text),
            vec![
                ListEntry {
                    name: "part1.rar".into(),
                    value: "1a2b3c4d".into()
                },
                ListEntry {
                    name: "part 2.rar".into(),
                    value: "cbf43926".into()
                },
            ]
        );
    }

    #[test]
    fn md5sum_and_bsd_lists() {
        let md5 = "900150983cd24fb0d6963f7d28e17f72";
        let text = format!(
            "# comment\n{md5} *disc one.iso\n{md5}  ./sub/two.iso\nMD5 (three.iso) = {}\nshort  four.iso\n",
            md5.to_ascii_uppercase()
        );
        let names: Vec<_> = parse_list(ChecksumAlgo::Md5, &text)
            .into_iter()
            .map(|e| (e.name, e.value))
            .collect();
        assert_eq!(
            names,
            vec![
                ("disc one.iso".to_owned(), md5.to_owned()),
                ("two.iso".to_owned(), md5.to_owned()),
                ("three.iso".to_owned(), md5.to_owned()),
            ]
        );
    }
}
