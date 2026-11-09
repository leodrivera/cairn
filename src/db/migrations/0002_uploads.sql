-- Temporary upload files while they exist, so startup can remove what a crash left behind.
CREATE TABLE uploads (
  -- The path's exact bytes: a folder name need not be UTF-8.
  path       BLOB PRIMARY KEY,
  started_at INTEGER NOT NULL
);
