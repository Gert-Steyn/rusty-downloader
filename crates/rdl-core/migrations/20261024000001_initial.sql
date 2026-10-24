-- Initial schema (PLAN.md §8). Timestamps are Unix epoch milliseconds.
-- Secrets never live here: *_keyring_ref columns point into Windows Credential Manager.

CREATE TABLE settings (
    key        TEXT PRIMARY KEY,
    value_json TEXT NOT NULL
) STRICT;

CREATE TABLE queues (
    id                 INTEGER PRIMARY KEY,
    name               TEXT    NOT NULL UNIQUE,
    max_active         INTEGER NOT NULL DEFAULT 3 CHECK (max_active > 0),
    speed_limit_bps    INTEGER CHECK (speed_limit_bps IS NULL OR speed_limit_bps > 0),
    schedule_json      TEXT,
    on_complete_action TEXT    NOT NULL DEFAULT 'none'
        CHECK (on_complete_action IN ('none', 'sleep', 'hibernate', 'shutdown', 'exit')),
    position           INTEGER NOT NULL DEFAULT 0,
    created_at         INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER))
) STRICT;

INSERT INTO queues (id, name) VALUES (1, 'Main');

CREATE TABLE packages (
    id                     INTEGER PRIMARY KEY,
    name                   TEXT    NOT NULL,
    dest_dir               TEXT    NOT NULL,
    queue_id               INTEGER NOT NULL REFERENCES queues (id) ON DELETE RESTRICT,
    priority               INTEGER NOT NULL DEFAULT 0,
    position               INTEGER NOT NULL DEFAULT 0,
    extract_passwords_json TEXT,
    settings_json          TEXT,
    created_at             INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER))
) STRICT;

CREATE INDEX packages_queue ON packages (queue_id, position);

CREATE TABLE downloads (
    id                INTEGER PRIMARY KEY,
    package_id        INTEGER NOT NULL REFERENCES packages (id) ON DELETE CASCADE,
    kind              TEXT    NOT NULL
        CHECK (kind IN ('http', 'ftp', 'sftp', 'torrent', 'ytdlp', 'gallerydl')),
    source_url        TEXT    NOT NULL,
    resolved_url      TEXT,
    filename          TEXT,
    dest_path         TEXT,
    size              INTEGER CHECK (size IS NULL OR size >= 0),
    downloaded        INTEGER NOT NULL DEFAULT 0 CHECK (downloaded >= 0),
    status            TEXT    NOT NULL DEFAULT 'queued'
        CHECK (status IN ('queued', 'active', 'paused', 'completed', 'failed', 'canceled')),
    error             TEXT,
    handler_opts_json TEXT,
    checksum_algo     TEXT CHECK (checksum_algo IS NULL OR checksum_algo IN ('md5', 'sha1', 'sha256', 'crc32')),
    checksum_value    TEXT,
    etag              TEXT,
    last_modified     TEXT,
    position          INTEGER NOT NULL DEFAULT 0,
    created_at        INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
    started_at        INTEGER,
    finished_at       INTEGER
) STRICT;

CREATE INDEX downloads_package ON downloads (package_id, position);
CREATE INDEX downloads_status ON downloads (status);

-- Byte ranges are inclusive, matching HTTP Range semantics.
CREATE TABLE segments (
    download_id INTEGER NOT NULL REFERENCES downloads (id) ON DELETE CASCADE,
    idx         INTEGER NOT NULL,
    start_byte  INTEGER NOT NULL CHECK (start_byte >= 0),
    end_byte    INTEGER NOT NULL CHECK (end_byte >= start_byte),
    written     INTEGER NOT NULL DEFAULT 0 CHECK (written >= 0),
    PRIMARY KEY (download_id, idx)
) STRICT, WITHOUT ROWID;

CREATE TABLE link_grabber (
    id                INTEGER PRIMARY KEY,
    source_url        TEXT    NOT NULL,
    kind              TEXT
        CHECK (kind IS NULL OR kind IN ('http', 'ftp', 'sftp', 'torrent', 'ytdlp', 'gallerydl')),
    filename          TEXT,
    size              INTEGER,
    availability      TEXT    NOT NULL DEFAULT 'unknown'
        CHECK (availability IN ('unknown', 'checking', 'online', 'offline', 'error')),
    proposed_package  TEXT,
    dest_dir          TEXT,
    handler_opts_json TEXT,
    info_json         TEXT,
    added_at          INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER))
) STRICT;

CREATE TABLE history (
    id          INTEGER PRIMARY KEY,
    url_hash    TEXT    NOT NULL,
    url         TEXT    NOT NULL,
    final_path  TEXT    NOT NULL,
    size        INTEGER,
    sha256      TEXT,
    finished_at INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER))
) STRICT;

CREATE INDEX history_url_hash ON history (url_hash);
CREATE INDEX history_path ON history (final_path);

CREATE TABLE accounts (
    id           INTEGER PRIMARY KEY,
    site         TEXT    NOT NULL,
    username     TEXT    NOT NULL,
    keyring_ref  TEXT,
    cookies_json TEXT,
    enabled      INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    created_at   INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
    UNIQUE (site, username)
) STRICT;

CREATE TABLE proxy_profiles (
    id          INTEGER PRIMARY KEY,
    name        TEXT    NOT NULL UNIQUE,
    kind        TEXT    NOT NULL CHECK (kind IN ('http', 'https', 'socks5')),
    host        TEXT    NOT NULL,
    port        INTEGER NOT NULL CHECK (port BETWEEN 1 AND 65535),
    username    TEXT,
    keyring_ref TEXT
) STRICT;

-- profile_id NULL means "connect directly" for hosts matching the pattern.
CREATE TABLE proxy_rules (
    id           INTEGER PRIMARY KEY,
    host_pattern TEXT    NOT NULL,
    profile_id   INTEGER REFERENCES proxy_profiles (id) ON DELETE CASCADE,
    position     INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE TABLE speed_profiles (
    id            INTEGER PRIMARY KEY,
    name          TEXT    NOT NULL UNIQUE,
    schedule_json TEXT    NOT NULL,
    limit_bps     INTEGER NOT NULL CHECK (limit_bps > 0),
    enabled       INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1))
) STRICT;

CREATE TABLE tools (
    name       TEXT PRIMARY KEY,
    version    TEXT,
    channel    TEXT NOT NULL DEFAULT 'stable' CHECK (channel IN ('stable', 'nightly')),
    path       TEXT,
    last_check INTEGER
) STRICT;
