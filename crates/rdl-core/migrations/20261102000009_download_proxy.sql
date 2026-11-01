-- Proxies (4a). A download's own connection choice: NULL lets the proxy
-- rules decide, 'direct' connects directly, a number is a proxy_profiles id
-- (one that's gone counts as NULL).
ALTER TABLE downloads ADD COLUMN proxy TEXT;
