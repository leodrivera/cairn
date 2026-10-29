-- Temporary upload files while they exist, so startup can remove what a crash left behind.
CREATE TABLE uploads (
  path       TEXT PRIMARY KEY,
  started_at INTEGER NOT NULL
);
