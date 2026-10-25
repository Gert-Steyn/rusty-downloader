-- History (1g). Every finished download is recorded, keyed by a hash of its
-- source URL (never by download id: SQLite reuses those), so the LinkGrabber
-- can warn before fetching something twice and the History page can list it.
ALTER TABLE history ADD COLUMN name TEXT;
ALTER TABLE history ADD COLUMN kind TEXT;
ALTER TABLE history ADD COLUMN package TEXT;

CREATE INDEX history_finished ON history (finished_at);
