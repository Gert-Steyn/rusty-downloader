-- Checksums (3d). `checksum_algo` / `checksum_value` exist since the first
-- migration; this records whether the finished file matched: NULL not
-- checked yet, 1 matched, 0 didn't.
ALTER TABLE downloads ADD COLUMN checksum_ok INTEGER CHECK (checksum_ok IS NULL OR checksum_ok IN (0, 1));
