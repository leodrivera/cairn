-- Grants: one owner lets one other user open a file or folder. `path` is relative to CAIRN_ROOT, like a share's;
-- whether a grant still works is checked on every use.
CREATE TABLE grants (
  id           TEXT PRIMARY KEY,
  owner_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  recipient_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  path         TEXT NOT NULL,
  write        INTEGER NOT NULL DEFAULT 0,
  expires_at   INTEGER,
  created_at   INTEGER NOT NULL,
  UNIQUE (owner_id, recipient_id, path)
);

CREATE INDEX grants_owner ON grants(owner_id);
CREATE INDEX grants_recipient ON grants(recipient_id);
