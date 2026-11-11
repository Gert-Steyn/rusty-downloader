-- Torrents (5a). The info hash of a torrent download, once its metadata is
-- known; the engine finds the torrent by it after a restart.
ALTER TABLE downloads ADD COLUMN info_hash TEXT;
