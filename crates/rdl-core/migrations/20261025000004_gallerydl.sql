-- gallery-dl (1f). A gallery is one download made of many files: count them,
-- and remember which files it wrote so "delete files" removes exactly those
-- (the gallery folder may hold files that were there before).
ALTER TABLE downloads ADD COLUMN files_done INTEGER CHECK (files_done IS NULL OR files_done >= 0);
ALTER TABLE downloads ADD COLUMN files_total INTEGER CHECK (files_total IS NULL OR files_total >= 0);

CREATE TABLE download_files (
    download_id INTEGER NOT NULL REFERENCES downloads (id) ON DELETE CASCADE,
    path        TEXT    NOT NULL,
    size        INTEGER,
    PRIMARY KEY (download_id, path)
) STRICT, WITHOUT ROWID;
