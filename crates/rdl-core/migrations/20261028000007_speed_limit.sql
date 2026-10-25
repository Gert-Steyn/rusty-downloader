-- Speed limits (3b). A download's own cap in bytes per second, on top of
-- the global one; NULL is no cap of its own.
ALTER TABLE downloads ADD COLUMN speed_limit INTEGER CHECK (speed_limit IS NULL OR speed_limit > 0);
