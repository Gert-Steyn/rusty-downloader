-- Seeding (5c). What a torrent has uploaded across all its runs, whether it
-- should be seeding now, and how long it has seeded in total.
ALTER TABLE downloads ADD COLUMN uploaded INTEGER NOT NULL DEFAULT 0;
ALTER TABLE downloads ADD COLUMN seeding INTEGER NOT NULL DEFAULT 0;
ALTER TABLE downloads ADD COLUMN seeded_secs INTEGER NOT NULL DEFAULT 0;
