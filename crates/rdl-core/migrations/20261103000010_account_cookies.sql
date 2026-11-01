-- Accounts (4c). An account's cookies, as JSON sealed with DPAPI for the
-- current Windows user; `cookies_json` from the first migration stays unused.
-- An account without a login has the empty user name (the column is NOT NULL).
ALTER TABLE accounts ADD COLUMN cookies_sealed BLOB;
