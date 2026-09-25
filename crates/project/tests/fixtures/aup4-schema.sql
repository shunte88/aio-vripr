-- Audacity AUP4 project schema, extracted verbatim.
--
-- Provenance: `sqlite3 "file://<project>?mode=ro" .schema`, run against
-- `simples_test.aup4` in the /data2/vinyl_rips corpus. application_id
-- 1096107097 (0x41554459, ASCII "AUDY"),
-- user_version 67108865 (0x04000001).
--
-- Both AUP3 and AUP4 report the same application_id, which is why version
-- dispatch is on user_version and never on the magic or the file extension.
--
-- Clean room (risk R13): this is the output of reading a file, not of reading
-- Audacity's source. Nothing here inherits Audacity's licence.
--
-- `sqlite_sequence` is omitted below: SQLite creates it itself for an
-- AUTOINCREMENT table and refuses an explicit CREATE.
--
-- This fixture is the reference `vcw-project` diffs its own `sampleblocks`
-- against, so D1's "column-for-column identical" claim is checked by CI rather
-- than asserted in a document.

CREATE TABLE project(  id                   INTEGER PRIMARY KEY,  dict                 BLOB,  doc                  BLOB);
CREATE TABLE autosave(  id                   INTEGER PRIMARY KEY,  dict                 BLOB,  doc                  BLOB);
CREATE TABLE sampleblocks(  blockid              INTEGER PRIMARY KEY AUTOINCREMENT,  sampleformat         INTEGER,  summin               REAL,  summax               REAL,  sumrms               REAL,  summary256           BLOB,  summary64k           BLOB,  samples              BLOB);
CREATE TABLE project_history(  generation           INTEGER PRIMARY KEY AUTOINCREMENT,  saved_at             INTEGER,  dict                 BLOB,  doc                  BLOB);
