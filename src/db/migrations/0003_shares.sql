-- Share links. `path` is relative to CAIRN_ROOT; nothing else about files is stored.
CREATE TABLE shares (
  id             TEXT PRIMARY KEY,
  user_id        INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  path           TEXT NOT NULL,
  password_hash  TEXT,
  allow_download INTEGER NOT NULL DEFAULT 1,
  expires_at     INTEGER,
  created_at     INTEGER NOT NULL,
  hits           INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX shares_user ON shares(user_id);
