-- Browser hand-off (Phase 2). What the browser would send with a link:
-- referer, user agent and cookies, as JSON (`request::RequestContext`).
-- NULL for links that came from anywhere else. A download's copy is cleared
-- once it completes, so cookies don't outlive their use.
ALTER TABLE downloads ADD COLUMN request_json TEXT;
ALTER TABLE link_grabber ADD COLUMN request_json TEXT;
