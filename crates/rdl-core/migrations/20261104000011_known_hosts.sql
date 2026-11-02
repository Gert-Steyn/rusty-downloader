-- SFTP (4e). The host key each SFTP server showed the first time, trusted
-- from then on (trust on first use); a different key later is refused until
-- the user forgets the old one. `fingerprint` is OpenSSH's SHA256:… form.
CREATE TABLE known_hosts (
    host        TEXT    NOT NULL,
    port        INTEGER NOT NULL,
    algorithm   TEXT    NOT NULL,
    fingerprint TEXT    NOT NULL,
    added_at    INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
    PRIMARY KEY (host, port)
);
