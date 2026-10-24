-- Tool manager (1e). Each install lives in its own versioned folder; `path`
-- points at the active one and `previous_*` at the one kept for rollback, so
-- switching versions is a single row update and never touches files in use.
-- Releases are compared by the SHA-256 of the upstream asset, because some
-- upstreams (FFmpeg-Builds) republish under the same tag.
ALTER TABLE tools ADD COLUMN release TEXT;
ALTER TABLE tools ADD COLUMN sha256 TEXT;
ALTER TABLE tools ADD COLUMN installed_at INTEGER;
ALTER TABLE tools ADD COLUMN latest_release TEXT;
ALTER TABLE tools ADD COLUMN latest_sha256 TEXT;
ALTER TABLE tools ADD COLUMN previous_path TEXT;
ALTER TABLE tools ADD COLUMN previous_version TEXT;
ALTER TABLE tools ADD COLUMN previous_release TEXT;
ALTER TABLE tools ADD COLUMN previous_sha256 TEXT;

-- Media title for yt-dlp downloads, shown until the real file name is known.
ALTER TABLE downloads ADD COLUMN title TEXT;
