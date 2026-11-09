-- Enriching mentors already in the list (docs/design.md#导师) is a crawl of its own kind, which
-- the crawl table's checks did not allow, so the table is rebuilt
-- (https://sqlite.org/lang_altertable.html#otheralter), with foreign keys off as db::open
-- upgrades, and with the legacy rename, so that what names the table finds the new one.

CREATE TABLE crawl_new (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  kind TEXT NOT NULL CHECK (kind IN ('site', 'agent', 'mentors')),
  university TEXT,
  school TEXT,
  start_urls TEXT,             -- JSON array
  limits TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(limits)),
  model_config_id INTEGER REFERENCES model_config(id) ON DELETE SET NULL,
  state TEXT NOT NULL CHECK (state IN ('running', 'paused', 'done', 'failed', 'canceled')),
  error TEXT,
  created_at INTEGER NOT NULL,
  finished_at INTEGER,
  reported_at INTEGER,
  CHECK ((kind = 'site') = (start_urls IS NOT NULL)),
  CHECK (start_urls IS NULL OR (json_valid(start_urls) AND json_type(start_urls) = 'array')),
  CHECK ((state IN ('done', 'failed', 'canceled')) = (finished_at IS NOT NULL)),
  CHECK (state != 'failed' OR error IS NOT NULL)
) STRICT;
INSERT INTO crawl_new (id, kind, university, school, start_urls, limits, model_config_id, state,
    error, created_at, finished_at, reported_at)
  SELECT id, kind, university, school, start_urls, limits, model_config_id, state, error,
    created_at, finished_at, reported_at
  FROM crawl;
-- The new table counts on from where the old one got, or a deleted crawl's ID would be given again
-- (docs/development.md#数据库).
DELETE FROM sqlite_sequence WHERE name = 'crawl_new';
INSERT INTO sqlite_sequence (name, seq) SELECT 'crawl_new', seq FROM sqlite_sequence WHERE name = 'crawl';
DROP TABLE crawl;
PRAGMA legacy_alter_table = ON;
ALTER TABLE crawl_new RENAME TO crawl;
PRAGMA legacy_alter_table = OFF;

-- A mentor's candidate a re-run crawl found stays to be reviewed; one made only to enrich the
-- mentor goes with its record.
CREATE TRIGGER crawl_delete_candidates BEFORE DELETE ON crawl BEGIN
  DELETE FROM candidate
  WHERE crawl_id = old.id AND review = 'pending' AND (mentor_id IS NULL OR old.kind = 'mentors');
END;

-- Else it would lose its mentor and, accepted, bring them back as new.
CREATE TRIGGER mentor_delete_enrichment BEFORE DELETE ON mentor BEGIN
  DELETE FROM candidate
  WHERE mentor_id = old.id AND review = 'pending'
    AND crawl_id IN (SELECT id FROM crawl WHERE kind = 'mentors');
END;
