-- Trash entries. `trash_dir` is the trash folder holding the entry and `path` where it was, both relative to
-- CAIRN_ROOT; `name` is the original name's bytes. The removal date is fixed when the item is deleted.
CREATE TABLE trash (
  id         TEXT PRIMARY KEY,
  trash_dir  TEXT NOT NULL,
  path       TEXT NOT NULL,
  name       BLOB NOT NULL,
  kind       TEXT NOT NULL,
  size       INTEGER NOT NULL,
  deleted_by INTEGER REFERENCES users(id) ON DELETE SET NULL,
  deleted_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL
);

CREATE INDEX trash_expires ON trash(expires_at);
