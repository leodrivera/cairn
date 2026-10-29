CREATE TABLE users (
  id            INTEGER PRIMARY KEY,
  email         TEXT NOT NULL UNIQUE COLLATE NOCASE,
  name          TEXT NOT NULL,
  password_hash TEXT,
  scope         TEXT NOT NULL DEFAULT '/',
  perm_read     INTEGER NOT NULL DEFAULT 1,
  perm_write    INTEGER NOT NULL DEFAULT 0,
  perm_share    INTEGER NOT NULL DEFAULT 0,
  perm_admin    INTEGER NOT NULL DEFAULT 0,
  lang          TEXT,
  theme         TEXT,
  view          TEXT,
  created_at    INTEGER NOT NULL
);

CREATE TABLE sessions (
  id         BLOB PRIMARY KEY,
  user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  method     TEXT NOT NULL,
  keep       INTEGER NOT NULL DEFAULT 0,
  label      TEXT,
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL
);

CREATE INDEX sessions_user ON sessions(user_id);

CREATE TABLE settings (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
